package connectk.core.model;

/**
 * A line direction on the board. Columns increase to the right, rows
 * increase upwards (row 0 is the bottom row).
 *
 * <p>The bitboard layout is {@code index = column * (height+1) + row}
 * (the {@code +1} is a never-set sentinel bit on top of each column), so
 * each direction corresponds to a constant bit shift.
 */
public enum Direction {
    VERTICAL(0, 1),
    HORIZONTAL(1, 0),
    DIAGONAL_UP(1, 1),
    DIAGONAL_DOWN(1, -1);

    private final int dCol;
    private final int dRow;

    Direction(int dCol, int dRow) {
        this.dCol = dCol;
        this.dRow = dRow;
    }

    public int dCol() {
        return dCol;
    }

    public int dRow() {
        return dRow;
    }

    /** Bit shift for one step in this direction given the board stride {@code height+1}. */
    public int shift(int stride) {
        return dCol * stride + dRow;
    }

    /** The direction this becomes under a left-right mirror of the board. */
    public Direction mirrored() {
        return switch (this) {
            case DIAGONAL_UP -> DIAGONAL_DOWN;
            case DIAGONAL_DOWN -> DIAGONAL_UP;
            default -> this;
        };
    }
}
