package connectk.core.model;

import java.util.Arrays;

/**
 * Low-level operations on little-endian bit arrays ({@code long[0]} holds
 * bits 0..63). Boards with at most 64 bitboard bits use a single word; the
 * same code supports arbitrarily large boards (e.g. 19x19) with more words.
 *
 * <p>All methods are allocation-free except {@link #create(int)} and friends;
 * destination arrays may alias source arrays where that is meaningful.
 */
public final class Bits {

    private Bits() {
    }

    public static long[] create(int words) {
        return new long[words];
    }

    public static void copy(long[] src, long[] dest) {
        System.arraycopy(src, 0, dest, 0, dest.length);
    }

    public static void clearAll(long[] a) {
        Arrays.fill(a, 0L);
    }

    public static boolean test(long[] a, int index) {
        return (a[index >>> 6] >>> (index & 63) & 1L) != 0L;
    }

    public static void set(long[] a, int index) {
        a[index >>> 6] |= 1L << (index & 63);
    }

    public static void clear(long[] a, int index) {
        a[index >>> 6] &= ~(1L << (index & 63));
    }

    public static boolean any(long[] a) {
        for (long w : a) {
            if (w != 0L) return true;
        }
        return false;
    }

    public static int bitCount(long[] a) {
        int n = 0;
        for (long w : a) n += Long.bitCount(w);
        return n;
    }

    public static void and(long[] a, long[] b, long[] dest) {
        for (int i = 0; i < dest.length; i++) dest[i] = a[i] & b[i];
    }

    public static void or(long[] a, long[] b, long[] dest) {
        for (int i = 0; i < dest.length; i++) dest[i] = a[i] | b[i];
    }

    public static void xor(long[] a, long[] b, long[] dest) {
        for (int i = 0; i < dest.length; i++) dest[i] = a[i] ^ b[i];
    }

    /** {@code dest = a & ~b} */
    public static void andNot(long[] a, long[] b, long[] dest) {
        for (int i = 0; i < dest.length; i++) dest[i] = a[i] & ~b[i];
    }

    public static int andCount(long[] a, long[] b) {
        int n = 0;
        for (int i = 0; i < a.length; i++) n += Long.bitCount(a[i] & b[i]);
        return n;
    }

    /** Logical shift towards lower indices: {@code dest = src >>> shift}. Alias-safe. */
    public static void shiftRight(long[] src, int shift, long[] dest) {
        int n = dest.length;
        int wordShift = shift >>> 6;
        int bitShift = shift & 63;
        for (int i = 0; i < n; i++) {
            long v = 0L;
            int s = i + wordShift;
            if (s < n) {
                v = src[s] >>> bitShift;
                if (bitShift != 0 && s + 1 < n) v |= src[s + 1] << (64 - bitShift);
            }
            dest[i] = v;
        }
    }

    /** Shift towards higher indices: {@code dest = src << shift}. Alias-safe. */
    public static void shiftLeft(long[] src, int shift, long[] dest) {
        int n = dest.length;
        int wordShift = shift >>> 6;
        int bitShift = shift & 63;
        for (int i = n - 1; i >= 0; i--) {
            long v = 0L;
            int s = i - wordShift;
            if (s >= 0) {
                v = src[s] << bitShift;
                if (bitShift != 0 && s - 1 >= 0) v |= src[s - 1] >>> (64 - bitShift);
            }
            dest[i] = v;
        }
    }

    /** {@code amount > 0} shifts left, {@code amount < 0} shifts right. */
    public static void signedShift(long[] src, int amount, long[] dest) {
        if (amount == 0) {
            if (src != dest) copy(src, dest);
        } else if (amount > 0) {
            shiftLeft(src, amount, dest);
        } else {
            shiftRight(src, -amount, dest);
        }
    }

    public static long signedShift(long value, int amount) {
        if (amount == 0) return value;
        if (amount >= 64 || amount <= -64) return 0L;
        return amount > 0 ? value << amount : value >>> -amount;
    }

    /** Clear bits at indices {@code >= totalBits} (in the last word). */
    public static void maskExcess(long[] a, int totalBits) {
        int rem = totalBits & 63;
        if (rem != 0 && a.length > 0) {
            a[a.length - 1] &= (1L << rem) - 1L;
        }
    }

    /** Unsigned lexicographic comparison from the most significant word down. */
    public static int compareUnsigned(long[] a, long[] b) {
        for (int i = a.length - 1; i >= 0; i--) {
            int c = Long.compareUnsigned(a[i], b[i]);
            if (c != 0) return c;
        }
        return 0;
    }
}
