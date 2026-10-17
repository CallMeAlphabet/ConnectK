package connectk.core.model;

import java.util.EnumSet;
import java.util.Objects;

/**
 * An immutable, fully-specified Connect-k ruleset.
 *
 * <p>Rules are only ever consulted for move generation, line detection and
 * terminal evaluation — every search algorithm in this project treats them
 * as parameters. That is what lets one solver cover Connect-4/5/6-style
 * games with or without diagonals, with or without gravity, and in misere
 * form (making a line loses).
 *
 * @param width     number of columns (>= 1)
 * @param height    number of rows (>= 1)
 * @param connect   line length needed ("k"), >= 2
 * @param placement gravity (drop into a column) or free placement
 * @param directions which line directions count (non-empty)
 * @param misere    if true, completing a line <em>loses</em>
 */
public record Rules(
        int width,
        int height,
        int connect,
        Placement placement,
        EnumSet<Direction> directions,
        boolean misere) {

    public Rules {
        Objects.requireNonNull(placement, "placement");
        Objects.requireNonNull(directions, "directions");
        if (directions.isEmpty()) throw new IllegalArgumentException("directions must not be empty");
        directions = EnumSet.copyOf(directions);
        if (width < 1) throw new IllegalArgumentException("width must be >= 1, got " + width);
        if (height < 1) throw new IllegalArgumentException("height must be >= 1, got " + height);
        if (connect < 2) throw new IllegalArgumentException("connect (k) must be >= 2, got " + connect);
        int bits = width * (height + 1);
        if (bits > 4096) throw new IllegalArgumentException("board too large: " + bits + " bitboard bits");
        int maxLine = 0;
        for (Direction d : directions) {
            int len = switch (d) {
                case HORIZONTAL -> width;
                case VERTICAL -> height;
                case DIAGONAL_UP, DIAGONAL_DOWN -> Math.min(width, height);
            };
            maxLine = Math.max(maxLine, len);
        }
        if (connect > maxLine) {
            throw new IllegalArgumentException(
                    "k=" + connect + " can never be achieved on a " + width + "x" + height
                            + " board with directions " + directions + " (max possible line is " + maxLine + ")");
        }
    }

    @Override
    public EnumSet<Direction> directions() {
        return EnumSet.copyOf(directions);
    }

    /** Bits per column in the bitboard layout: {@code height} cells + 1 sentinel bit. */
    public int stride() {
        return height + 1;
    }

    public int cells() {
        return width * height;
    }

    /** Total bitboard bits including sentinels. */
    public int bitCount() {
        return width * stride();
    }

    public int wordCount() {
        return (bitCount() + 63) >>> 6;
    }

    public int index(int column, int row) {
        return column * stride() + row;
    }

    public int columnOf(int index) {
        return index / stride();
    }

    public int rowOf(int index) {
        return index % stride();
    }

    /** Bitmask with exactly the playable cells set (sentinel bits clear). Fresh copy per call. */
    public long[] boardMask() {
        long[] mask = Bits.create(wordCount());
        for (int c = 0; c < width; c++) {
            for (int r = 0; r < height; r++) {
                Bits.set(mask, index(c, r));
            }
        }
        return mask;
    }

    public boolean hasDiagonals() {
        return directions.contains(Direction.DIAGONAL_UP) || directions.contains(Direction.DIAGONAL_DOWN);
    }

    /**
     * True if the ruleset is invariant under a left-right mirror, i.e. it is
     * safe to canonicalise transposition-table keys by mirroring. Mirroring
     * swaps the two diagonals, so both or neither must be present.
     */
    public boolean mirrorSymmetric() {
        return directions.contains(Direction.DIAGONAL_UP) == directions.contains(Direction.DIAGONAL_DOWN);
    }

    public static EnumSet<Direction> allDirections() {
        return EnumSet.allOf(Direction.class);
    }

    public static EnumSet<Direction> orthogonalDirections() {
        return EnumSet.of(Direction.HORIZONTAL, Direction.VERTICAL);
    }

    /** The classic game: 7x6, connect 4, gravity, all directions. */
    public static Rules standardConnect4() {
        return new Rules(7, 6, 4, Placement.GRAVITY, allDirections(), false);
    }

    public static Rules gravity(int width, int height, int connect, boolean diagonals) {
        return new Rules(width, height, connect, Placement.GRAVITY,
                diagonals ? allDirections() : orthogonalDirections(), false);
    }

    public static Rules free(int width, int height, int connect, boolean diagonals) {
        return new Rules(width, height, connect, Placement.FREE,
                diagonals ? allDirections() : orthogonalDirections(), false);
    }

    public Rules withMisere(boolean newMisere) {
        return new Rules(width, height, connect, placement, directions, newMisere);
    }

    @Override
    public String toString() {
        return "Connect-" + connect + " " + width + "x" + height + " " + placement
                + (hasDiagonals() ? " diagonals" : " no-diagonals") + (misere ? " misere" : "");
    }
}
