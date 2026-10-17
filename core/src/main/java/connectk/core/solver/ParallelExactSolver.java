package connectk.core.solver;

import connectk.core.model.Move;
import connectk.core.model.Outcome;
import connectk.core.model.Player;
import connectk.core.model.Position;
import connectk.core.model.PositionKey;
import connectk.core.model.Rules;
import connectk.core.theory.OpeningBook;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.OptionalInt;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.ExecutorCompletionService;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * Exact (proof) solver that parallelizes proof search across worker
 * threads, with the same score convention and result semantics as the
 * sequential solvers: {@code PROVEN_EXACT} only when every part of the
 * proof completed, otherwise {@code UNKNOWN_LIMIT_REACHED} — never a
 * guess.
 *
 * <p>Two strategies, by rule support:</p>
 * <ul>
 *   <li><b>Fast-path rules</b> ({@link FastExactSolver#supports}):
 *       <em>root principal-variation splitting</em>. A heuristically
 *       best first move is solved exactly, establishing a root alpha
 *       bound; every sibling move is then <em>refuted</em> in parallel
 *       with null-window bound tests ("prove this move is no better
 *       than alpha"), which are typically far cheaper than exact
 *       proofs. A move that survives refutation is solved exactly,
 *       raises alpha, and the remaining survivors are re-tested. This
 *       preserves sequential PVS's pruning logic while parallelizing
 *       its dominant cost. (Naive frontier splitting — solving every
 *       depth-2 subposition exactly — was measured at ~8x the nodes
 *       and 4x the wall time of sequential search on hard openings,
 *       because it loses root bound sharing; see docs/PERFORMANCE.md.)</li>
 *   <li><b>Other rules</b>: frontier splitting with generic workers —
 *       expand a couple of plies, solve the unique frontier positions
 *       exactly in parallel, combine by minimax. Less efficient, but
 *       the generic solver has no bounded-search API, and these
 *       variants are typically solved on smaller trees.</li>
 * </ul>
 *
 * <p>Semantics notes: the time limit is one shared deadline; the node
 * limit applies per worker task; workers own separate transposition
 * tables (no sharing), so total nodes exceed the sequential count
 * somewhat in exchange for wall-clock speedup.</p>
 */
public final class ParallelExactSolver {

    private final int threads;
    private final OpeningBook book;

    public ParallelExactSolver(int threads) {
        this(threads, null);
    }

    public ParallelExactSolver(int threads, OpeningBook book) {
        if (threads < 1) throw new IllegalArgumentException("threads must be >= 1");
        this.threads = threads;
        this.book = book;
    }

    public int threads() {
        return threads;
    }

    public SearchResult solve(Position position, long nodeLimit, long timeLimitMillis,
                              CancellationToken token, SearchListener listener) {
        if (threads <= 1 || position.isTerminal()) {
            return solveSequential(position, nodeLimit, timeLimitMillis, token, listener);
        }
        if (FastExactSolver.supports(position.rules())) {
            return solveRootParallel(position, nodeLimit, timeLimitMillis, token, listener);
        }
        return solveFrontier(position, nodeLimit, timeLimitMillis, token, listener);
    }

    /** Root principal-variation splitting for fast-path rules (see class doc). */
    private SearchResult solveRootParallel(Position position, long nodeLimit,
                                           long timeLimitMillis, CancellationToken token,
                                           SearchListener listener) {
        long startNanos = System.nanoTime();
        long deadlineNanos = timeLimitMillis > 0
                ? startNanos + timeLimitMillis * 1_000_000L : Long.MAX_VALUE;
        Rules rules = position.rules();
        int cells = rules.cells();
        int rootMovesPlayed = position.movesPlayed();
        AtomicBoolean abort = new AtomicBoolean(false);
        CancellationToken workerToken = () -> abort.get() || token.isCancelled();

        List<Move> moves = new ArrayList<>(position.legalMoves());
        long totalNodes = 0;
        // Cheap heuristic pass: a good first move gives a strong alpha early,
        // which is what makes the parallel refutations cheap.
        {
            long hintMs = deadlineNanos == Long.MAX_VALUE ? 250
                    : Math.max(10, Math.min(1_000, remainingMillis(deadlineNanos) / 20));
            SearchResult hint = new DepthLimitedSolver().solve(position,
                    Math.min(12, cells - rootMovesPlayed), 300_000, hintMs,
                    workerToken, SearchListener.NONE);
            totalNodes += hint.nodes();
            if (hint.bestMove().isPresent()) {
                int col = hint.bestMove().get().column();
                for (int i = 1; i < moves.size(); i++) {
                    if (moves.get(i).column() == col) {
                        moves.add(0, moves.remove(i));
                        break;
                    }
                }
            }
        }
        int n = moves.size();
        Position[] children = new Position[n];
        for (int i = 0; i < n; i++) {
            children[i] = position.copy();
            children[i].play(moves.get(i));
        }

        // Phase 1: solve the first move exactly -> root alpha bound.
        SearchResult bestChild = solveSequential(children[0], nodeLimit,
                remainingMillis(deadlineNanos), workerToken, SearchListener.NONE);
        totalNodes += bestChild.nodes();
        if (bestChild.certainty() != SearchResult.Certainty.PROVEN_EXACT) {
            return unknownParallel(totalNodes, startNanos,
                    "first root move not proven within budget");
        }
        int alpha = -bestChild.score();
        int bestIdx = 0;
        int exactSolves = 1;
        int refutations = 0;

        // Phase 2: refute siblings in parallel against alpha; exact-solve survivors.
        List<Integer> remaining = new ArrayList<>();
        for (int i = 1; i < n; i++) remaining.add(i);
        ExecutorService pool = Executors.newFixedThreadPool(threads);
        try {
            while (!remaining.isEmpty()) {
                int testAlpha = alpha;
                ExecutorCompletionService<RefOutcome> completion =
                        new ExecutorCompletionService<>(pool);
                for (int idx : remaining) {
                    completion.submit(() -> {
                        FastExactSolver worker = new FastExactSolver();
                        worker.setOpeningBook(book);
                        FastExactSolver.BoundResult br = worker.searchBound(children[idx],
                                -testAlpha - 1, -testAlpha, nodeLimit,
                                remainingMillis(deadlineNanos), workerToken);
                        return new RefOutcome(idx, br);
                    });
                }
                Map<Integer, FastExactSolver.BoundResult> results = new HashMap<>();
                boolean abortedAny = false;
                for (int i = 0; i < remaining.size(); i++) {
                    RefOutcome outcome;
                    try {
                        outcome = completion.take().get();
                    } catch (InterruptedException e) {
                        Thread.currentThread().interrupt();
                        abortedAny = true;
                        break;
                    } catch (ExecutionException e) {
                        abortedAny = true;
                        continue;
                    }
                    totalNodes += outcome.result.nodes();
                    if (outcome.result.aborted()) abortedAny = true;
                    else results.put(outcome.idx, outcome.result);
                }
                listener.onProgress(totalNodes, 0, Optional.of(moves.get(bestIdx)));
                if (abortedAny) {
                    abort.set(true);
                    return unknownParallel(totalNodes, startNanos,
                            "a refutation task exhausted the budget");
                }
                List<Integer> candidates = new ArrayList<>();
                for (int idx : remaining) {
                    // value < -alpha: this move might beat alpha; else refuted.
                    if (results.get(idx).value() < -testAlpha) candidates.add(idx);
                    else refutations++;
                }
                if (candidates.isEmpty()) break;
                int idx = candidates.get(0);
                SearchResult res = solveSequential(children[idx], nodeLimit,
                        remainingMillis(deadlineNanos), workerToken, SearchListener.NONE);
                totalNodes += res.nodes();
                exactSolves++;
                if (res.certainty() != SearchResult.Certainty.PROVEN_EXACT) {
                    return unknownParallel(totalNodes, startNanos,
                            "a surviving root move was not proven within budget");
                }
                int v = -res.score();
                if (v > alpha) {
                    alpha = v;
                    bestIdx = idx;
                    bestChild = res;
                }
                candidates.remove(0);
                remaining = candidates; // retest survivors against the raised alpha
            }
        } finally {
            pool.shutdownNow();
        }

        int score = alpha;
        List<Move> pv = new ArrayList<>();
        pv.add(moves.get(bestIdx));
        pv.addAll(bestChild.principalVariation());
        int plies = score == 0
                ? cells - rootMovesPlayed
                : cells - Math.abs(score) + 1 - rootMovesPlayed;
        long elapsed = (System.nanoTime() - startNanos) / 1_000_000L;
        return new SearchResult(SearchResult.Certainty.PROVEN_EXACT,
                Optional.of(Outcome.fromScoreSign(score)), Optional.of(moves.get(bestIdx)), true,
                score, OptionalInt.of(plies), plies, totalNodes, elapsed, pv,
                "Proved by parallel exact search: root principal-variation splitting on "
                        + threads + " threads (" + exactSolves + " exact root-move solve(s), "
                        + refutations + " sibling refutation(s) by parallel null-window "
                        + "bound tests against the shared alpha bound).");
    }

    private SearchResult unknownParallel(long totalNodes, long startNanos, String why) {
        long elapsed = (System.nanoTime() - startNanos) / 1_000_000L;
        return SearchResult.unknown(totalNodes, elapsed,
                "Parallel exact search (root principal-variation splitting, " + threads
                        + " threads) incomplete: " + why + "; no outcome claimed.");
    }

    private static long remainingMillis(long deadlineNanos) {
        if (deadlineNanos == Long.MAX_VALUE) return 0;
        return Math.max(1L, (deadlineNanos - System.nanoTime()) / 1_000_000L);
    }

    private record RefOutcome(int idx, FastExactSolver.BoundResult result) {
    }

    /** Frontier splitting for rules without a bounded-search fast path (see class doc). */
    private SearchResult solveFrontier(Position position, long nodeLimit, long timeLimitMillis,
                                       CancellationToken token, SearchListener listener) {
        long startNanos = System.nanoTime();
        Rules rules = position.rules();
        int cells = rules.cells();
        int rootMoves = position.movesPlayed();
        int branching = position.legalMoves().size();
        if (branching == 0) {
            return solveSequential(position, nodeLimit, timeLimitMillis, token, listener);
        }
        int depth = frontierDepth(branching, cells - rootMoves);

        Node root = new Node(position.copy(), null);
        Map<PositionKey, Task> tasksByKey = new HashMap<>();
        List<Task> tasks = new ArrayList<>();
        expand(root, depth, tasksByKey, tasks);

        long deadlineNanos = timeLimitMillis > 0
                ? startNanos + timeLimitMillis * 1_000_000L : Long.MAX_VALUE;
        AtomicBoolean abort = new AtomicBoolean(false);
        CancellationToken workerToken = () -> abort.get() || token.isCancelled();

        long totalNodes = 0;
        int provenTasks = 0;
        boolean allProven = true;
        if (!tasks.isEmpty()) {
            ExecutorService pool = Executors.newFixedThreadPool(threads);
            try {
                ExecutorCompletionService<TaskOutcome> completion =
                        new ExecutorCompletionService<>(pool);
                for (Task task : tasks) {
                    completion.submit(() -> {
                        long budgetMs = 0;
                        if (deadlineNanos != Long.MAX_VALUE) {
                            budgetMs = Math.max(1L,
                                    (deadlineNanos - System.nanoTime()) / 1_000_000L);
                        }
                        SearchResult r = solveSequential(task.position, nodeLimit,
                                budgetMs, workerToken, SearchListener.NONE);
                        return new TaskOutcome(task, r);
                    });
                }
                for (int i = 0; i < tasks.size(); i++) {
                    TaskOutcome outcome;
                    try {
                        Future<TaskOutcome> f = completion.take();
                        outcome = f.get();
                    } catch (InterruptedException e) {
                        Thread.currentThread().interrupt();
                        abort.set(true);
                        allProven = false;
                        break;
                    } catch (ExecutionException e) {
                        abort.set(true);
                        allProven = false;
                        continue;
                    }
                    totalNodes += outcome.result.nodes();
                    if (outcome.result.certainty() == SearchResult.Certainty.PROVEN_EXACT) {
                        provenTasks++;
                        for (Node leaf : outcome.task.leaves) {
                            leaf.value = outcome.result.score();
                            leaf.leafResult = outcome.result;
                        }
                    } else {
                        allProven = false;
                        abort.set(true); // siblings notice within their budget checks
                    }
                    listener.onProgress(totalNodes, depth, Optional.empty());
                }
            } finally {
                pool.shutdownNow();
            }
        }

        long elapsed = (System.nanoTime() - startNanos) / 1_000_000L;
        if (!allProven) {
            return SearchResult.unknown(totalNodes, elapsed,
                    "Parallel exact search (frontier depth " + depth + ", " + threads
                            + " threads) did not prove all frontier subpositions within budget ("
                            + provenTasks + " of " + tasks.size()
                            + " proven); no outcome claimed.");
        }

        int score = combine(root);
        List<Move> pv = principalVariation(root);
        Optional<Move> bestMove = pv.isEmpty() ? Optional.empty() : Optional.of(pv.get(0));
        int plies = score == 0
                ? cells - rootMoves
                : cells - Math.abs(score) + 1 - rootMoves;
        String workerName = FastExactSolver.supports(rules)
                ? "fast one-word gravity solver" : "generic exact solver";
        return new SearchResult(SearchResult.Certainty.PROVEN_EXACT,
                Optional.of(Outcome.fromScoreSign(score)), bestMove, true,
                score, OptionalInt.of(plies), plies, totalNodes, elapsed, pv,
                "Proved by parallel exact search: frontier split at depth " + depth
                        + " into " + tasks.size() + " unique subpositions, solved by "
                        + threads + " worker threads (" + workerName
                        + ") and combined by minimax.");
    }

    private SearchResult solveSequential(Position position, long nodeLimit, long timeLimitMillis,
                                         CancellationToken token, SearchListener listener) {
        if (FastExactSolver.supports(position.rules())) {
            FastExactSolver fast = new FastExactSolver();
            fast.setOpeningBook(book);
            return fast.solve(position, nodeLimit, timeLimitMillis, token, listener);
        }
        return new ExactSolver().solve(position, nodeLimit, timeLimitMillis, token, listener);
    }

    private int frontierDepth(int branching, int remainingPlies) {
        int depth = threads >= 8 && branching <= 8 ? 3 : 2;
        while (depth > 1 && Math.pow(branching, depth) > 8192) depth--;
        return Math.max(1, Math.min(depth, remainingPlies));
    }

    /** Expand the tree to the frontier; register unique leaf tasks (terminal leaves get values). */
    private void expand(Node node, int levelsToGo,
                        Map<PositionKey, Task> tasksByKey, List<Task> tasks) {
        if (levelsToGo == 0 || node.position.isTerminal()
                || node.position.legalMoves().isEmpty()) {
            markLeaf(node, tasksByKey, tasks);
            return;
        }
        for (Move move : node.position.legalMoves()) {
            Position childPos = node.position.copy();
            childPos.play(move);
            Node child = new Node(childPos, move);
            node.children.add(child);
            expand(child, levelsToGo - 1, tasksByKey, tasks);
        }
    }

    private void markLeaf(Node node, Map<PositionKey, Task> tasksByKey, List<Task> tasks) {
        node.leaf = true;
        Position p = node.position;
        if (p.isTerminal()) {
            node.value = terminalScore(p);
            return;
        }
        PositionKey key = p.rawKey();
        Task task = tasksByKey.get(key);
        if (task == null) {
            task = new Task(p.copy());
            tasksByKey.put(key, task);
            tasks.add(task);
        }
        task.leaves.add(node);
    }

    /** Exact score of a terminal position, from the side-to-move perspective. */
    private static int terminalScore(Position p) {
        Optional<Player> winner = p.winner();
        if (winner.isEmpty()) return 0; // full board, draw
        int magnitude = p.rules().cells() - p.movesPlayed() + 1;
        return winner.get() == p.sideToMove() ? magnitude : -magnitude;
    }

    /** Minimax from the frontier up; returns the node's exact value. */
    private int combine(Node node) {
        if (node.leaf) return node.value;
        int best = Integer.MIN_VALUE;
        for (Node child : node.children) {
            best = Math.max(best, -combine(child));
        }
        node.value = best;
        return best;
    }

    private List<Move> principalVariation(Node root) {
        List<Move> pv = new ArrayList<>();
        Node node = root;
        while (!node.leaf) {
            Node chosen = null;
            for (Node child : node.children) {
                if (-child.value == node.value) {
                    chosen = child;
                    break;
                }
            }
            if (chosen == null) break; // cannot happen when all proven
            pv.add(chosen.moveFromParent);
            node = chosen;
        }
        if (node.leafResult != null) pv.addAll(node.leafResult.principalVariation());
        return pv;
    }

    private static final class Node {
        final Position position;
        final Move moveFromParent;
        final List<Node> children = new ArrayList<>();
        boolean leaf;
        int value;
        SearchResult leafResult;

        Node(Position position, Move moveFromParent) {
            this.position = position;
            this.moveFromParent = moveFromParent;
        }
    }

    private static final class Task {
        final Position position;
        final List<Node> leaves = new ArrayList<>();

        Task(Position position) {
            this.position = position;
        }
    }

    private record TaskOutcome(Task task, SearchResult result) {
    }
}
