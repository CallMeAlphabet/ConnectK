package connectk.core.theory;

import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;

/**
 * A generated opening book: exact scores (and a proven best column) for
 * a set of one-word-bitboard positions, keyed by mirror-canonical
 * {@code (current, mask)} bitboards — the same canonical form the fast
 * solver uses for its transposition table.
 *
 * <p>Books are produced offline by the CLI {@code bookgen} command
 * (which solves frontier positions with the exact solver, in parallel,
 * checkpointing each result) and loaded at runtime. Lookups are
 * binary searches over sorted primitive arrays: no allocation, no
 * hashing, exact key verification (no collision risk to proofs).</p>
 *
 * <p>File format (text, one entry per line):</p>
 * <pre>
 * # connectk-book v1 depth=&lt;plies&gt;
 * &lt;curHex16&gt; &lt;maskHex16&gt; &lt;score&gt; &lt;canonicalBestColumnOrMinus1&gt;
 * </pre>
 *
 * <p>The score uses this project's exact-solver convention
 * ({@code ±(cells - finalPly + 1)}, 0 = draw, from the side to move).
 * The best column is stored in canonical (possibly mirrored)
 * orientation; callers that know whether their position was mirrored
 * to obtain the canonical key must un-mirror it.</p>
 */
public final class OpeningBook {

    /** Returned by {@link #lookup} when the position is not in the book. */
    public static final int MISS = Integer.MIN_VALUE;

    private final long[] curs;    // sorted by (cur, mask), unsigned lexicographic
    private final long[] masks;
    private final byte[] scores;  // exact score, fits a signed byte for one-word boards
    private final byte[] cols;    // canonical best column, or -1
    private final int depth;      // plies played for all entries, or -1 if mixed/unknown
    private final int minPly;
    private final int maxPly;

    private OpeningBook(long[] curs, long[] masks, byte[] scores, byte[] cols, int depth) {
        this.curs = curs;
        this.masks = masks;
        this.scores = scores;
        this.cols = cols;
        this.depth = depth;
        int mn = Integer.MAX_VALUE;
        int mx = -1;
        for (long m : masks) {
            int p = Long.bitCount(m);
            mn = Math.min(mn, p);
            mx = Math.max(mx, p);
        }
        this.minPly = masks.length == 0 ? 0 : mn;
        this.maxPly = masks.length == 0 ? -1 : mx;
    }

    /** Number of entries in the book. */
    public int size() {
        return curs.length;
    }

    /** Plies played shared by all entries, or -1 if mixed. */
    public int depth() {
        return depth;
    }

    /** Whether probing the book for a position with this many stones can possibly hit. */
    public boolean coversPly(int movesPlayed) {
        if (depth >= 0) return movesPlayed == depth;
        return movesPlayed >= minPly && movesPlayed <= maxPly;
    }

    /** Exact score for the canonical key, or {@link #MISS}. */
    public int lookup(long cur, long mask) {
        int i = indexOf(cur, mask);
        return i < 0 ? MISS : scores[i];
    }

    /** Canonical best column for the key, or -1 if absent / not stored. */
    public int bestCol(long cur, long mask) {
        int i = indexOf(cur, mask);
        return i < 0 ? -1 : cols[i];
    }

    private int indexOf(long cur, long mask) {
        int lo = 0;
        int hi = curs.length - 1;
        while (lo <= hi) {
            int mid = (lo + hi) >>> 1;
            int cmp = Long.compareUnsigned(cur, curs[mid]);
            if (cmp == 0) cmp = Long.compareUnsigned(mask, masks[mid]);
            if (cmp < 0) hi = mid - 1;
            else if (cmp > 0) lo = mid + 1;
            else return mid;
        }
        return -1;
    }

    /** One book line in the text format described above. */
    public static String formatEntry(long cur, long mask, int score, int canonicalBestCol) {
        return String.format("%016x %016x %d %d", cur, mask, score, canonicalBestCol);
    }

    public static OpeningBook load(Path file) throws IOException {
        LongList curList = new LongList();
        LongList maskList = new LongList();
        ByteList scoreList = new ByteList();
        ByteList colList = new ByteList();
        int depth = -1;
        try (BufferedReader in = Files.newBufferedReader(file)) {
            String line;
            while ((line = in.readLine()) != null) {
                line = line.trim();
                if (line.isEmpty()) continue;
                if (line.startsWith("#")) {
                    int d = line.indexOf("depth=");
                    if (d >= 0) {
                        try {
                            depth = Integer.parseInt(line.substring(d + 6).trim().split("\\s+")[0]);
                        } catch (NumberFormatException ignored) {
                            // keep -1
                        }
                    }
                    continue;
                }
                String[] p = line.split("\\s+");
                if (p.length < 3) continue;
                long cur = Long.parseUnsignedLong(p[0], 16);
                long mask = Long.parseUnsignedLong(p[1], 16);
                int score = Integer.parseInt(p[2]);
                int col = p.length > 3 ? Integer.parseInt(p[3]) : -1;
                curList.add(cur);
                maskList.add(mask);
                scoreList.add((byte) score);
                colList.add((byte) col);
            }
        }
        int n = curList.size;
        long[] curs = curList.toArray();
        long[] masks = maskList.toArray();
        byte[] scores = scoreList.toArray();
        byte[] cols = colList.toArray();
        Integer[] order = new Integer[n];
        for (int i = 0; i < n; i++) order[i] = i;
        Arrays.sort(order, (a, b) -> {
            int c = Long.compareUnsigned(curs[a], curs[b]);
            return c != 0 ? c : Long.compareUnsigned(masks[a], masks[b]);
        });
        long[] sCurs = new long[n];
        long[] sMasks = new long[n];
        byte[] sScores = new byte[n];
        byte[] sCols = new byte[n];
        for (int i = 0; i < n; i++) {
            sCurs[i] = curs[order[i]];
            sMasks[i] = masks[order[i]];
            sScores[i] = scores[order[i]];
            sCols[i] = cols[order[i]];
        }
        if (depth < 0 && n > 0) {
            int p0 = Long.bitCount(sMasks[0]);
            boolean uniform = true;
            for (long m : sMasks) {
                if (Long.bitCount(m) != p0) {
                    uniform = false;
                    break;
                }
            }
            if (uniform) depth = p0;
        }
        return new OpeningBook(sCurs, sMasks, sScores, sCols, depth);
    }

    /** Write a book header + the given raw entry lines (already formatted) to a fresh file. */
    public static void writeHeader(BufferedWriter out, int depth) throws IOException {
        out.write("# connectk-book v1 depth=" + depth);
        out.newLine();
    }

    private static final class LongList {
        long[] a = new long[1024];
        int size;

        void add(long v) {
            if (size == a.length) a = Arrays.copyOf(a, a.length * 2);
            a[size++] = v;
        }

        long[] toArray() {
            return Arrays.copyOf(a, size);
        }
    }

    private static final class ByteList {
        byte[] a = new byte[1024];
        int size;

        void add(byte v) {
            if (size == a.length) a = Arrays.copyOf(a, a.length * 2);
            a[size++] = v;
        }

        byte[] toArray() {
            return Arrays.copyOf(a, size);
        }
    }
}
