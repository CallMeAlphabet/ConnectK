package connectk.core.solver;

import java.util.Objects;

/**
 * Everything a solve run is allowed to spend.
 *
 * @param mode               AUTO / EXACT / DEPTH_LIMITED
 * @param maxDepth           plies for depth-limited search (>= 1)
 * @param timeLimitMillis    overall wall-clock budget (0 = no limit)
 * @param nodeLimit          overall node budget for heuristic search (0 = no limit)
 * @param exactTimeLimitMillis budget for an exact attempt in AUTO mode (0 = no limit)
 * @param exactNodeLimit     node budget for exact search (0 = no limit)
 * @param useTheory          consult the published-results database
 */
public record SolveRequest(
        SolveMode mode,
        int maxDepth,
        long timeLimitMillis,
        long nodeLimit,
        long exactTimeLimitMillis,
        long exactNodeLimit,
        boolean useTheory) {

    public SolveRequest {
        Objects.requireNonNull(mode, "mode");
        if (maxDepth < 1) throw new IllegalArgumentException("maxDepth must be >= 1");
        if (timeLimitMillis < 0 || nodeLimit < 0 || exactTimeLimitMillis < 0 || exactNodeLimit < 0) {
            throw new IllegalArgumentException("limits must be >= 0 (0 means unlimited)");
        }
    }

    public static SolveRequest defaults() {
        return new SolveRequest(SolveMode.AUTO, 10, 5_000, 20_000_000, 1_500, 5_000_000, true);
    }

    public static SolveRequest exact(long timeLimitMillis, long nodeLimit) {
        return new SolveRequest(SolveMode.EXACT, 1, timeLimitMillis, 0, timeLimitMillis, nodeLimit, true);
    }

    public static SolveRequest depthLimited(int depth, long timeLimitMillis) {
        return new SolveRequest(SolveMode.DEPTH_LIMITED, depth, timeLimitMillis, 0, 0, 0, true);
    }
}
