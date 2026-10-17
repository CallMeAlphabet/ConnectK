package connectk.core.solver;

public enum SolveMode {
    /**
     * Use published theory where available, attempt an exact proof within
     * the exact budget when the position looks tractable, and otherwise
     * fall back to depth-limited search (result labelled heuristic).
     */
    AUTO,
    /** Exact proof search only; returns UNKNOWN if the budget runs out. */
    EXACT,
    /** Depth-limited heuristic search only. Never claims a proof. */
    DEPTH_LIMITED
}
