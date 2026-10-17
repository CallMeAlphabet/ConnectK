package connectk.core.model;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Optional;

/**
 * A mutable Connect-k position on a bitboard, with fast play/undo for
 * search. Use {@link #copy()} to snapshot a position before handing it to
 * another thread.
 *
 * <h2>Representation</h2>
 * Two bitboards in the layout {@code index = column*(height+1) + row}
 * (one never-set sentinel bit per column, which stops shift-based line
 * tests from wrapping between columns):
 * <ul>
 *   <li>{@code current} – stones of the player to move;</li>
 *   <li>{@code mask} – all stones. The opponent's stones are
 *       {@code mask ^ current}.</li>
 * </ul>
 * Playing a move is {@code current ^= mask; mask |= moveBit}, which swaps
 * the perspective to the next player; undo is the exact inverse. Boards
 * larger than 64 bitboard bits transparently use several words.
 */
public final class Position {

    private final Rules rules;
    private final Direction[] directions;
    private final long[] boardMask;
    private final long[] current;
    private final long[] mask;
    private final int[] columnCounts;
    private final List<Move> history = new ArrayList<>();
    private int moves;

    private Position(Rules rules) {
        this.rules = rules;
        this.directions = rules.directions().toArray(new Direction[0]);
        this.boardMask = rules.boardMask();
        this.current = Bits.create(rules.wordCount());
        this.mask = Bits.create(rules.wordCount());
        this.columnCounts = new int[rules.width()];
        this.moves = 0;
    }

    public static Position initial(Rules rules) {
        return new Position(rules);
    }

    public static Position fromMoveSequence(Rules rules, List<Move> sequence) {
        Position p = new Position(rules);
        for (Move m : sequence) p.play(m);
        return p;
    }

    /**
     * Parse a board diagram, rows given top-to-bottom, using {@code X}
     * (player one), {@code O} (player two) and {@code .} (empty). Validates
     * stone counts, gravity support and that at most the player who moved
     * last has a completed line.
     */
    public static Position fromBoard(Rules rules, String... rowsTopToBottom) {
        if (rowsTopToBottom.length != rules.height()) {
            throw new IllegalArgumentException("expected " + rules.height() + " rows, got " + rowsTopToBottom.length);
        }
        Position p = new Position(rules);
        long[] one = Bits.create(rules.wordCount());
        long[] two = Bits.create(rules.wordCount());
        int countOne = 0;
        int countTwo = 0;
        for (int i = 0; i < rules.height(); i++) {
            String line = rowsTopToBottom[i].replace(" ", "");
            if (line.length() != rules.width()) {
                throw new IllegalArgumentException("row " + i + " has length " + line.length() + ", expected " + rules.width());
            }
            int row = rules.height() - 1 - i;
            for (int c = 0; c < rules.width(); c++) {
                char ch = line.charAt(c);
                int idx = rules.index(c, row);
                switch (ch) {
                    case 'X', 'x' -> {
                        Bits.set(one, idx);
                        Bits.set(p.mask, idx);
                        countOne++;
                        p.columnCounts[c]++;
                    }
                    case 'O', 'o' -> {
                        Bits.set(two, idx);
                        Bits.set(p.mask, idx);
                        countTwo++;
                        p.columnCounts[c]++;
                    }
                    case '.', '-' -> {
                    }
                    default -> throw new IllegalArgumentException("unexpected board character '" + ch + "'");
                }
            }
        }
        if (rules.placement() == Placement.GRAVITY) {
            for (int c = 0; c < rules.width(); c++) {
                boolean seenEmpty = false;
                for (int r = 0; r < rules.height(); r++) {
                    boolean filled = Bits.test(p.mask, rules.index(c, r));
                    if (!filled) seenEmpty = true;
                    else if (seenEmpty) throw new IllegalArgumentException("floating stone in column " + (c + 1));
                }
            }
        }
        Player side;
        if (countOne == countTwo) side = Player.ONE;
        else if (countOne == countTwo + 1) side = Player.TWO;
        else throw new IllegalArgumentException("illegal stone counts X=" + countOne + " O=" + countTwo);
        Bits.copy(side == Player.ONE ? one : two, p.current);
        p.moves = countOne + countTwo;

        boolean lineOne = p.hasLine(Player.ONE);
        boolean lineTwo = p.hasLine(Player.TWO);
        if (lineOne && lineTwo) throw new IllegalArgumentException("both players have a completed line");
        if (lineOne && countOne != countTwo + 1) throw new IllegalArgumentException("X has a line but did not move last");
        if (lineTwo && countOne != countTwo) throw new IllegalArgumentException("O has a line but did not move last");
        return p;
    }

    public Position copy() {
        Position p = new Position(rules);
        Bits.copy(current, p.current);
        Bits.copy(mask, p.mask);
        System.arraycopy(columnCounts, 0, p.columnCounts, 0, columnCounts.length);
        p.moves = moves;
        p.history.addAll(history);
        return p;
    }

    // ------------------------------------------------------------------
    // Basic state
    // ------------------------------------------------------------------

    public Rules rules() {
        return rules;
    }

    public int movesPlayed() {
        return moves;
    }

    public Player sideToMove() {
        return (moves & 1) == 0 ? Player.ONE : Player.TWO;
    }

    public boolean isFull() {
        return moves == rules.cells();
    }

    /** Number of stones in a column; under gravity this is also the next free row. */
    public int columnCount(int column) {
        return columnCounts[column];
    }

    public Optional<Player> ownerAt(int column, int row) {
        int idx = rules.index(column, row);
        if (!Bits.test(mask, idx)) return Optional.empty();
        Player side = sideToMove();
        return Optional.of(Bits.test(current, idx) ? side : side.other());
    }

    public boolean isEmpty(int column, int row) {
        return !Bits.test(mask, rules.index(column, row));
    }

    public List<Move> moveHistory() {
        return List.copyOf(history);
    }

    public Optional<Move> lastMove() {
        return history.isEmpty() ? Optional.empty() : Optional.of(history.get(history.size() - 1));
    }

    /** Stones of the given player as a fresh bitboard array. */
    public long[] stoneBits(Player player) {
        long[] out = Bits.create(rules.wordCount());
        writeStoneBits(player, out);
        return out;
    }

    private void writeStoneBits(Player player, long[] dest) {
        if (player == sideToMove()) Bits.copy(current, dest);
        else Bits.xor(mask, current, dest);
    }

    // ------------------------------------------------------------------
    // Lines, threats, terminal state
    // ------------------------------------------------------------------

    public boolean hasLine(Player player) {
        if (player == sideToMove()) return hasLineBits(current);
        long[] opp = Bits.create(rules.wordCount());
        Bits.xor(mask, current, opp);
        return hasLineBits(opp);
    }

    private boolean hasLineBits(long[] p) {
        int stride = rules.stride();
        int k = rules.connect();
        if (p.length == 1) {
            long bits = p[0];
            for (Direction d : directions) {
                int s = d.shift(stride);
                long x = bits;
                for (int i = 1; i < k && x != 0L; i++) x &= x >>> s;
                if (x != 0L) return true;
            }
            return false;
        }
        long[] x = Bits.create(p.length);
        long[] tmp = Bits.create(p.length);
        for (Direction d : directions) {
            int s = d.shift(stride);
            Bits.copy(p, x);
            for (int i = 1; i < k && Bits.any(x); i++) {
                Bits.shiftRight(x, s, tmp);
                Bits.and(x, tmp, x);
            }
            if (Bits.any(x)) return true;
        }
        return false;
    }

    /** The player who has completed a line, if any (the "line maker"). */
    public Optional<Player> lineMaker() {
        if (moves == 0) return Optional.empty();
        Player side = sideToMove();
        Player prev = side.other();
        if (hasLine(prev)) return Optional.of(prev);
        if (hasLine(side)) return Optional.of(side);
        return Optional.empty();
    }

    /**
     * The winner under the rules: the line maker normally, the other player
     * in misere rules. Empty if nobody has won (yet).
     */
    public Optional<Player> winner() {
        return lineMaker().map(maker -> rules.misere() ? maker.other() : maker);
    }

    public boolean isTerminal() {
        return isFull() || lineMaker().isPresent();
    }

    public boolean isDraw() {
        return isFull() && lineMaker().isEmpty();
    }

    /**
     * All empty cells on which {@code player} would complete a line,
     * whether currently playable or not ("winning spots" / threats).
     * Returned as a fresh bitboard array.
     */
    public long[] winningSpots(Player player) {
        int stride = rules.stride();
        int k = rules.connect();
        long[] result = Bits.create(rules.wordCount());
        long[] p = stoneBits(player);
        if (p.length == 1) {
            long bits = p[0];
            long bm = boardMask[0];
            long res = 0L;
            for (Direction d : directions) {
                int s = d.shift(stride);
                for (int hole = 0; hole < k; hole++) {
                    long cand = bm;
                    for (int i = 0; i < k; i++) {
                        if (i == hole) continue;
                        cand &= Bits.signedShift(bits, -(i - hole) * s);
                        if (cand == 0L) break;
                    }
                    res |= cand;
                }
            }
            result[0] = res & bm & ~mask[0];
            return result;
        }
        long[] cand = Bits.create(p.length);
        long[] tmp = Bits.create(p.length);
        for (Direction d : directions) {
            int s = d.shift(stride);
            for (int hole = 0; hole < k; hole++) {
                Bits.copy(boardMask, cand);
                for (int i = 0; i < k; i++) {
                    if (i == hole) continue;
                    Bits.signedShift(p, -(i - hole) * s, tmp);
                    Bits.and(cand, tmp, cand);
                    if (!Bits.any(cand)) break;
                }
                Bits.or(result, cand, result);
            }
        }
        Bits.and(result, boardMask, result);
        Bits.andNot(result, mask, result);
        return result;
    }

    public int countWinningSpots(Player player) {
        return Bits.bitCount(winningSpots(player));
    }

    private long[] legalMoveCellMask() {
        long[] cells = Bits.create(rules.wordCount());
        if (rules.placement() == Placement.GRAVITY) {
            for (int c = 0; c < rules.width(); c++) {
                int r = columnCounts[c];
                if (r < rules.height()) Bits.set(cells, rules.index(c, r));
            }
        } else {
            Bits.andNot(boardMask, mask, cells);
        }
        return cells;
    }

    /**
     * Legal moves (for whoever were to move on this board) whose target cell
     * completes a line for {@code player}. Note: making a line is a
     * <em>losing</em> move under misere rules — this method only reports
     * line completion, the interpretation is up to the caller.
     */
    public List<Move> lineCompletingMoves(Player player) {
        if (isTerminal()) return List.of();
        long[] spots = winningSpots(player);
        long[] legal = legalMoveCellMask();
        List<Move> out = new ArrayList<>();
        for (Move m : legalMoves()) {
            int idx = rules.index(m.column(), m.row());
            if (Bits.test(spots, idx) && Bits.test(legal, idx)) out.add(m);
        }
        return out;
    }

    /** Would playing {@code move} (as the side to move) complete a line? */
    public boolean completesLine(Move move) {
        Move resolved = resolve(move);
        long[] spots = winningSpots(sideToMove());
        return Bits.test(spots, rules.index(resolved.column(), resolved.row()));
    }

    /** Cells of one completed line (for display), empty if there is none. */
    public List<Move> completedLineCells() {
        Optional<Player> maker = lineMaker();
        if (maker.isEmpty()) return List.of();
        Player p = maker.get();
        int k = rules.connect();
        for (Direction d : directions) {
            for (int c = 0; c < rules.width(); c++) {
                for (int r = 0; r < rules.height(); r++) {
                    int endC = c + d.dCol() * (k - 1);
                    int endR = r + d.dRow() * (k - 1);
                    if (endC < 0 || endC >= rules.width() || endR < 0 || endR >= rules.height()) continue;
                    boolean all = true;
                    List<Move> cells = new ArrayList<>(k);
                    for (int i = 0; i < k; i++) {
                        int cc = c + d.dCol() * i;
                        int rr = r + d.dRow() * i;
                        if (ownerAt(cc, rr).orElse(null) != p) {
                            all = false;
                            break;
                        }
                        cells.add(Move.cell(cc, rr));
                    }
                    if (all) return cells;
                }
            }
        }
        return List.of();
    }

    // ------------------------------------------------------------------
    // Move generation / play / undo
    // ------------------------------------------------------------------

    public List<Move> legalMoves() {
        if (isTerminal()) return List.of();
        List<Move> out = new ArrayList<>();
        if (rules.placement() == Placement.GRAVITY) {
            for (int c = 0; c < rules.width(); c++) {
                int r = columnCounts[c];
                if (r < rules.height()) out.add(Move.cell(c, r));
            }
        } else {
            for (int c = 0; c < rules.width(); c++) {
                for (int r = 0; r < rules.height(); r++) {
                    if (isEmpty(c, r)) out.add(Move.cell(c, r));
                }
            }
        }
        return out;
    }

    public Move resolve(Move move) {
        int c = move.column();
        if (c < 0 || c >= rules.width()) throw new IllegalArgumentException("column out of range: " + (c + 1));
        if (rules.placement() == Placement.GRAVITY) {
            int r = columnCounts[c];
            if (r >= rules.height()) throw new IllegalArgumentException("column " + (c + 1) + " is full");
            if (move.row() >= 0 && move.row() != r) {
                throw new IllegalArgumentException("a stone dropped in column " + (c + 1) + " lands on row " + (r + 1));
            }
            return Move.cell(c, r);
        }
        int r = move.row();
        if (r < 0 || r >= rules.height()) throw new IllegalArgumentException("row out of range: " + (r + 1));
        if (!isEmpty(c, r)) throw new IllegalArgumentException("cell " + move.notation() + " is occupied");
        return Move.cell(c, r);
    }

    /** Play a move (resolving gravity drops). Returns the resolved move. */
    public Move play(Move move) {
        if (isTerminal()) throw new IllegalStateException("the game is already over");
        Move resolved = resolve(move);
        int idx = rules.index(resolved.column(), resolved.row());
        Bits.xor(current, mask, current); // perspective: opponent's stones
        Bits.set(mask, idx);
        columnCounts[resolved.column()]++;
        moves++;
        history.add(resolved);
        return resolved;
    }

    public Move playColumn(int column) {
        return play(Move.column(column));
    }

    public Move undo() {
        if (history.isEmpty()) throw new IllegalStateException("nothing to undo");
        Move last = history.remove(history.size() - 1);
        int idx = rules.index(last.column(), last.row());
        Bits.clear(mask, idx);
        Bits.xor(current, mask, current);
        columnCounts[last.column()]--;
        moves--;
        return last;
    }

    public void undo(Move expected) {
        if (history.isEmpty() || !history.get(history.size() - 1).equals(expected)) {
            throw new IllegalStateException("cannot undo " + expected + "; last move was " + lastMove());
        }
        undo();
    }

    // ------------------------------------------------------------------
    // Keys
    // ------------------------------------------------------------------

    public PositionKey rawKey() {
        return new PositionKey(current, mask);
    }

    /**
     * Key canonicalised under the left-right mirror symmetry (only when the
     * ruleset is mirror-symmetric; otherwise identical to {@link #rawKey()}).
     */
    public PositionKey canonicalKey() {
        if (!rules.mirrorSymmetric()) return rawKey();
        long[] mirroredMask = mirror(mask);
        int cmp = Bits.compareUnsigned(mirroredMask, mask);
        if (cmp > 0) return rawKey();
        long[] mirroredCurrent = mirror(current);
        if (cmp == 0 && Bits.compareUnsigned(mirroredCurrent, current) >= 0) return rawKey();
        return new PositionKey(mirroredCurrent, mirroredMask);
    }

    private long[] mirror(long[] bits) {
        long[] out = Bits.create(bits.length);
        int w = rules.width();
        for (int c = 0; c < w; c++) {
            int mc = w - 1 - c;
            for (int r = 0; r < rules.height(); r++) {
                if (Bits.test(bits, rules.index(c, r))) Bits.set(out, rules.index(mc, r));
            }
        }
        return out;
    }

    // ------------------------------------------------------------------
    // Misc
    // ------------------------------------------------------------------

    @Override
    public boolean equals(Object o) {
        if (this == o) return true;
        if (!(o instanceof Position other)) return false;
        return rules.equals(other.rules) && moves == other.moves
                && Arrays.equals(current, other.current) && Arrays.equals(mask, other.mask);
    }

    @Override
    public int hashCode() {
        int h = rules.hashCode();
        h = 31 * h + Arrays.hashCode(current);
        h = 31 * h + Arrays.hashCode(mask);
        return h;
    }

    /** Text diagram, top row first, with 1-based column numbers underneath. */
    @Override
    public String toString() {
        StringBuilder sb = new StringBuilder();
        for (int r = rules.height() - 1; r >= 0; r--) {
            for (int c = 0; c < rules.width(); c++) {
                if (c > 0) sb.append(' ');
                sb.append(ownerAt(c, r).map(Player::symbol).orElse('.'));
            }
            sb.append('\n');
        }
        for (int c = 0; c < rules.width(); c++) {
            if (c > 0) sb.append(' ');
            sb.append((c + 1) % 10);
        }
        return sb.toString();
    }
}
