package connectk.core.solver;

import connectk.core.model.Outcome;
import connectk.core.model.Position;
import connectk.core.theory.OpeningBook;
import connectk.core.theory.TheoryDatabase;
import connectk.core.theory.TheoryResult;

import java.util.List;
import java.util.Optional;
import java.util.OptionalInt;

/**
 * The entry point most callers want. It implements the project's solving
 * policy:
 *
 * <ol>
 *   <li>If published mathematical theory covers the position, its outcome
 *       is reported as proven (in AUTO mode the recommended move may still
 *       come from depth-limited search when theory publishes no move).</li>
 *   <li>Where an exact proof looks tractable, run the exact solver within
 *       the exact budget.</li>
 *   <li>Otherwise play/analyse with depth-limited search and label the
 *       result as heuristic — never as a proof.</li>
 * </ol>
 *
 * Stateless and thread-safe; each call builds its own search objects.
 */
public final class ConnectKSolver {

    private final OpeningBook book;
    private final int threads;

    public ConnectKSolver() {
        this(null, 1);
    }

    /**
     * @param book optional generated opening book used by the fast exact
     *             solver (exact scores without search for covered
     *             positions); may be null
     */
    public ConnectKSolver(OpeningBook book) {
        this(book, 1);
    }

    /**
     * @param book    optional generated opening book; may be null
     * @param threads worker threads for exact proving (1 = sequential);
     *                values &gt; 1 use {@link ParallelExactSolver}'s
     *                frontier splitting for the exact stage
     */
    public ConnectKSolver(OpeningBook book, int threads) {
        if (threads < 1) throw new IllegalArgumentException("threads must be >= 1");
        this.book = book;
        this.threads = threads;
    }

    public SearchResult solve(Position position, SolveRequest request) {
        return solve(position, request, CancellationToken.NONE, SearchListener.NONE);
    }

    public SearchResult solve(Position position, SolveRequest request,
                              CancellationToken token, SearchListener listener) {
        long startNanos = System.nanoTime();
        if (token == null) token = CancellationToken.NONE;
        if (listener == null) listener = SearchListener.NONE;

        if (position.isTerminal()) {
            return new ExactSolver().solve(position, 0, 0, token, listener);
        }

        // DEPTH_LIMITED means "heuristic search only, never claim a proof",
        // so theory is not consulted in that mode even if useTheory is set.
        Optional<TheoryResult> theory = request.useTheory() && request.mode() != SolveMode.DEPTH_LIMITED
                ? TheoryDatabase.lookup(position)
                : Optional.empty();

        // Theory even publishes an optimal move: no search needed at all.
        if (request.mode() != SolveMode.DEPTH_LIMITED
                && theory.isPresent() && theory.get().provenBestMove().isPresent()) {
            TheoryResult t = theory.get();
            int score = theoryScore(position, t);
            return new SearchResult(SearchResult.Certainty.PROVEN_BY_THEORY,
                    Optional.of(t.outcomeForSideToMove()), t.provenBestMove(), true,
                    score, t.pliesToEnd(), t.pliesToEnd().orElse(0), 0, elapsed(startNanos),
                    List.copyOf(t.provenBestMove().map(List::of).orElse(List.of())),
                    "Outcome and move from published theory: " + t.source());
        }

        SearchResult exactAttempt = null;
        boolean attemptExact = switch (request.mode()) {
            case EXACT -> true;
            case DEPTH_LIMITED -> false;
            case AUTO -> looksTractable(position);
        };

        if (attemptExact) {
            long exactTime = request.mode() == SolveMode.EXACT
                    ? request.timeLimitMillis()
                    : request.exactTimeLimitMillis();
            exactAttempt = solveExact(
                    position, request.exactNodeLimit(), exactTime, token, listener);
            if (exactAttempt.certainty() == SearchResult.Certainty.PROVEN_EXACT) {
                return exactAttempt;
            }
            if (request.mode() == SolveMode.EXACT) {
                // Honour the caller's "proof or nothing" request, but still
                // surface a theoretical outcome if one is published.
                if (theory.isPresent()) {
                    TheoryResult t = theory.get();
                    return new SearchResult(SearchResult.Certainty.PROVEN_BY_THEORY,
                            Optional.of(t.outcomeForSideToMove()), Optional.empty(), false,
                            theoryScore(position, t), t.pliesToEnd(), 0,
                            exactAttempt.nodes(), elapsed(startNanos), List.of(),
                            "Exact search hit its budget, but the outcome is published theory: "
                                    + t.source() + " No proven move available.");
                }
                return exactAttempt;
            }
        }

        long remainingMillis = remainingMillis(request, startNanos);
        SearchResult heuristic = new DepthLimitedSolver().solve(
                position, request.maxDepth(), request.nodeLimit(), remainingMillis, token, listener);

        long totalNodes = heuristic.nodes() + (exactAttempt == null ? 0 : exactAttempt.nodes());
        if (theory.isPresent()) {
            TheoryResult t = theory.get();
            String note = "Outcome is published mathematical theory (" + t.source() + ")."
                    + " The recommended move comes from depth-limited search and is not"
                    + " certified optimal. " + heuristic.explanation();
            return new SearchResult(SearchResult.Certainty.PROVEN_BY_THEORY,
                    Optional.of(t.outcomeForSideToMove()), heuristic.bestMove(), false,
                    t.pliesToEnd().isPresent() ? theoryScore(position, t) : heuristic.score(),
                    t.pliesToEnd(), heuristic.depthReached(), totalNodes, elapsed(startNanos),
                    heuristic.principalVariation(), note);
        }

        if (exactAttempt != null) {
            return new SearchResult(heuristic.certainty(), heuristic.outcomeForSideToMove(),
                    heuristic.bestMove(), false, heuristic.score(), heuristic.pliesToEnd(),
                    heuristic.depthReached(), totalNodes, elapsed(startNanos),
                    heuristic.principalVariation(),
                    "Exact proof did not finish within its budget (" + exactAttempt.nodes()
                            + " nodes), so this position is not mathematically solved here. "
                            + heuristic.explanation());
        }
        return heuristic;
    }

    /** Exact search via the fast one-word gravity solver when supported, else the generic solver. */
    private SearchResult solveExact(Position position, long nodeLimit, long timeMillis,
                                           CancellationToken token, SearchListener listener) {
        if (threads > 1) {
            return new ParallelExactSolver(threads, book)
                    .solve(position, nodeLimit, timeMillis, token, listener);
        }
        if (FastExactSolver.supports(position.rules())) {
            FastExactSolver fast = new FastExactSolver();
            fast.setOpeningBook(book);
            return fast.solve(position, nodeLimit, timeMillis, token, listener);
        }
        return new ExactSolver().solve(position, nodeLimit, timeMillis, token, listener);
    }

    private static long elapsed(long startNanos) {
        return (System.nanoTime() - startNanos) / 1_000_000L;
    }

    private static long remainingMillis(SolveRequest request, long startNanos) {
        if (request.timeLimitMillis() <= 0) return 0;
        long left = request.timeLimitMillis() - elapsed(startNanos);
        return Math.max(50, left);
    }

    /**
     * Heuristic tractability guess for AUTO mode. Conservative on purpose:
     * failing an exact attempt only costs its (separate, small) budget.
     */
    private static boolean looksTractable(Position p) {
        int cells = p.rules().cells();
        int remaining = cells - p.movesPlayed();
        if (remaining <= 22) return true;
        if (cells <= 30) return true;
        return p.rules().placement() == connectk.core.model.Placement.GRAVITY
                && p.rules().width() <= 7 && remaining <= 28;
    }

    /** Convert a theory outcome/distance to the exact solver's score convention. */
    private static int theoryScore(Position p, TheoryResult t) {
        if (t.outcomeForSideToMove() == Outcome.DRAW || t.pliesToEnd().isEmpty()) return 0;
        int terminalPly = p.movesPlayed() + t.pliesToEnd().getAsInt();
        int magnitude = p.rules().cells() - terminalPly + 1;
        return t.outcomeForSideToMove() == Outcome.WIN ? magnitude : -magnitude;
    }
}
