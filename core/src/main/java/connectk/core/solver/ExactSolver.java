package connectk.core.solver;

import connectk.core.model.Bits;
import connectk.core.model.Move;
import connectk.core.model.Outcome;
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
 * Exact (proof) solver: negamax to the end of the game with alpha-beta
 * pruning, a transposition table with mirror-canonical keys, threat-based
 * move ordering and one-ply tactical shortcuts (immediate wins, forced
 * blocks, double threats). It either returns a mathematically proven
 * result or, when its node/time budget runs out, UNKNOWN — never a guess.
 *
 * <p>Score convention (side to move): 0 = draw; a forced win ending on ply
 * {@code t} of a {@code cells}-cell board scores {@code +(cells - t + 1)}
 * (faster wins are bigger); forced losses are the negation (slower losses
 * are bigger, i.e. closer to 0).</p>
 *
 * <p>Instances are single-use and not thread-safe; create one per solve.</p>
 */
public final class ExactSolver {

    private enum Bound { EXACT, LOWER, UPPER }

    private record Entry(int score, Bound bound) {
    }

    private record Prepared(List<Move> moves, Integer forcedScore, Move forcedMove) {
    }

    private record RootResult(int score, Move bestMove) {
    }

    private static final int MAX_TABLE_ENTRIES = 1_000_000;

    private final Map<PositionKey, Entry> table = new HashMap<>(1 << 15);
    private final Map<PositionKey, Move> principalMoves = new HashMap<>();

    private Rules rules;
    private Position pos;
    private int cells;
    private int infinity;
    private long nodes;
    private long nodeLimit;
    private long startNanos;
    private long deadlineNanos;
    private CancellationToken token = CancellationToken.NONE;
    private SearchListener listener = SearchListener.NONE;

    public SearchResult solve(Position position, long nodeLimit, long timeLimitMillis,
                              CancellationToken token, SearchListener listener) {
        this.rules = position.rules();
        this.pos = position.copy();
        this.cells = rules.cells();
        this.infinity = cells + 2;
        this.nodes = 0;
        this.nodeLimit = nodeLimit;
        this.token = token == null ? CancellationToken.NONE : token;
        this.listener = listener == null ? SearchListener.NONE : listener;
        this.startNanos = System.nanoTime();
        this.deadlineNanos = timeLimitMillis > 0
                ? startNanos + timeLimitMillis * 1_000_000L
                : Long.MAX_VALUE;

        if (position.isTerminal()) {
            int score = terminalScore(position);
            Outcome outcome = Outcome.fromScoreSign(score);
            return new SearchResult(SearchResult.Certainty.PROVEN_EXACT, Optional.of(outcome),
                    Optional.empty(), true, score, OptionalInt.of(0), 0, 0, 0, List.of(),
                    "Position is already terminal.");
        }

        try {
            RootResult root = searchRoot();
            long elapsed = elapsedMillis();
            List<Move> pv = reconstructVariation(position);
            int plies;
            if (root.score() == 0) {
                plies = cells - position.movesPlayed();
            } else {
                int terminalPly = cells - Math.abs(root.score()) + 1;
                plies = terminalPly - position.movesPlayed();
            }
            return new SearchResult(SearchResult.Certainty.PROVEN_EXACT,
                    Optional.of(Outcome.fromScoreSign(root.score())),
                    Optional.ofNullable(root.bestMove()), true, root.score(),
                    OptionalInt.of(plies), plies, nodes, elapsed, pv,
                    "Proved by exhaustive alpha-beta search (negamax, transposition table, threat ordering).");
        } catch (SearchAbortedException e) {
            return SearchResult.unknown(nodes, elapsedMillis(),
                    "Exact proof aborted (" + e.reason() + ") after " + nodes + " nodes; no conclusion claimed.");
        }
    }

    public long nodesExplored() {
        return nodes;
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
        if ((nodes & 65535L) == 0L) {
            listener.onProgress(nodes, -1, Optional.empty());
        }
    }

    private int terminalScore(Position p) {
        Optional<Player> winner = p.winner();
        if (winner.isEmpty()) return 0;
        int magnitude = cells - p.movesPlayed() + 1;
        return winner.get() == p.sideToMove() ? magnitude : -magnitude;
    }

    private RootResult searchRoot() {
        nodes++;
        checkBudget();
        Prepared prep = prepareMoves();
        if (prep.forcedScore() != null) {
            if (prep.forcedMove() != null) principalMoves.put(pos.rawKey(), prep.forcedMove());
            return new RootResult(prep.forcedScore(), prep.forcedMove());
        }
        int alpha = -infinity;
        int beta = infinity;
        int best = -infinity;
        Move bestMove = null;
        for (Move move : prep.moves()) {
            pos.play(move);
            int score;
            try {
                score = -negamax(-beta, -alpha);
            } finally {
                pos.undo();
            }
            if (score > best) {
                best = score;
                bestMove = move;
            }
            if (best > alpha) alpha = best;
        }
        if (bestMove != null) principalMoves.put(pos.rawKey(), bestMove);
        return new RootResult(best, bestMove);
    }

    private int negamax(int alpha, int beta) {
        nodes++;
        checkBudget();
        if (pos.isTerminal()) return terminalScore(pos);

        int alphaOrig = alpha;
        int betaOrig = beta;
        PositionKey key = pos.canonicalKey();
        Entry hit = table.get(key);
        if (hit != null) {
            switch (hit.bound()) {
                case EXACT -> {
                    return hit.score();
                }
                case LOWER -> alpha = Math.max(alpha, hit.score());
                case UPPER -> beta = Math.min(beta, hit.score());
            }
            if (alpha >= beta) return hit.score();
            alphaOrig = alpha;
            betaOrig = beta;
        }

        Prepared prep = prepareMoves();
        if (prep.forcedScore() != null) {
            if (prep.forcedMove() != null && principalMoves.size() < MAX_TABLE_ENTRIES) {
                principalMoves.put(pos.rawKey(), prep.forcedMove());
            }
            store(key, prep.forcedScore(), Bound.EXACT);
            return prep.forcedScore();
        }

        int best = -infinity;
        Move bestMove = null;
        for (Move move : prep.moves()) {
            pos.play(move);
            int score;
            try {
                score = -negamax(-beta, -alpha);
            } finally {
                pos.undo();
            }
            if (score > best) {
                best = score;
                bestMove = move;
            }
            if (best > alpha) alpha = best;
            if (alpha >= beta) break;
        }

        Bound bound = best <= alphaOrig ? Bound.UPPER : (best >= betaOrig ? Bound.LOWER : Bound.EXACT);
        store(key, best, bound);
        if (bestMove != null && principalMoves.size() < MAX_TABLE_ENTRIES) {
            principalMoves.put(pos.rawKey(), bestMove);
        }
        return best;
    }

    private void store(PositionKey key, int score, Bound bound) {
        if (table.size() < MAX_TABLE_ENTRIES) {
            table.put(key, new Entry(score, bound));
        }
    }

    /**
     * Tactical shortcuts + move list preparation. Returns a forced exact
     * score when the position is decided within two plies.
     */
    private Prepared prepareMoves() {
        List<Move> legal = pos.legalMoves();
        Player side = pos.sideToMove();
        int moves = pos.movesPlayed();

        if (!rules.misere()) {
            List<Move> winners = pos.lineCompletingMoves(side);
            if (!winners.isEmpty()) {
                // Win on the next ply: score = cells - (moves+1) + 1.
                return new Prepared(List.of(), cells - moves, winners.get(0));
            }
            List<Move> threats = pos.lineCompletingMoves(side.other());
            if (threats.size() >= 2) {
                // Two distinct immediate threats cannot both be blocked: loss in 2 plies.
                return new Prepared(List.of(), -(cells - moves - 1), legal.isEmpty() ? null : legal.get(0));
            }
            if (threats.size() == 1) {
                legal = threats; // only the blocking move can avoid losing at once
            }
        } else {
            List<Move> suicides = pos.lineCompletingMoves(side);
            if (!suicides.isEmpty()) {
                List<Move> safe = new ArrayList<>();
                for (Move m : legal) {
                    if (!suicides.contains(m)) safe.add(m);
                }
                if (safe.isEmpty()) {
                    // Every move completes a line and loses immediately.
                    return new Prepared(List.of(), -(cells - moves), legal.get(0));
                }
                legal = safe;
            }
        }
        return new Prepared(orderMoves(legal), null, null);
    }

    private List<Move> orderMoves(List<Move> moves) {
        if (moves.size() <= 1) return moves;
        Player side = pos.sideToMove();
        long[] opponentSpots = rules.misere() ? null : pos.winningSpots(side.other());
        record Scored(Move move, int score) {
        }
        List<Scored> scored = new ArrayList<>(moves.size());
        int w = rules.width();
        for (Move move : moves) {
            int s = (Math.min(move.column(), w - 1 - move.column()) + 1) * 10;
            if (opponentSpots != null && Bits.test(opponentSpots, rules.index(move.column(), move.row()))) {
                s += 5_000; // occupies an opponent threat cell
            }
            pos.play(move);
            try {
                if (!pos.isTerminal()) {
                    s += pos.countWinningSpots(side) * 100;
                    if (!rules.misere() && !pos.lineCompletingMoves(pos.sideToMove()).isEmpty()) {
                        s -= 20_000; // hands the opponent an immediate win (e.g. playing under a threat)
                    }
                }
            } finally {
                pos.undo();
            }
            scored.add(new Scored(move, s));
        }
        scored.sort((a, b) -> Integer.compare(b.score(), a.score()));
        List<Move> out = new ArrayList<>(moves.size());
        for (Scored sc : scored) out.add(sc.move());
        return out;
    }

    private List<Move> reconstructVariation(Position original) {
        List<Move> pv = new ArrayList<>();
        Position cursor = original.copy();
        while (!cursor.isTerminal() && pv.size() <= cells) {
            Move m = principalMoves.get(cursor.rawKey());
            if (m == null) {
                // Node was resolved by a transposition-table hit during the
                // search, so no principal move was recorded. Recover the
                // obvious tactical move if there is one.
                if (!cursor.rules().misere()
                        && !cursor.lineCompletingMoves(cursor.sideToMove()).isEmpty()) {
                    m = cursor.lineCompletingMoves(cursor.sideToMove()).get(0);
                } else {
                    break;
                }
            }
            try {
                pv.add(cursor.play(m));
            } catch (RuntimeException e) {
                break;
            }
        }
        return pv;
    }
}
