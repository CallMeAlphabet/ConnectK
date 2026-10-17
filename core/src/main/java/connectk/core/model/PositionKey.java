package connectk.core.model;

import java.util.Arrays;

/**
 * Immutable transposition-table key: the bitboard of the side to move plus
 * the occupancy bitboard. Instances created via
 * {@link Position#canonicalKey()} additionally fold in the left-right
 * mirror symmetry; {@link Position#rawKey()} does not.
 */
public final class PositionKey {

    private final long[] current;
    private final long[] mask;
    private final int hash;

    public PositionKey(long[] current, long[] mask) {
        this.current = current.clone();
        this.mask = mask.clone();
        this.hash = 31 * Arrays.hashCode(this.current) + Arrays.hashCode(this.mask);
    }

    public long[] currentBits() {
        return current.clone();
    }

    public long[] maskBits() {
        return mask.clone();
    }

    @Override
    public boolean equals(Object o) {
        if (this == o) return true;
        if (!(o instanceof PositionKey other)) return false;
        return Arrays.equals(current, other.current) && Arrays.equals(mask, other.mask);
    }

    @Override
    public int hashCode() {
        return hash;
    }

    @Override
    public String toString() {
        return "key[cur=" + Arrays.toString(current) + ",mask=" + Arrays.toString(mask) + "]";
    }
}
