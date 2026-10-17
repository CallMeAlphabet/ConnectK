package connectk.core.solver;

import connectk.core.eval.Evaluator;
import connectk.core.model.Bits;
import connectk.core.model.Move;
import connectk.core.model.Outcome;
import connectk.core.model.Placement;
import connectk.core.model.Player;
import connectk.core.model.Position;
import connectk.core.model.PositionKey;
import connectk.core.model.Rules;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.OptionalInt;

/**
 * Depth-limited solver for positions that cannot (yet) be proved:
 * iterative-deepening negamax with alpha-beta pruning, a depth-aware
 * transposition table, history/move-ordering heuristics and the
 * {@link Evaluator} at the horizon.
 *
 * <p>Results are always labelled
 * {@link SearchResult.Certainty#HEURISTIC_DEPTH_LIMITED}. For free-placement
 * games with many empty cells the candidate moves are additionally
 * restricted to the neighbourhood of existing stones (standard Gomoku
 * practice), which is another reason the result is a playing heuristic,
 * not a proof.</p>
 *
 * <p>Instances are single-use and not thread-safe.</p>
 */
public final class DepthLimitedSolver {

    private enum Bound { EXACT, LOWER, UPPER }

    private record Entry(int depth, int score, Bound bound, Move bestMove) {
    }

    private record RootResult(int score, Move bestMove) {
    }

    private static final int MAX_TABLE_ENTRIES = 1_000_000;
    private static final int INF = Evaluator.WIN_SCORE * 2;

    private final Map<PositionKey, Entry> table = new HashMap<>(1 << 15);
    private Map<PositionKey, Move> iterationBestMoves = new HashMap<>();

    private Rules rules;
    private Position pos;
    private Evaluator evaluator;
    private int[] history;
    private long nodes;
    private long nodeLimit;
    private long startNanos;
    private long deadlineNanos;
    private boolean candidateRestrictionUsed;
    private CancellationToken token = CancellationToken.NONE;
    private SearchListener listener = SearchListener.NONE;

    public SearchResult solve(Position position, int maxDepth, long nodeLimit, long timeLimitMillis,
                              CancellationToken token, SearchListener listener) {
        this.rules = position.rules();
        this.pos = position.copy();
        this.evaluator = new Evaluator(rules);
        this.history = new int[rules.bitCount()];
        this.nodes = 0;
        this.nodeLimit = nodeLimit;
        this.token = token == null ? CancellationToken.NONE : token;
        this.listener = listener == null ? SearchListener.NONE : listener;
        this.startNanos = System.nanoTime();
        this.deadlineNanos = timeLimitMillis > 0
                ? startNanos + timeLimitMillis * 1_000_000L
                : Long.MAX_VALUE;
        this.candidateRestrictionUsed = false;

        if (position.isTerminal()) {
            int exactScore = exactTerminalScore(position);
            return new SearchResult(SearchResult.Certainty.PROVEN_EXACT,
                    Optional.of(Outcome.fromScoreSign(exactScore)), Optional.empty(), true,
                    exactScore, OptionalInt.of(0), 0, 0, 0, List.of(), "Position is already terminal.");
        }

        Move completedBest = null;
        int completedScore = 0;
        int completedDepth = 0;
        List<Move> completedPv = List.of();

        for (int depth = 1; depth <= maxDepth; depth++) {
            iterationBestMoves = new HashMap<>();
            RootResult root;
            try {
                root = searchRoot(depth);
            } catch (SearchAbortedException e) {
                break; // keep the last fully completed iteration
            }
            completedBest = root.bestMove();
            completedScore = root.score();
            completedDepth = depth;
            completedPv = reconstructVariation(position, iterationBestMoves);
            listener.onProgress(nodes, depth, Optional.ofNullable(completedBest));
            if (Evaluator.isWinningScore(completedScore)) break; // forced line already found
            if (System.nanoTime() > deadlineNanos) break;
        }

        long elapsed = elapsedMillis();
        if (completedBest == null) {
            // Budget died during depth 1: fall back to a statically ordered move.
            List<Move> legal = position.legalMoves();
            Move fallback = legal.isEmpty() ? null : legal.get(0);
            int eval = evaluator.evaluate(position);
            return new SearchResult(SearchResult.Certainty.UNKNOWN_LIMIT_REACHED,
                    Optional.of(Outcome.fromScoreSign(eval)), Optional.ofNullable(fallback), false,
                    eval, OptionalInt.empty(), 0, nodes, elapsed, List.of(),
                    "Budget exhausted before depth 1 completed; move is a static fallback, evaluation only.");
        }

        OptionalInt plies = OptionalInt.empty();
        if (Evaluator.isWinningScore(completedScore) || Evaluator.isLosingScore(completedScore)) {
            int terminalMoves = (Evaluator.WIN_SCORE - Math.abs(completedScore)) / 10;
            plies = OptionalInt.of(Math.max(0, terminalMoves - position.movesPlayed()));
        }
        String explanation = "Iterative-deepening alpha-beta to depth " + completedDepth
                + " plies with heuristic evaluation (" + evaluator.windowCount() + " potential lines)."
                + (candidateRestrictionUsed
                ? " Candidate moves were restricted to the neighbourhood of existing stones."
                : "")
                + " This is a playing strength estimate, not a mathematical proof.";
        return new SearchResult(SearchResult.Certainty.HEURISTIC_DEPTH_LIMITED,
                Optional.of(Outcome.fromScoreSign(completedScore)),
                Optional.of(completedBest), false, completedScore, plies,
                completedDepth, nodes, elapsed, completedPv, explanation);
    }

    private int exactTerminalScore(Position p) {
        return p.winner()
                .map(w -> {
                    int magnitude = rules.cells() - p.movesPlayed() + 1;
                    return w == p.sideToMove() ? magnitude : -magnitude;
                })
                .orElse(0);
    }

    private long elapsedMillis() {
        return (System.nanoTime() - startNanos) / 1_000_000L;
    }

    private void checkBudget() {
        if (nodeLimit > 0 && nodes > nodeLimit) {
            throw new SearchAbortedException(SearchAbortedException.Reason.NODE_LIMIT);
        }
        if ((nodes & 1023L) == 0L) {
            if (token.isCancelled()) throw new SearchAbortedException(SearchAbortedException.Reason.CANCELLED);
            if (System.nanoTime() > deadlineNanos) {
                throw new SearchAbortedException(SearchAbortedException.Reason.TIME_LIMIT);
            }
        }
    }

    private RootResult searchRoot(int depth) {
        nodes++;
        checkBudget();
        Entry tt = table.get(pos.rawKey());
        List<Move> moves = orderedCandidates(tt == null ? null : tt.bestMove(), true);
        int alpha = -INF;
        int best = -INF;
        Move bestMove = null;
        for (Move move : moves) {
            pos.play(move);
            int score;
            try {
                score = -negamax(depth - 1, -INF, -alpha, 1);
            } finally {
                pos.undo();
            }
            if (score > best) {
                best = score;
                bestMove = move;
            }
            if (best > alpha) alpha = best;
        }
        if (bestMove != null) iterationBestMoves.put(pos.rawKey(), bestMove);
        return new RootResult(best, bestMove);
    }

    private int negamax(int depth, int alpha, int beta, int ply) {
        nodes++;
        checkBudget();
        if (pos.isTerminal()) return Evaluator.terminalScore(pos);
        if (depth == 0) return evaluator.evaluate(pos);

        int alphaOrig = alpha;
        PositionKey key = pos.rawKey();
        Entry hit = table.get(key);
        Move ttMove = null;
        if (hit != null) {
            ttMove = hit.bestMove();
            if (hit.depth() >= depth) {
                switch (hit.bound()) {
                    case EXACT -> {
                        return hit.score();
                    }
                    case LOWER -> alpha = Math.max(alpha, hit.score());
                    case UPPER -> beta = Math.min(beta, hit.score());
                }
                if (alpha >= beta) return hit.score();
            }
        }

        List<Move> moves = orderedCandidates(ttMove, false);
        int best = -INF;
        Move bestMove = null;
        for (Move move : moves) {
            pos.play(move);
            int score;
            try {
                score = -negamax(depth - 1, -beta, -alpha, ply + 1);
            } finally {
                pos.undo();
            }
            if (score > best) {
                best = score;
                bestMove = move;
            }
            if (best > alpha) alpha = best;
            if (alpha >= beta) {
                int idx = rules.index(move.column(), move.row());
                history[idx] += depth * depth;
                break;
            }
        }

        Bound bound = best <= alphaOrig ? Bound.UPPER : (best >= beta ? Bound.LOWER : Bound.EXACT);
        if (table.size() < MAX_TABLE_ENTRIES) {
            table.put(key, new Entry(depth, best, bound, bestMove));
        }
        if (bestMove != null) iterationBestMoves.put(key, bestMove);
        return best;
    }

    private List<Move> orderedCandidates(Move ttMove, boolean root) {
        List<Move> legal = pos.legalMoves();
        if (legal.size() <= 1) return legal;

        List<Move> candidates = legal;
        if (rules.placement() == Placement.FREE && legal.size() > 40) {
            candidates = neighbourhoodCandidates(legal);
            candidateRestrictionUsed = true;
        }

        Player side = pos.sideToMove();
        long[] ownSpots = pos.winningSpots(side);
        long[] oppSpots = pos.winningSpots(side.other());
        record Scored(Move move, long score) {
        }
        List<Scored> scored = new ArrayList<>(candidates.size());
        int w = rules.width();
        int h = rules.height();
        for (Move move : candidates) {
            long s = 0;
            int idx = rules.index(move.column(), move.row());
            if (move.equals(ttMove)) s += 10_000_000L;
            boolean completes = Bits.test(ownSpots, idx);
            if (completes) s += rules.misere() ? -9_000_000L : 9_000_000L;
            if (!rules.misere() && Bits.test(oppSpots, idx)) s += 8_000_000L;
            s += history[idx];
            s += (Math.min(move.column(), w - 1 - move.column()) + 1) * 10L;
            s += (Math.min(move.row(), h - 1 - move.row()) + 1) * 4L;
            pos.play(move);
            try {
                if (!pos.isTerminal()) {
                    s += pos.countWinningSpots(side) * 1_000L;
                    if (!rules.misere() && !pos.lineCompletingMoves(pos.sideToMove()).isEmpty()) {
                        s -= 500_000L;
                    }
                }
            } finally {
                pos.undo();
            }
            scored.add(new Scored(move, s));
        }
        scored.sort((a, b) -> Long.compare(b.score(), a.score()));

        int cap = Integer.MAX_VALUE;
        if (rules.placement() == Placement.FREE) cap = root ? 36 : 28;
        else if (w > 18) cap = 16;
        if (scored.size() > cap) candidateRestrictionUsed = true;

        List<Move> out = new ArrayList<>(Math.min(scored.size(), cap));
        for (Scored sc : scored) {
            if (out.size() >= cap) break;
            out.add(sc.move());
        }
        return out;
    }

    private List<Move> neighbourhoodCandidates(List<Move> legal) {
        int w = rules.width();
        int h = rules.height();
        if (pos.movesPlayed() == 0) {
            List<Move> centre = new ArrayList<>();
            for (Move m : legal) {
                boolean centreCol = m.column() == (w - 1) / 2 || m.column() == w / 2;
                boolean centreRow = m.row() == (h - 1) / 2 || m.row() == h / 2;
                if (centreCol && centreRow) centre.add(m);
            }
            return centre.isEmpty() ? legal : centre;
        }
        boolean[][] near = new boolean[w][h];
        for (int c = 0; c < w; c++) {
            for (int r = 0; r < h; r++) {
                if (pos.isEmpty(c, r)) continue;
                for (int dc = -2; dc <= 2; dc++) {
                    for (int dr = -2; dr <= 2; dr++) {
                        int cc = c + dc;
                        int rr = r + dr;
                        if (cc >= 0 && cc < w && rr >= 0 && rr < h) near[cc][rr] = true;
                    }
                }
            }
        }
        List<Move> out = new ArrayList<>();
        for (Move m : legal) {
            if (near[m.column()][m.row()]) out.add(m);
        }
        return out.isEmpty() ? legal : out;
    }

    private List<Move> reconstructVariation(Position original, Map<PositionKey, Move> bestMoves) {
        List<Move> pv = new ArrayList<>();
        Position cursor = original.copy();
        while (!cursor.isTerminal() && pv.size() <= rules.cells()) {
            Move m = bestMoves.get(cursor.rawKey());
            if (m == null) break;
            try {
                pv.add(cursor.play(m));
            } catch (RuntimeException e) {
                break;
            }
        }
        return pv;
    }
}
