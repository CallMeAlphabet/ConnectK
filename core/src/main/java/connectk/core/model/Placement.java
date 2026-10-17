package connectk.core.model;

/** How a stone may be placed. */
public enum Placement {
    /** Choose a column; the stone falls to the lowest empty cell (Connect-4 style). */
    GRAVITY,
    /** Place a stone on any empty cell (Gomoku / m,n,k-game style). */
    FREE
}
