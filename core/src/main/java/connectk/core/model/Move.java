package connectk.core.model;

/**
 * A move: the cell a stone ends up in. For gravity games an unresolved move
 * may carry {@code row == -1} ("drop in this column, wherever it lands");
 * {@link Position#play(Move)} resolves it to the landing row. Legal-move
 * lists always contain resolved moves.
 */
public record Move(int column, int row) {

    public Move {
        if (column < 0) throw new IllegalArgumentException("column < 0");
        if (row < -1) throw new IllegalArgumentException("row < -1");
    }

    public static Move column(int column) {
        return new Move(column, -1);
    }

    public static Move cell(int column, int row) {
        return new Move(column, row);
    }

    public boolean isResolved() {
        return row >= 0;
    }

    /** Human notation, 1-based: {@code "4"} for a column move, {@code "4:2"} for a cell. */
    public String notation() {
        return isResolved() ? (column + 1) + ":" + (row + 1) : Integer.toString(column + 1);
    }

    @Override
    public String toString() {
        return notation();
    }
}
