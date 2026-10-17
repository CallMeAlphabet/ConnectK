package connectk.core.theory;

import connectk.core.model.Direction;
import connectk.core.model.Move;
import connectk.core.model.Outcome;
import connectk.core.model.Placement;
import connectk.core.model.Player;
import connectk.core.model.Position;
import connectk.core.model.Rules;

import java.util.HashMap;
import java.util.Map;
import java.util.Optional;
import java.util.OptionalInt;

/**
 * Published mathematical results, encoded as data. Currently this covers
 * the classical gravity game (k=4, all directions, normal rules):
 *
 * <ul>
 *   <li>the game-theoretic value of the empty board for a range of board
 *       sizes (John Tromp, solved 2006 for width+height &lt;= 15 and 8x8 in 2015;
 *       extended by Christophe Steininger to width+height = 16 and beyond),</li>
 *   <li>the exact first-move values of the standard 7x6 game
 *       (James D. Allen / Victor Allis, 1988; exact scores from modern strong solvers).</li>
 * </ul>
 *
 * Anything not in this database is <em>not</em> claimed as proven here —
 * the solver will either prove it by search or label its answer heuristic.
 * See docs/THEORY.md for references.
 */
public final class TheoryDatabase {

    private TheoryDatabase() {
    }

    private record Entry(Outcome outcomeForFirstPlayer, int lastMovePly) {
    }

    private static final String SOURCE_TABLE =
            "Published board-size table (Tromp 2006/2015; Steininger extension) - see docs/THEORY.md";
    private static final String SOURCE_STANDARD =
            "Standard 7x6 solution (James D. Allen / Victor Allis 1988; exact scores verified by modern strong solvers)";

    private static final Map<Long, Entry> EMPTY_BOARD = new HashMap<>();

    private static long key(int width, int height) {
        return ((long) width << 32) | (height & 0xffffffffL);
    }

    private static void put(int w, int h, Outcome o, int lastMove) {
        EMPTY_BOARD.put(key(w, h), new Entry(o, lastMove));
    }

    private static void put(int w, int h, Outcome o) {
        put(w, h, o, -1);
    }

    static {
        // height 4
        put(4, 4, Outcome.DRAW, 16); put(5, 4, Outcome.DRAW, 20); put(6, 4, Outcome.LOSS, 24);
        put(7, 4, Outcome.DRAW, 28); put(8, 4, Outcome.LOSS, 32); put(9, 4, Outcome.LOSS, 36);
        put(10, 4, Outcome.LOSS, 40); put(11, 4, Outcome.LOSS, 44); put(12, 4, Outcome.LOSS, 48);
        // height 5
        put(4, 5, Outcome.DRAW, 20); put(5, 5, Outcome.DRAW, 25); put(6, 5, Outcome.DRAW, 30);
        put(7, 5, Outcome.DRAW, 35); put(8, 5, Outcome.WIN, 39); put(9, 5, Outcome.WIN, 41);
        put(10, 5, Outcome.WIN, 47); put(11, 5, Outcome.WIN, 51);
        // height 6
        put(4, 6, Outcome.DRAW, 24); put(5, 6, Outcome.DRAW, 30); put(6, 6, Outcome.LOSS, 36);
        put(7, 6, Outcome.WIN, 41); put(8, 6, Outcome.LOSS, 48); put(9, 6, Outcome.LOSS, 52);
        put(10, 6, Outcome.WIN, 59);
        // height 7
        put(4, 7, Outcome.DRAW, 28); put(5, 7, Outcome.DRAW, 35); put(6, 7, Outcome.WIN, 41);
        put(7, 7, Outcome.DRAW, 49); put(8, 7, Outcome.WIN, 53); put(9, 7, Outcome.WIN, 55);
        // height 8
        put(4, 8, Outcome.DRAW, 32); put(5, 8, Outcome.DRAW, 40); put(6, 8, Outcome.LOSS, 48);
        put(7, 8, Outcome.WIN, 55); put(8, 8, Outcome.LOSS, 62);
        // height 9
        put(4, 9, Outcome.DRAW, 36); put(5, 9, Outcome.DRAW, 45); put(6, 9, Outcome.WIN, 53);
        put(7, 9, Outcome.DRAW, 63); put(8, 9, Outcome.WIN);
        // height 10
        put(4, 10, Outcome.DRAW, 40); put(5, 10, Outcome.DRAW, 50); put(6, 10, Outcome.LOSS, 60);
        put(7, 10, Outcome.WIN, 69);
        // height 11
        put(4, 11, Outcome.DRAW, 44); put(5, 11, Outcome.DRAW, 55); put(6, 11, Outcome.WIN, 63);
        // height 12
        put(4, 12, Outcome.DRAW, 48); put(5, 12, Outcome.DRAW, 60); put(6, 12, Outcome.LOSS, 72);
        // height 13
        put(4, 13, Outcome.DRAW, 52); put(5, 13, Outcome.DRAW, 65);
    }

    private static boolean applies(Rules r) {
        return r.placement() == Placement.GRAVITY
                && r.connect() == 4
                && !r.misere()
                && r.directions().containsAll(java.util.EnumSet.allOf(Direction.class));
    }

    public static Optional<TheoryResult> lookup(Position p) {
        Rules r = p.rules();
        if (!applies(r) || p.isTerminal()) return Optional.empty();

        if (p.movesPlayed() == 0) {
            Entry e = EMPTY_BOARD.get(key(r.width(), r.height()));
            if (e == null) return Optional.empty();
            Optional<Move> best = Optional.empty();
            if (r.width() == 7 && r.height() == 6) {
                best = Optional.of(Move.cell(3, 0)); // the unique winning opening
            }
            return Optional.of(new TheoryResult(
                    e.outcomeForFirstPlayer(),
                    e.lastMovePly() > 0 ? OptionalInt.of(e.lastMovePly()) : OptionalInt.empty(),
                    best,
                    r.width() == 7 && r.height() == 6 ? SOURCE_STANDARD : SOURCE_TABLE));
        }

        // Standard 7x6 after exactly one move: exact values are published.
        if (p.movesPlayed() == 1 && r.width() == 7 && r.height() == 6) {
            int firstColumn = -1;
            for (int c = 0; c < 7; c++) {
                if (p.ownerAt(c, 0).orElse(null) == Player.ONE) {
                    firstColumn = c;
                    break;
                }
            }
            if (firstColumn < 0) return Optional.empty();
            // Outcome for the side to move (player two) and the ply the game ends on.
            Outcome outcome;
            int endPly;
            switch (firstColumn) {
                case 3 -> { outcome = Outcome.LOSS; endPly = 41; }   // centre: first player wins
                case 2, 4 -> { outcome = Outcome.DRAW; endPly = 42; }
                case 1, 5 -> { outcome = Outcome.WIN; endPly = 42; } // second player wins
                default -> { outcome = Outcome.WIN; endPly = 40; }   // columns 1 and 7 (0-based 0,6)
            }
            return Optional.of(new TheoryResult(outcome, OptionalInt.of(endPly - 1), Optional.empty(), SOURCE_STANDARD));
        }

        return Optional.empty();
    }
}
