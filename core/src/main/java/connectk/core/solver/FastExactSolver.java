package connectk.core.solver;

import connectk.core.model.Direction;
import connectk.core.model.Move;
import connectk.core.model.Outcome;
import connectk.core.model.Placement;
import connectk.core.model.Player;
import connectk.core.model.Position;
import connectk.core.model.Rules;
import connectk.core.theory.OpeningBook;

import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import java.util.OptionalInt;

/**
 * High-performance exact (proof) solver for gravity positions whose
 * bitboard fits in a single {@code long} — the fast path behind
 * {@link ConnectKSolver}. Same score convention and result semantics
 * as {@link ExactSolver}, but the search is a PVS negamax core
 * (dual-bound transposition table with
 * mirror-canonical keys, TT-move + threat-based ordering, tactical
 * shortcuts, direct score-bound tightening, optional generated
 * opening book), with a primitive, allocation-free search loop:
 *
 * <ul>
 *   <li>state is two {@code long}s plus an {@code int[]} of column
 *       heights; play/undo are a handful of arithmetic ops;</li>
 *   <li>line and threat tests are shift/AND sequences on one word;</li>
 *   <li>the transposition table is an open-addressing primitive table
 *       (no {@code HashMap}, no key objects, no boxing), keyed by the
 *       full (current, mask) pair so entries are collision-safe;</li>
 *   <li>move lists live in preallocated per-ply {@code int[]} scratch
 *       arrays.</li>
 * </ul>
 *
 * <p>Supported rules: gravity placement, non-misère, one-word bitboard
 * ({@code width * (height+1) <= 64}), any k and any direction subset.
 * Everything else falls back to {@link ExactSolver}. Instances are
 * single-use and not thread-safe; create one per solve.</p>
 *
 * <p>The techniques are the standard ones from the Connect-4 solver
 * literature (Allis 1988; Tromp's Fhourstones; Pons's solver tutorial);
 * this implementation was written independently for this project.</p>
 */
public final class FastExactSolver {

    // Dual-bound TT entries: every entry carries both a lower and an
    // upper bound on the exact score (equal = exact), so null-window
    // probes and full-window re-searches accumulate information instead
    // of overwriting each other's bounds. Scores fit in a signed byte
    // (one-word boards have |score| <= 66); sentinels mean "no bound".
    private static final int NO_LOWER = -128;
    private static final int NO_UPPER = 127;

    private static final int DEFAULT_TT_CAPACITY = 1 << 21; // ~2M entries, ~42 MB
    private static final int MAX_PROBES = 64;

    // Transposition table (primitive, open addressing). Allocated lazily
    // on first use and sized to the position, so small endgame solves do
    // not pay for zeroing tens of MB; reused (and cleared) across solves
    // on the same instance.
    private final int maxTtCapacity;
    private long[] ttCur;
    private long[] ttMask;
    private int[] ttData;   // packed: lower(8) | upper(8) | bestCol+1(7)
    private byte[] ttUsed;
    private int ttIndexMask;

    // Rules / derived constants.
    private Rules rules;
    private int width;
    private int height;
    private int connect;
    private int cells;
    private int stride;
    private int infinity;
    private int[] shifts;
    private long boardMask;
    private long pairLowerMask; // cells in 0-based even rows (lower cell of each vertical pair)
    private long pairUpperMask; // cells in 0-based odd rows (upper cell of each pair, "even rows")
    private boolean claimEvenApplicable;

    /** Kill switch for A/B benchmarking: -Dconnectk.fast.claimEven=false disables the bound. */
    private static final boolean CLAIM_EVEN_ENABLED =
            !"false".equalsIgnoreCase(System.getProperty("connectk.fast.claimEven", "true"));
    private boolean mirrorSymmetric;

    // Mutable search state.
    private long cur;   // stones of the side to move
    private long mask;  // all stones
    private int[] heights;
    private int moves;

    // Per-ply move-ordering scratch (indexed by moves played).
    private int[][] moveCols;
    private int[][] moveScores;
    private int prepCount;
    private boolean prepForced;
    private int prepForcedScore;
    private int prepForcedCol;
    private boolean prepMinPossibleValid;

    private int rootBestCol;
    private boolean rootMoveProven;
    private int prepTtCol = -1;

    private OpeningBook book;

    private long nodes;
    private long nodeLimit;
    private long startNanos;
    private long deadlineNanos;
    private CancellationToken token = CancellationToken.NONE;
    private SearchListener listener = SearchListener.NONE;

    public FastExactSolver() {
        this(DEFAULT_TT_CAPACITY);
    }

    /** @param maxTtCapacity maximum transposition table entries; must be a power of two. */
    public FastExactSolver(int maxTtCapacity) {
        if (maxTtCapacity <= 0 || (maxTtCapacity & (maxTtCapacity - 1)) != 0) {
            throw new IllegalArgumentException("maxTtCapacity must be a positive power of two");
        }
        this.maxTtCapacity = maxTtCapacity;
    }

    /** Canonical one-word boards for a position: side-to-move stones, all stones, mirroring applied. */
    public record CanonicalBoards(long cur, long mask, boolean flipped) {
    }

    /**
     * The mirror-canonical {@code (cur, mask)} key this solver uses
     * internally (and opening books are keyed by), for a position whose
     * rules {@link #supports(Rules)}.
     */
    public static CanonicalBoards encodeCanonical(Position position) {
        Rules rules = position.rules();
        Player side = position.sideToMove();
        long cur = position.stoneBits(side)[0];
        long mask = cur | position.stoneBits(side.other())[0];
        if (!rules.mirrorSymmetric()) return new CanonicalBoards(cur, mask, false);
        int width = rules.width();
        int stride = rules.height() + 1;
        long mMask = mirrorBits(mask, width, stride);
        int cmp = Long.compareUnsigned(mMask, mask);
        if (cmp < 0) return new CanonicalBoards(mirrorBits(cur, width, stride), mMask, true);
        if (cmp == 0) {
            long mCur = mirrorBits(cur, width, stride);
            if (Long.compareUnsigned(mCur, cur) < 0) return new CanonicalBoards(mCur, mMask, true);
        }
        return new CanonicalBoards(cur, mask, false);
    }

    private static long mirrorBits(long bits, int width, int stride) {
        long colMask = stride >= 64 ? -1L : (1L << stride) - 1L;
        long out = 0L;
        for (int c = 0; c < width; c++) {
            long chunk = (bits >>> (c * stride)) & colMask;
            out |= chunk << ((width - 1 - c) * stride);
        }
        return out;
    }

    /**
     * Attach a generated opening book. Positions in the book are answered
     * exactly without search (the book was itself produced by exact
     * search); everything else is searched as usual.
     */
    public void setOpeningBook(OpeningBook book) {
        this.book = book;
    }

    /** Whether this fast path can handle the given rules. */
    public static boolean supports(Rules rules) {
        return rules.placement() == Placement.GRAVITY
                && !rules.misere()
                && rules.wordCount() == 1;
    }

    public SearchResult solve(Position position, long nodeLimit, long timeLimitMillis,
                              CancellationToken token, SearchListener listener) {
        if (!supports(position.rules())) {
            throw new IllegalArgumentException("FastExactSolver does not support rules: " + position.rules());
        }
        initState(position, nodeLimit, timeLimitMillis, token, listener);

        if (position.isTerminal()) {
            int score = terminalScore(position);
            return new SearchResult(SearchResult.Certainty.PROVEN_EXACT,
                    Optional.of(Outcome.fromScoreSign(score)), Optional.empty(), true,
                    score, OptionalInt.of(0), 0, 0, 0, List.of(),
                    "Position is already terminal.");
        }

        try {
            int score = searchRoot();
            long elapsed = elapsedMillis();
            Optional<Move> bestMove = rootBestCol >= 0
                    ? Optional.of(Move.cell(rootBestCol, position.columnCount(rootBestCol)))
                    : Optional.empty();
            List<Move> pv = reconstructVariation();
            int plies;
            if (score == 0) {
                plies = cells - position.movesPlayed();
            } else {
                int terminalPly = cells - Math.abs(score) + 1;
                plies = terminalPly - position.movesPlayed();
            }
            return new SearchResult(SearchResult.Certainty.PROVEN_EXACT,
                    Optional.of(Outcome.fromScoreSign(score)), bestMove, rootMoveProven,
                    score, OptionalInt.of(plies), plies, nodes, elapsed, pv,
                    "Proved by exhaustive principal-variation alpha-beta search (fast one-word gravity "
                            + "solver: primitive bitboard state, open-addressing transposition "
                            + "table, threat ordering" + (book != null ? ", generated opening book" : "") + ")."
                            + (rootMoveProven ? "" : " The score is proven; identifying an optimal "
                            + "move ran out of budget, so the suggested move is not certified."));
        } catch (SearchAbortedException e) {
            return SearchResult.unknown(nodes, elapsedMillis(),
                    "Exact proof aborted (" + e.reason() + ") after " + nodes + " nodes; no conclusion claimed.");
        }
    }

    public long nodesExplored() {
        return nodes;
    }

    // ------------------------------------------------------------------
    // Search
    // ------------------------------------------------------------------


    /** Outcome of a bounded (window) search: fail-soft value, abort flag, node count. */
    public record BoundResult(int value, boolean aborted, long nodes) {
    }

    /**
     * Fail-soft bounded search of a position with window
     * {@code (alpha, beta)} — typically a null window
     * ({@code beta = alpha + 1}) to test "is the exact score at least
     * beta?". Returns a value {@code <= alpha} (proven upper bound),
     * {@code >= beta} (proven lower bound), or the exact score in
     * between; {@code aborted} is set instead of a meaningful value when
     * the budget runs out. This is the building block
     * {@link ParallelExactSolver} uses for parallel refutations, which
     * are typically far cheaper than exact proofs.
     */
    public BoundResult searchBound(Position position, int alpha, int beta,
                                   long nodeLimit, long timeLimitMillis,
                                   CancellationToken token) {
        if (!supports(position.rules())) {
            throw new IllegalArgumentException("FastExactSolver does not support rules: " + position.rules());
        }
        if (alpha >= beta) throw new IllegalArgumentException("need alpha < beta");
        initState(position, nodeLimit, timeLimitMillis, token, SearchListener.NONE);
        if (position.isTerminal()) {
            return new BoundResult(terminalScore(position), false, 0);
        }
        try {
            return new BoundResult(negamax(alpha, beta), false, nodes);
        } catch (SearchAbortedException aborted) {
            return new BoundResult(0, true, nodes);
        }
    }

    private void initState(Position position, long nodeLimit, long timeLimitMillis,
                           CancellationToken token, SearchListener listener) {
        this.rules = position.rules();
        this.width = rules.width();
        this.height = rules.height();
        this.connect = rules.connect();
        this.cells = rules.cells();
        this.stride = rules.stride();
        this.infinity = cells + 2;
        this.boardMask = rules.boardMask()[0];
        this.pairLowerMask = 0L;
        this.pairUpperMask = 0L;
        for (int c = 0; c < width; c++) {
            for (int r = 0; r < height; r++) {
                if ((r & 1) == 0) pairLowerMask |= 1L << (c * stride + r);
                else pairUpperMask |= 1L << (c * stride + r);
            }
        }
        this.claimEvenApplicable = CLAIM_EVEN_ENABLED && height % 2 == 0 && height >= 2;
        this.mirrorSymmetric = rules.mirrorSymmetric();
        Direction[] dirs = rules.directions().toArray(new Direction[0]);
        this.shifts = new int[dirs.length];
        for (int i = 0; i < dirs.length; i++) shifts[i] = dirs[i].shift(stride);

        Player side = position.sideToMove();
        this.cur = position.stoneBits(side)[0];
        this.mask = cur | position.stoneBits(side.other())[0];
        this.heights = new int[width];
        for (int c = 0; c < width; c++) heights[c] = position.columnCount(c);
        this.moves = position.movesPlayed();
        initTable(cells - moves);
        this.moveCols = new int[cells + 1][Math.max(1, width)];
        this.moveScores = new int[cells + 1][Math.max(1, width)];
        this.rootBestCol = -1;
        this.rootMoveProven = true;

        this.nodes = 0;
        this.nodeLimit = nodeLimit;
        this.token = token == null ? CancellationToken.NONE : token;
        this.listener = listener == null ? SearchListener.NONE : listener;
        this.startNanos = System.nanoTime();
        this.deadlineNanos = timeLimitMillis > 0
                ? startNanos + timeLimitMillis * 1_000_000L
                : Long.MAX_VALUE;
    }

    private int searchRoot() {
        nodes++;
        checkBudget();
        CanonicalKey key = canonicalKey();
        int slot = slotFor(key.cur, key.mask);
        prepTtCol = -1;
        prepare();
        if (prepForced) {
            rootBestCol = prepForcedCol;
            storeExact(slot, key, prepForcedScore, prepForcedCol);
            return prepForcedScore;
        }
        if (book != null && book.coversPly(moves)) {
            int bookScore = book.lookup(key.cur, key.mask);
            if (bookScore != OpeningBook.MISS) {
                int bc = book.bestCol(key.cur, key.mask);
                rootBestCol = bc >= 0
                        ? (key.flipped ? width - 1 - bc : bc)
                        : findAchievingMove(bookScore, key);
                storeExact(slot, key, bookScore, rootBestCol);
                return bookScore;
            }
        }
        // Principal-variation search at the root: first move full window,
        // later moves null-window probed then re-searched if they beat
        // alpha. (An MTD(f) driver was tried and measured 17-31% MORE
        // nodes on opening sets: with PVS already probing null windows
        // per move, MTD's repeated root passes are pure overhead for
        // strong solving. See docs/PERFORMANCE.md.)
        int level = moves;
        int n = prepCount;
        int alpha = -infinity;
        int best = -infinity;
        rootBestCol = -1;
        for (int i = 0; i < n; i++) {
            int col = moveCols[level][i];
            play(col);
            int score;
            if (i == 0) {
                score = -negamax(-infinity, -alpha);
            } else {
                score = -negamax(-alpha - 1, -alpha);
                if (score > alpha) score = -negamax(-infinity, -alpha);
            }
            undo(col);
            if (score > best) {
                best = score;
                rootBestCol = col;
            }
            if (best > alpha) alpha = best;
        }
        storeExact(slot, key, best, rootBestCol);
        return best;
    }

    /**
     * Find a root move that achieves the proven score. Tries the
     * transposition-table move first, then the ordered candidates, each
     * with a null-window test on the child ("is the child value at most
     * -score?"). If the budget dies during verification, falls back to
     * the best unverified candidate and marks the move unproven (the
     * score itself stays proven).
     */
    private int findAchievingMove(int score, CanonicalKey key) {
        prepTtCol = -1;
        prepare();
        if (prepForced) return prepForcedCol;
        int level = moves;
        int n = prepCount;
        // Candidate order: TT move (if any) first, then prepare()'s order.
        int ttCol = -1;
        int slot = slotFor(key.cur, key.mask);
        if (ttUsed[slot] != 0 && ttCur[slot] == key.cur && ttMask[slot] == key.mask) {
            int best = ttBestCol(ttData[slot]);
            if (best >= 0) ttCol = key.flipped ? width - 1 - best : best;
        }
        try {
            if (ttCol >= 0 && heights[ttCol] < height && achieves(ttCol, score)) return ttCol;
            for (int i = 0; i < n; i++) {
                int col = moveCols[level][i];
                if (col == ttCol) continue;
                if (achieves(col, score)) return col;
            }
            // Should not happen (some move must achieve the maximum);
            // fall back to a full-window scan as a safety net.
            int bestCol = moveCols[level][0];
            int bestVal = -infinity;
            for (int i = 0; i < n; i++) {
                int col = moveCols[level][i];
                play(col);
                int v;
                try {
                    v = -negamax(-infinity, infinity);
                } finally {
                    undo(col);
                }
                if (v > bestVal) {
                    bestVal = v;
                    bestCol = col;
                }
            }
            rootMoveProven = bestVal == score;
            return bestCol;
        } catch (SearchAbortedException aborted) {
            rootMoveProven = false;
            return ttCol >= 0 && heights[ttCol] < height ? ttCol : moveCols[level][0];
        }
    }

    /** Null-window test: does playing {@code col} achieve at least the proven root score? */
    private boolean achieves(int col, int score) {
        play(col);
        int v;
        try {
            v = negamax(-score, -score + 1);
        } finally {
            undo(col);
        }
        return v <= -score;
    }

    private int negamax(int alpha, int beta) {
        nodes++;
        checkBudget();
        // Only the player who just moved can have completed a line.
        if (moves > 0 && hasLine(mask ^ cur)) return -(cells - moves + 1);
        if (moves == cells) return 0;

        int alphaOrig = alpha;
        int betaOrig = beta;
        CanonicalKey key = canonicalKey();
        if (book != null && book.coversPly(moves)) {
            int bookScore = book.lookup(key.cur, key.mask);
            if (bookScore != OpeningBook.MISS) return bookScore;
        }
        prepTtCol = -1;
        int slot = slotFor(key.cur, key.mask);
        if (ttUsed[slot] != 0 && ttCur[slot] == key.cur && ttMask[slot] == key.mask) {
            int data = ttData[slot];
            int lower = ttLower(data);
            int upper = ttUpper(data);
            int ttBest = ttBestCol(data);
            if (ttBest >= 0) prepTtCol = key.flipped ? width - 1 - ttBest : ttBest;
            if (lower >= beta) return lower;
            if (upper <= alpha) return upper;
            alpha = Math.max(alpha, lower);
            beta = Math.min(beta, upper);
            if (alpha >= beta) return alpha; // bounds met: exact
            alphaOrig = alpha;
            betaOrig = beta;
        }

        prepare();
        if (prepForced) {
            storeExact(slot, key, prepForcedScore, prepForcedCol);
            return prepForcedScore;
        }
        // Score-bound tightening: prepare() has established that the side
        // to move has no immediate win, so its earliest win is in 3 plies;
        // and (when prepMinPossibleValid) the opponent's earliest win is
        // in 4 plies. Clamp the window with these direct bounds.
        int empties = cells - moves;
        int maxPossible = empties >= 3 ? cells - moves - 2 : 0;
        if (maxPossible < beta) {
            beta = maxPossible;
            if (alpha >= beta) {
                storeBounds(slot, key, NO_LOWER, maxPossible, -1);
                return maxPossible;
            }
            betaOrig = beta;
        }
        boolean minValid = prepMinPossibleValid;
        int minPossible = 0;
        if (minValid) {
            minPossible = empties >= 4 ? -(cells - moves - 3) : 0;
            if (minPossible > alpha) {
                alpha = minPossible;
                if (alpha >= beta) {
                    storeBounds(slot, key, minPossible, NO_UPPER, -1);
                    return minPossible;
                }
                alphaOrig = alpha;
            }
        }
        // Claim-even bound: if the opponent can claim all pair-upper cells
        // (see claimEvenBound()), the side to move's score is capped.
        if (claimEvenApplicable && beta > -height) {
            int ceBound = claimEvenBound();
            if (ceBound < beta) {
                beta = ceBound;
                if (alpha >= beta) {
                    storeBounds(slot, key, NO_LOWER, ceBound, -1);
                    return ceBound;
                }
                betaOrig = beta;
            }
        }
        int level = moves;
        int n = prepCount;
        int best = -infinity;
        int bestCol = -1;
        for (int i = 0; i < n; i++) {
            int col = moveCols[level][i];
            play(col);
            int score;
            if (i == 0) {
                score = -negamax(-beta, -alpha);
            } else {
                // Principal variation search: null-window probe first,
                // full re-search only if the move might raise alpha.
                score = -negamax(-alpha - 1, -alpha);
                if (score > alpha && score < beta) {
                    score = -negamax(-beta, -alpha);
                }
            }
            undo(col);
            if (score > best) {
                best = score;
                bestCol = col;
            }
            if (best > alpha) alpha = best;
            if (alpha >= beta) break;
        }
        if (best > maxPossible) best = maxPossible;
        if (minValid && best < minPossible) best = minPossible;
        if (best <= alphaOrig) storeBounds(slot, key, NO_LOWER, best, bestCol);
        else if (best >= betaOrig) storeBounds(slot, key, best, NO_UPPER, bestCol);
        else storeExact(slot, key, best, bestCol);
        return best;
    }

    /**
     * Tactical shortcuts + move ordering, mirroring {@link ExactSolver}:
     * immediate win, double threat (exact loss in 2), single threat
     * (only the block is searched). Results are left in the prep fields
     * and the per-ply scratch arrays.
     */
    private void prepare() {
        prepForced = false;
        prepMinPossibleValid = false;
        long playable = playableMask();
        if ((winningSpots(cur) & playable) != 0) {
            prepForced = true;
            prepForcedScore = cells - moves; // win on the next ply
            prepForcedCol = columnOf(winningSpots(cur) & playable);
            return;
        }
        long oppSpots = winningSpots(mask ^ cur);
        long oppPlayable = oppSpots & playable;
        int oppCount = Long.bitCount(oppPlayable);
        if (oppCount >= 2) {
            prepForced = true;
            prepForcedScore = -(cells - moves - 1); // cannot block both: loss in 2 plies
            prepForcedCol = columnOf(playable);
            return;
        }
        int level = moves;
        if (oppCount == 1) {
            int blockCol = columnOf(oppPlayable);
            int blockIdx = blockCol * stride + heights[blockCol];
            if (blockIdx + 1 < 64 && ((oppSpots >>> (blockIdx + 1)) & 1L) != 0) {
                // Second threat stacked directly above the blocking cell:
                // blocking merely makes it playable. Every move loses in 2.
                prepForced = true;
                prepForcedScore = -(cells - moves - 1);
                prepForcedCol = blockCol;
                return;
            }
            moveCols[level][0] = blockCol;
            prepCount = 1;
            prepMinPossibleValid = true; // after the block the opponent cannot win before ply +4
            return;
        }
        // No immediate opponent threat: exclude moves that play directly
        // below an opponent winning spot — they hand the opponent an
        // immediate win (loss in 2 plies) and are strictly worse than any
        // remaining move, whose earliest possible loss is in 4 plies.
        prepMinPossibleValid = true;
        int n = 0;
        int firstLegalCol = -1;
        for (int c = 0; c < width; c++) {
            if (heights[c] >= height) continue;
            if (firstLegalCol < 0) firstLegalCol = c;
            int idx = c * stride + heights[c];
            if (idx + 1 < 64 && ((oppSpots >>> (idx + 1)) & 1L) != 0) continue; // losing move
            int s = (Math.min(c, width - 1 - c) + 1) * 10;
            if (c == prepTtCol) s += 10_000_000; // transposition-table move first
            if (((oppSpots >>> idx) & 1L) != 0) s += 5_000; // occupies an opponent threat cell
            play(c);
            boolean terminalAfter = moves == cells || hasLine(mask ^ cur);
            if (!terminalAfter) {
                s += Long.bitCount(winningSpots(mask ^ cur)) * 100;
            }
            undo(c);
            moveCols[level][n] = c;
            moveScores[level][n] = s;
            n++;
        }
        if (n == 0) {
            // Every legal move enables an immediate opponent win.
            prepForced = true;
            prepForcedScore = -(cells - moves - 1);
            prepForcedCol = firstLegalCol;
            return;
        }
        // Insertion sort, best first (n <= width, tiny).
        for (int i = 1; i < n; i++) {
            int c0 = moveCols[level][i];
            int s0 = moveScores[level][i];
            int j = i - 1;
            while (j >= 0 && moveScores[level][j] < s0) {
                moveCols[level][j + 1] = moveCols[level][j];
                moveScores[level][j + 1] = moveScores[level][j];
                j--;
            }
            moveCols[level][j + 1] = c0;
            moveScores[level][j + 1] = s0;
        }
        prepCount = n;
    }

    private List<Move> reconstructVariation() {
        List<Move> pv = new ArrayList<>();
        while (moves < cells && pv.size() <= cells) {
            if (moves > 0 && hasLine(mask ^ cur)) break;
            CanonicalKey key = canonicalKey();
            int slot = slotFor(key.cur, key.mask);
            int col = -1;
            if (ttUsed[slot] != 0 && ttCur[slot] == key.cur && ttMask[slot] == key.mask) {
                int best = ttBestCol(ttData[slot]);
                if (best >= 0) col = key.flipped ? width - 1 - best : best;
            }
            if (col < 0) {
                // Entry was evicted: recover an immediate winning move if any.
                long win = winningSpots(cur) & playableMask();
                if (win == 0) break;
                col = columnOf(win);
            }
            if (col < 0 || col >= width || heights[col] >= height) break;
            pv.add(Move.cell(col, heights[col]));
            play(col);
        }
        return pv;
    }

    // ------------------------------------------------------------------
    // Bitboard primitives
    // ------------------------------------------------------------------

    private void play(int col) {
        long bit = 1L << (col * stride + heights[col]);
        cur ^= mask;
        mask |= bit;
        heights[col]++;
        moves++;
    }

    private void undo(int col) {
        heights[col]--;
        moves--;
        long bit = 1L << (col * stride + heights[col]);
        mask ^= bit;
        cur ^= mask;
    }


    /**
     * Claim-even upper bound on the exact score for the side to move
     * ("A"), or {@code infinity} if the argument does not apply.
     *
     * <p><b>The pairing argument</b> (after Victor Allis, 1988). The board has
     * even height. Partition each column's <em>remaining</em> cells into
     * pairs from the bottom of the empty segment. In a column whose
     * current stone count is even, pairs are (row 0,1), (2,3), ... in
     * 0-based rows; call the first cell of a pair its <i>lower</i> and
     * the second its <i>upper</i>. Gravity forces a pair to be started
     * by playing its lower, so the player who moves second from here
     * ("B", the opponent) can always reply directly above A's stone
     * and thereby obtain the upper of every pair A starts. If no
     * column has an odd stone count, every A move starts a pair, so B
     * obtains <b>all</b> upper cells and A only lower cells.
     *
     * <p>With exactly one odd column q, q's segment pairs are flipped
     * (B obtains q's 0-based even-row cells) and q has one unpaired
     * top cell, the "single". A could invert the roles by playing the
     * single, but only after q is completely filled \u2014 which hands B
     * all of q's claimed cells first. So if B's stones together with
     * B's claimed cells in q already contain a line, filling q
     * completes that line before the single can be played, and the
     * flipped assignment is likewise guaranteed.</p>
     *
     * <p><b>Consequences used here.</b> Let A-future = A's stones plus
     * A-assigned cells, B-future likewise. (1) A can only ever win
     * with a line inside A-future; such a line is additionally
     * unrealizable if the B-assigned cells that must be filled
     * underneath it (its gravity support) already give B a line,
     * because B completes that line while providing the support,
     * before A's line can be played. If no realizable A-line exists,
     * A's score is at most 0. (2) If B-future contains a line, B
     * completes it under claiming play (A cannot win first, by (1),
     * and the board cannot fill without B receiving its cells). Its
     * last empty cell, at 0-based row r_max, is filled after at most
     * {@code empties - (height-1-r_max)} plies in any filling order
     * (only cells above it in its column can be scheduled after it),
     * so B wins by ply {@code cells - (height-1-r_max)} and A's score
     * is at most {@code -(height - r_max)}.</p>
     *
     * <p>Soundness is not taken on faith: {@code claimEvenBoundSound}
     * in the test suite checks this bound against exact scores on
     * thousands of positions across several even-height rulesets, and
     * the benchmark harness verifies every solved score against the
     * published test sets with the bound enabled.</p>
     */
    private int claimEvenBound() {
        int oddColumns = 0;
        int oddColumn = -1;
        for (int c = 0; c < width; c++) {
            if ((heights[c] & 1) != 0) {
                oddColumns++;
                oddColumn = c;
            }
        }
        if (oddColumns > 1) return infinity;
        long opponent = mask ^ cur;
        long empty = boardMask & ~mask;
        long claimed; // empty cells the opponent (B) can guarantee
        if (oddColumns == 0) {
            claimed = empty & pairUpperMask;
        } else {
            long oddColMask = (((1L << height) - 1L) << (oddColumn * stride)) & boardMask;
            claimed = (empty & pairUpperMask & ~oddColMask) | (empty & pairLowerMask & oddColMask);
            // Guard: filling the odd column must complete a B line first.
            if (!hasLine(opponent | (claimed & oddColMask))) return infinity;
        }
        long myFuture = cur | (empty & ~claimed);
        // (1) Does A have any realizable line?
        for (int shift : shifts) {
            long ends = lineStarts(myFuture, shift);
            while (ends != 0L) {
                int start = Long.numberOfTrailingZeros(ends);
                ends &= ends - 1;
                long line = lineMask(start, shift);
                long support = belowMask(line) & claimed;
                if (!hasLine(opponent | support)) return infinity; // realizable: no bound
            }
        }
        int bound = 0;
        // (2) B lines inside B-future give negative caps by their highest empty cell.
        long oppFuture = opponent | claimed;
        for (int shift : shifts) {
            long ends = lineStarts(oppFuture, shift);
            while (ends != 0L) {
                int start = Long.numberOfTrailingZeros(ends);
                ends &= ends - 1;
                long line = lineMask(start, shift);
                long emptyLine = line & claimed;
                if (emptyLine == 0L) continue; // (already terminal; cannot occur here)
                int maxRow = 0;
                for (long e = emptyLine; e != 0L; e &= e - 1) {
                    int row = Long.numberOfTrailingZeros(e) % stride;
                    if (row > maxRow) maxRow = row;
                }
                bound = Math.min(bound, -(height - maxRow));
            }
        }
        return bound;
    }

    /** Bitmask of start cells (lowest index) of all k-lines of {@code bits} along {@code shift}. */
    private long lineStarts(long bits, int shift) {
        long x = bits;
        for (int i = 1; i < connect && x != 0L; i++) x &= x >>> shift;
        return x;
    }

    /** All cells of the k-line starting at {@code start} along {@code shift}. */
    private long lineMask(int start, int shift) {
        long line = 0L;
        for (int i = 0; i < connect; i++) line |= 1L << (start + i * shift);
        return line;
    }

    /** All board cells strictly below any cell of {@code line}, excluding the line itself. */
    private long belowMask(long line) {
        long below = 0L;
        for (long m = line; m != 0L; m &= m - 1) {
            int idx = Long.numberOfTrailingZeros(m);
            int row = idx % stride;
            if (row > 0) below |= ((1L << row) - 1L) << (idx - row);
        }
        return (below & ~line) & boardMask;
    }

    /**
     * Public exposure of the claim-even bound for validation tests:
     * an upper bound on the exact score for the side to move, or
     * {@link Integer#MAX_VALUE} when the argument does not apply
     * (unsupported rules, odd height, terminal position, or no bound).
     */
    public static int claimEvenUpperBound(Position position) {
        if (!supports(position.rules()) || position.isTerminal()) return Integer.MAX_VALUE;
        FastExactSolver solver = new FastExactSolver();
        solver.initState(position, 0, 0, CancellationToken.NONE, SearchListener.NONE);
        if (!solver.claimEvenApplicable) return Integer.MAX_VALUE;
        int bound = solver.claimEvenBound();
        return bound >= solver.infinity ? Integer.MAX_VALUE : bound;
    }

    private boolean hasLine(long bits) {
        for (int s : shifts) {
            long x = bits;
            for (int i = 1; i < connect && x != 0L; i++) x &= x >>> s;
            if (x != 0L) return true;
        }
        return false;
    }

    /** Empty cells on which the owner of {@code bits} would complete a line. */
    private long winningSpots(long bits) {
        long res = 0L;
        for (int s : shifts) {
            for (int hole = 0; hole < connect; hole++) {
                long cand = boardMask;
                for (int i = 0; i < connect; i++) {
                    if (i == hole) continue;
                    cand &= signedShift(bits, -(i - hole) * s);
                    if (cand == 0L) break;
                }
                res |= cand;
            }
        }
        return res & boardMask & ~mask;
    }

    private static long signedShift(long value, int amount) {
        if (amount == 0) return value;
        if (amount >= 64 || amount <= -64) return 0L;
        return amount > 0 ? value << amount : value >>> -amount;
    }

    private long playableMask() {
        long p = 0L;
        for (int c = 0; c < width; c++) {
            if (heights[c] < height) p |= 1L << (c * stride + heights[c]);
        }
        return p;
    }

    private int columnOf(long bits) {
        return Long.numberOfTrailingZeros(bits) / stride;
    }

    private long mirrorBits(long bits) {
        return mirrorBits(bits, width, stride);
    }

    // ------------------------------------------------------------------
    // Canonical key + transposition table
    // ------------------------------------------------------------------

    /** Size the table to the remaining game length; allocate once, then just clear the used flags. */
    private void initTable(int empties) {
        int cap;
        if (empties <= 14) cap = 1 << 14;
        else if (empties <= 18) cap = 1 << 16;
        else if (empties <= 22) cap = 1 << 18;
        else if (empties <= 26) cap = 1 << 20;
        else cap = maxTtCapacity;
        cap = Math.min(cap, maxTtCapacity);
        if (ttCur == null || ttCur.length != cap) {
            ttCur = new long[cap];
            ttMask = new long[cap];
            ttData = new int[cap];
            ttUsed = new byte[cap];
        } else {
            java.util.Arrays.fill(ttUsed, (byte) 0);
        }
        ttIndexMask = cap - 1;
    }

    private record CanonicalKey(long cur, long mask, boolean flipped) {
    }

    /** Mirror-canonical key, with the same tie-breaking as Position.canonicalKey(). */
    private CanonicalKey canonicalKey() {
        if (!mirrorSymmetric) return new CanonicalKey(cur, mask, false);
        long mMask = mirrorBits(mask);
        int cmp = Long.compareUnsigned(mMask, mask);
        if (cmp < 0) return new CanonicalKey(mirrorBits(cur), mMask, true);
        if (cmp == 0) {
            long mCur = mirrorBits(cur);
            if (Long.compareUnsigned(mCur, cur) < 0) return new CanonicalKey(mCur, mMask, true);
        }
        return new CanonicalKey(cur, mask, false);
    }

    /** Index of the matching entry, or of the slot to write (first empty / home slot). */
    private int slotFor(long kCur, long kMask) {
        long h = kCur * 0x9E3779B97F4A7C15L + kMask * 0xC2B2AE3D27D4EB4FL;
        h ^= h >>> 29;
        h *= 0xBF58476D1CE4E5B9L;
        h ^= h >>> 32;
        int home = (int) h & ttIndexMask;
        for (int probe = 0; probe < MAX_PROBES; probe++) {
            int i = (home + probe) & ttIndexMask;
            if (ttUsed[i] == 0 || (ttCur[i] == kCur && ttMask[i] == kMask)) return i;
        }
        return home; // probe budget exhausted: overwrite home slot
    }

    private static int ttLower(int data) {
        return (data << 24) >> 24;
    }

    private static int ttUpper(int data) {
        return (data << 16) >> 24;
    }

    private static int ttBestCol(int data) {
        return ((data >>> 16) & 0x7F) - 1;
    }

    private void storeExact(int slot, CanonicalKey key, int score, int bestCol) {
        storeBounds(slot, key, score, score, bestCol);
    }

    /** Merge new bounds into the slot (keeping any tighter existing bounds for the same key). */
    private void storeBounds(int slot, CanonicalKey key, int lower, int upper, int bestCol) {
        int canonicalCol = bestCol < 0 ? -1 : (key.flipped ? width - 1 - bestCol : bestCol);
        if (ttUsed[slot] != 0 && ttCur[slot] == key.cur && ttMask[slot] == key.mask) {
            int data = ttData[slot];
            lower = Math.max(lower, ttLower(data));
            upper = Math.min(upper, ttUpper(data));
            if (canonicalCol < 0) canonicalCol = ttBestCol(data);
        }
        ttCur[slot] = key.cur;
        ttMask[slot] = key.mask;
        ttData[slot] = (lower & 0xFF) | ((upper & 0xFF) << 8) | ((canonicalCol + 1) << 16);
        ttUsed[slot] = 1;
    }

    // ------------------------------------------------------------------
    // Budget / misc
    // ------------------------------------------------------------------

    private void checkBudget() {
        if (nodeLimit > 0 && nodes > nodeLimit) {
            throw new SearchAbortedException(SearchAbortedException.Reason.NODE_LIMIT);
        }
        if ((nodes & 1023L) == 0L) {
            if (token.isCancelled()) throw new SearchAbortedException(SearchAbortedException.Reason.CANCELLED);
            if (System.nanoTime() > deadlineNanos) {
                throw new SearchAbortedException(SearchAbortedException.Reason.TIME_LIMIT);
            }
        }
        if ((nodes & 65535L) == 0L) {
            listener.onProgress(nodes, -1, Optional.empty());
        }
    }

    private long elapsedMillis() {
        return (System.nanoTime() - startNanos) / 1_000_000L;
    }

    private int terminalScore(Position p) {
        Optional<Player> winner = p.winner();
        if (winner.isEmpty()) return 0;
        int magnitude = cells - p.movesPlayed() + 1;
        return winner.get() == p.sideToMove() ? magnitude : -magnitude;
    }
}
