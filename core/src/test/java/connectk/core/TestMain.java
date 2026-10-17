package connectk.core;

import connectk.core.eval.Evaluator;
import connectk.core.model.Direction;
import connectk.core.model.Move;
import connectk.core.model.Outcome;
import connectk.core.model.Placement;
import connectk.core.model.Player;
import connectk.core.model.Position;
import connectk.core.model.Rules;
import connectk.core.solver.CancellationToken;
import connectk.core.solver.ConnectKSolver;
import connectk.core.solver.DepthLimitedSolver;
import connectk.core.solver.ExactSolver;
import connectk.core.solver.FastExactSolver;
import connectk.core.solver.SearchListener;
import connectk.core.solver.SearchResult;
import connectk.core.solver.SolveRequest;
import connectk.core.theory.TheoryDatabase;
import connectk.core.theory.TheoryResult;

import java.util.ArrayList;
import java.util.EnumSet;
import java.util.List;
import java.util.Random;

/** Zero-dependency test runner (no JUnit download needed): java connectk.core.TestMain */
public final class TestMain {

    private static int passed;
    private static int failed;
    private static final List<String> failures = new ArrayList<>();

    public static void main(String[] args) {
        long start = System.currentTimeMillis();

        test("gravity play/landing/undo", TestMain::gravityPlayUndo);
        test("column full and illegal moves rejected", TestMain::illegalMoves);
        test("win detection in all four directions (gravity)", TestMain::winDirections);
        test("diagonals off: diagonal four is not a win", TestMain::diagonalsOff);
        test("free placement diagonal win", TestMain::freeWin);
        test("multi-word board (>64 bits) diagonal win", TestMain::multiWordWin);
        test("misere: making a line loses", TestMain::misereLoses);
        test("fromBoard parsing and validation", TestMain::fromBoard);
        test("mirror canonical key equality", TestMain::mirrorKey);
        test("lineCompletingMoves matches simulation on random positions", TestMain::lineCompletingMatchesSimulation);
        test("exact: win in 1 found with proven best move", TestMain::exactWinInOne);
        test("exact: K=5 win in 1", TestMain::exactWinInOneK5);
        test("exact: double threat is a proven loss in 2", TestMain::exactDoubleThreat);
        test("exact: 4x4 connect-4 empty board is a draw", TestMain::exact4x4Draw);
        test("exact: 3x3 free k=3 (tic-tac-toe) is a draw", TestMain::exactTicTacToe);
        test("exact solver returns UNKNOWN on tiny node budget", TestMain::exactBudgetUnknown);
        test("exact vs brute-force reference on random small positions", TestMain::exactVsBruteForce);
        test("fast exact vs generic exact on random gravity positions", TestMain::fastVsGeneric);
        test("fast exact: known results (5x4 draw, 6x4 loss, diagonals flip)", TestMain::fastKnownResults);
        test("depth-limited: takes immediate win", TestMain::depthTakesWin);
        test("depth-limited: blocks immediate threat", TestMain::depthBlocks);
        test("depth-limited result is labelled heuristic", TestMain::depthLabelledHeuristic);
        test("theory: standard 7x6 empty board", TestMain::theoryStandard);
        test("theory: 8x8 is a second-player win", TestMain::theory8x8);
        test("theory: 7x6 after a corner opening", TestMain::theoryAfterCorner);
        test("theory does not apply to no-diagonals rules", TestMain::theoryNotForNoDiagonals);
        test("facade AUTO: standard empty board via theory, centre move", TestMain::facadeAutoTheory);
        test("facade EXACT on tactical position proves", TestMain::facadeExact);
        test("evaluator: terminal scores and symmetry sign", TestMain::evaluatorBasics);

        long ms = System.currentTimeMillis() - start;
        System.out.println();
        System.out.printf("Tests: %d passed, %d failed (%d ms)%n", passed, failed, ms);
        if (failed > 0) {
            System.out.println("Failures:");
            failures.forEach(f -> System.out.println("  - " + f));
            System.exit(1);
        }
    }

    // ------------------------------------------------------------------
    // Tests
    // ------------------------------------------------------------------

    private static void gravityPlayUndo() {
        Position p = Position.initial(Rules.standardConnect4());
        Move m1 = p.playColumn(3);
        assertEquals(0, m1.row(), "first disc lands on row 0");
        assertEquals(Player.ONE, p.ownerAt(3, 0).orElseThrow(), "owner of first disc");
        assertEquals(Player.TWO, p.sideToMove(), "turn passes");
        Move m2 = p.playColumn(3);
        assertEquals(1, m2.row(), "second disc stacks");
        p.undo();
        assertEquals(1, p.movesPlayed(), "undo restores move count");
        assertTrue(p.isEmpty(3, 1), "undo clears the cell");
        assertEquals(Player.TWO, p.sideToMove(), "undo restores side to move");
        Position q = Position.initial(Rules.standardConnect4());
        q.playColumn(3);
        assertEquals(q, p, "play+undo equals direct play");
        assertEquals(q.canonicalKey(), p.canonicalKey(), "keys equal");
    }

    private static void illegalMoves() {
        Position p = Position.initial(Rules.gravity(2, 2, 2, true));
        p.playColumn(0);
        p.playColumn(0);
        assertThrows(() -> p.playColumn(0), "full column rejected (game may also be over)");
        Position f = Position.initial(Rules.free(3, 3, 3, true));
        f.play(Move.cell(1, 1));
        assertThrows(() -> f.play(Move.cell(1, 1)), "occupied cell rejected");
        assertThrows(() -> f.play(Move.column(1)), "unresolved move rejected in free placement");
    }

    private static Position playCols(Rules r, int... colsZeroBased) {
        Position p = Position.initial(r);
        for (int c : colsZeroBased) p.playColumn(c);
        return p;
    }

    private static void winDirections() {
        Rules r = Rules.standardConnect4();
        // vertical: X stacks column 0, O plays elsewhere
        Position v = playCols(r, 0, 1, 0, 1, 0, 1, 0);
        assertEquals(Player.ONE, v.winner().orElseThrow(), "vertical win");
        // horizontal: X bottom 0..3, O stacks on top
        Position h = playCols(r, 0, 0, 1, 1, 2, 2, 3);
        assertEquals(Player.ONE, h.winner().orElseThrow(), "horizontal win");
        // diagonal up: X at (0,0),(1,1),(2,2),(3,3) with supports
        Position d = playCols(r, 0, 1, 1, 2, 3, 2, 2, 3, 4, 3, 3);
        assertEquals(Player.ONE, d.winner().orElseThrow(), "diagonal win, board:\n" + d);
        assertEquals(4, d.completedLineCells().size(), "winning line cells reported");
    }

    private static void diagonalsOff() {
        Rules noDiag = new Rules(4, 4, 4, Placement.FREE, Rules.orthogonalDirections(), false);
        Position p = Position.fromBoard(noDiag,
                "XOO.",
                ".XO.",
                "..XO",
                "...X");
        assertTrue(!p.hasLine(Player.ONE), "diagonal must not count without diagonals");
        Rules diag = new Rules(4, 4, 4, Placement.FREE, Rules.allDirections(), false);
        Position q = Position.initial(diag);
        // X builds the diagonal (0,0),(1,1),(2,2),(3,3); O plays harmless cells.
        q.play(Move.cell(0, 0)); q.play(Move.cell(3, 0));
        q.play(Move.cell(1, 1)); q.play(Move.cell(3, 1));
        q.play(Move.cell(2, 2)); q.play(Move.cell(0, 3));
        q.play(Move.cell(3, 3));
        assertEquals(Player.ONE, q.winner().orElseThrow(), "same diagonal wins with diagonals on");
    }

    private static void freeWin() {
        Position p = Position.initial(Rules.free(5, 5, 4, true));
        p.play(Move.cell(1, 1)); p.play(Move.cell(0, 4));
        p.play(Move.cell(2, 2)); p.play(Move.cell(0, 3));
        p.play(Move.cell(3, 3)); p.play(Move.cell(4, 0));
        assertTrue(!p.isTerminal(), "three diagonal is not terminal");
        p.play(Move.cell(4, 4));
        assertEquals(Player.ONE, p.winner().orElseThrow(), "free-placement diagonal win");
    }

    private static void multiWordWin() {
        Rules r = Rules.free(10, 7, 4, true); // 10*8 = 80 bitboard bits > 64
        assertTrue(r.bitCount() > 64, "board really is multi-word");
        Position p = Position.initial(r);
        p.play(Move.cell(0, 0)); p.play(Move.cell(9, 6));
        p.play(Move.cell(1, 1)); p.play(Move.cell(8, 6));
        p.play(Move.cell(2, 2)); p.play(Move.cell(9, 5));
        p.play(Move.cell(3, 3));
        assertEquals(Player.ONE, p.winner().orElseThrow(), "multi-word diagonal win");
    }

    private static void misereLoses() {
        Rules r = Rules.standardConnect4().withMisere(true);
        Position p = playCols(r, 0, 4, 1, 5, 2, 6, 3); // X completes bottom row 0..3
        assertEquals(Player.ONE, p.lineMaker().orElseThrow(), "X made the line");
        assertEquals(Player.TWO, p.winner().orElseThrow(), "in misere the line maker loses");
    }

    private static void fromBoard() {
        Rules r = Rules.standardConnect4();
        Position p = Position.fromBoard(r,
                ".......",
                ".......",
                ".......",
                ".......",
                ".......",
                "XXX.OOO");
        assertEquals(Player.ONE, p.sideToMove(), "equal counts -> X to move");
        assertTrue(p.lineCompletingMoves(Player.ONE).contains(Move.cell(3, 0)), "X can complete at column 4");
        assertThrows(() -> Position.fromBoard(r,
                ".......",
                ".......",
                ".......",
                ".......",
                "X......",
                "......."), "floating stone rejected");
        assertThrows(() -> Position.fromBoard(r,
                ".......",
                ".......",
                ".......",
                ".......",
                ".......",
                "XX....."), "impossible stone counts rejected");
    }

    private static void mirrorKey() {
        Rules r = Rules.standardConnect4();
        Position a = playCols(r, 0, 1, 2, 2);
        Position b = playCols(r, 6, 5, 4, 4);
        assertEquals(a.canonicalKey(), b.canonicalKey(), "mirrored positions share canonical key");
        assertTrue(!a.rawKey().equals(b.rawKey()), "raw keys differ");
    }

    private static void lineCompletingMatchesSimulation() {
        List<Rules> variants = List.of(
                Rules.gravity(5, 4, 4, true),
                Rules.gravity(5, 4, 4, false),
                Rules.gravity(6, 5, 5, true),
                Rules.free(5, 5, 4, true));
        Random rng = new Random(42);
        for (Rules r : variants) {
            for (int trial = 0; trial < 40; trial++) {
                Position p = randomPosition(r, rng, 3 + rng.nextInt(8));
                if (p.isTerminal()) continue;
                Player side = p.sideToMove();
                List<Move> reported = p.lineCompletingMoves(side);
                for (Move m : p.legalMoves()) {
                    Position q = p.copy();
                    q.play(m);
                    boolean actually = q.hasLine(side);
                    assertEquals(actually, reported.contains(m),
                            "lineCompleting mismatch for " + m + " in\n" + p + "\nrules=" + r);
                }
            }
        }
    }

    private static Position randomPosition(Rules r, Random rng, int plies) {
        Position p = Position.initial(r);
        for (int i = 0; i < plies && !p.isTerminal(); i++) {
            List<Move> legal = p.legalMoves();
            if (legal.isEmpty()) break;
            p.play(legal.get(rng.nextInt(legal.size())));
        }
        return p;
    }

    private static SearchResult exact(Position p, long nodes, long millis) {
        return new ExactSolver().solve(p, nodes, millis, CancellationToken.NONE, SearchListener.NONE);
    }

    private static void exactWinInOne() {
        Position p = Position.fromBoard(Rules.standardConnect4(),
                ".......", ".......", ".......", ".......", ".......", "XXX.OOO");
        SearchResult res = exact(p, 1_000_000, 10_000);
        assertEquals(SearchResult.Certainty.PROVEN_EXACT, res.certainty(), "certainty");
        assertEquals(Outcome.WIN, res.outcomeForSideToMove().orElseThrow(), "outcome");
        assertEquals(3, res.bestMove().orElseThrow().column(), "winning column");
        assertEquals(1, res.pliesToEnd().orElseThrow(), "ends in 1 ply");
        assertTrue(res.moveIsProvenOptimal(), "move proven");
    }

    private static void exactWinInOneK5() {
        Rules r = Rules.gravity(7, 6, 5, true);
        Position p = playCols(r, 0, 0, 1, 1, 2, 2, 3, 3); // X bottom 0..3, X to move
        SearchResult res = exact(p, 1_000_000, 10_000);
        assertEquals(Outcome.WIN, res.outcomeForSideToMove().orElseThrow(), "K5 outcome");
        assertEquals(4, res.bestMove().orElseThrow().column(), "K5 winning column");
        assertEquals(1, res.pliesToEnd().orElseThrow(), "K5 ends in 1");
    }

    private static void exactDoubleThreat() {
        // O owns bottom columns 2,3,4 (1-based) with both ends open: X to move loses in 2.
        Position p = playCols(Rules.standardConnect4(), 6, 1, 5, 2, 6, 3);
        SearchResult res = exact(p, 1_000_000, 10_000);
        assertEquals(SearchResult.Certainty.PROVEN_EXACT, res.certainty(), "certainty");
        assertEquals(Outcome.LOSS, res.outcomeForSideToMove().orElseThrow(), "double threat loses");
        assertEquals(2, res.pliesToEnd().orElseThrow(), "loss in 2 plies");
    }

    private static void exact4x4Draw() {
        Position p = Position.initial(Rules.gravity(4, 4, 4, true));
        SearchResult res = exact(p, 20_000_000, 30_000);
        assertEquals(SearchResult.Certainty.PROVEN_EXACT, res.certainty(), "4x4 proven, got: " + res);
        assertEquals(Outcome.DRAW, res.outcomeForSideToMove().orElseThrow(), "4x4 is a draw (published table)");
    }

    private static void exactTicTacToe() {
        Position p = Position.initial(Rules.free(3, 3, 3, true));
        SearchResult res = exact(p, 5_000_000, 20_000);
        assertEquals(Outcome.DRAW, res.outcomeForSideToMove().orElseThrow(), "tic-tac-toe draw");
    }

    private static void exactBudgetUnknown() {
        Position p = Position.initial(Rules.free(5, 5, 4, true));
        SearchResult res = exact(p, 100, 20_000);
        assertEquals(SearchResult.Certainty.UNKNOWN_LIMIT_REACHED, res.certainty(), "tiny budget -> unknown");
        assertTrue(res.bestMove().isEmpty(), "no move claimed");
    }

    /** Independent reference: plain alpha-beta, no TT, no tactical shortcuts, fixed order. */
    private static int brute(Position p, int alpha, int beta, int cells) {
        if (p.isTerminal()) {
            return p.winner().map(w -> {
                int mag = cells - p.movesPlayed() + 1;
                return w == p.sideToMove() ? mag : -mag;
            }).orElse(0);
        }
        int best = Integer.MIN_VALUE / 2;
        for (Move m : p.legalMoves()) {
            p.play(m);
            int s = -brute(p, -beta, -alpha, cells);
            p.undo();
            best = Math.max(best, s);
            alpha = Math.max(alpha, best);
            if (alpha >= beta) break;
        }
        return best;
    }

    private static void exactVsBruteForce() {
        List<Rules> variants = List.of(
                Rules.gravity(4, 4, 4, true),
                Rules.gravity(4, 4, 4, false),
                Rules.gravity(5, 4, 5, true),
                Rules.gravity(4, 4, 4, true).withMisere(true),
                new Rules(4, 4, 3, Placement.FREE, EnumSet.allOf(Direction.class), false));
        Random rng = new Random(7);
        int compared = 0;
        for (Rules r : variants) {
            for (int trial = 0; trial < 25; trial++) {
                int target = Math.max(4, r.cells() - 10);
                Position p = randomPosition(r, rng, target);
                if (p.isTerminal()) continue;
                int expected = brute(p.copy(), -10_000, 10_000, r.cells());
                SearchResult res = exact(p, 5_000_000, 20_000);
                assertEquals(SearchResult.Certainty.PROVEN_EXACT, res.certainty(),
                        "exact must finish on small position\n" + p + "\nrules=" + r);
                assertEquals(expected, res.score(),
                        "score mismatch vs brute force\n" + p + "\nrules=" + r);
                compared++;
            }
        }
        assertTrue(compared >= 50, "enough comparisons actually ran: " + compared);
    }

    private static SearchResult fastExact(Position p, long nodes, long millis) {
        return new FastExactSolver().solve(p, nodes, millis, CancellationToken.NONE, SearchListener.NONE);
    }

    private static void fastVsGeneric() {
        assertTrue(FastExactSolver.supports(Rules.standardConnect4()), "fast path supports standard rules");
        assertTrue(!FastExactSolver.supports(Rules.standardConnect4().withMisere(true)), "no misere fast path");
        assertTrue(!FastExactSolver.supports(
                new Rules(4, 4, 3, Placement.FREE, EnumSet.allOf(Direction.class), false)), "no free-placement fast path");
        assertTrue(!FastExactSolver.supports(Rules.gravity(10, 7, 4, true)), "no multi-word fast path");

        List<Rules> variants = List.of(
                Rules.standardConnect4(),
                Rules.gravity(7, 6, 4, false),   // no diagonals
                Rules.gravity(6, 5, 5, true),    // K=5
                Rules.gravity(4, 4, 4, true),
                // Mirror-asymmetric direction set: canonical key must fall back to raw.
                new Rules(5, 4, 4, Placement.GRAVITY,
                        EnumSet.of(Direction.VERTICAL, Direction.HORIZONTAL, Direction.DIAGONAL_UP), false));
        Random rng = new Random(99);
        int compared = 0;
        for (Rules r : variants) {
            assertTrue(FastExactSolver.supports(r), "variant should be fast-supported: " + r);
            for (int trial = 0; trial < 20; trial++) {
                int target = Math.max(4, r.cells() - 12);
                Position p = randomPosition(r, rng, target);
                if (p.isTerminal()) continue;
                SearchResult generic = exact(p, 20_000_000, 30_000);
                SearchResult fast = fastExact(p, 20_000_000, 30_000);
                assertEquals(SearchResult.Certainty.PROVEN_EXACT, fast.certainty(),
                        "fast exact must finish\n" + p + "\nrules=" + r);
                assertEquals(generic.score(), fast.score(),
                        "fast/generic score mismatch\n" + p + "\nrules=" + r);
                assertEquals(generic.outcomeForSideToMove(), fast.outcomeForSideToMove(),
                        "fast/generic outcome mismatch\n" + p + "\nrules=" + r);
                // PV must be a legal move sequence from this position.
                Position cursor = p.copy();
                for (Move m : fast.principalVariation()) {
                    cursor.play(m); // throws if illegal
                }
                compared++;
            }
        }
        assertTrue(compared >= 60, "enough fast/generic comparisons ran: " + compared);
    }

    private static void fastKnownResults() {
        SearchResult draw = fastExact(Position.initial(Rules.gravity(5, 4, 4, true)), 0, 30_000);
        assertEquals(SearchResult.Certainty.PROVEN_EXACT, draw.certainty(), "5x4 proven");
        assertEquals(Outcome.DRAW, draw.outcomeForSideToMove().orElseThrow(), "5x4 is a draw");

        SearchResult loss = fastExact(Position.initial(Rules.gravity(6, 4, 4, true)), 0, 60_000);
        assertEquals(SearchResult.Certainty.PROVEN_EXACT, loss.certainty(), "6x4 proven");
        assertEquals(Outcome.LOSS, loss.outcomeForSideToMove().orElseThrow(), "6x4 is a second-player win");

        // Same position, different rules -> different proven outcome
        // (1-based columns 4,5,4,4,1,5,1,3,3,5,5,3,2,2,2,3).
        int[] seq = {3, 4, 3, 3, 0, 4, 0, 2, 2, 4, 4, 2, 1, 1, 1, 2};
        SearchResult std = fastExact(playCols(Rules.standardConnect4(), seq), 0, 60_000);
        assertEquals(Outcome.LOSS, std.outcomeForSideToMove().orElseThrow(), "standard: proven loss");
        assertEquals(6, std.pliesToEnd().orElseThrow(), "standard: loss in 6 plies");
        SearchResult noDiag = fastExact(playCols(Rules.gravity(7, 6, 4, false), seq), 0, 60_000);
        assertEquals(Outcome.DRAW, noDiag.outcomeForSideToMove().orElseThrow(), "no-diagonals: proven draw");
    }

    private static void depthTakesWin() {
        Position p = Position.fromBoard(Rules.standardConnect4(),
                ".......", ".......", ".......", ".......", ".......", "XXX.OOO");
        SearchResult res = new DepthLimitedSolver().solve(p, 4, 0, 10_000,
                CancellationToken.NONE, SearchListener.NONE);
        assertEquals(3, res.bestMove().orElseThrow().column(), "depth search takes the win");
    }

    private static void depthBlocks() {
        // O threatens bottom column 4 (index 3); X must block there.
        Position p = playCols(Rules.standardConnect4(), 0, 4, 0, 5, 1, 6);
        SearchResult res = new DepthLimitedSolver().solve(p, 5, 0, 10_000,
                CancellationToken.NONE, SearchListener.NONE);
        assertEquals(3, res.bestMove().orElseThrow().column(), "depth search blocks the threat, pv=" + res.principalVariation());
    }

    private static void depthLabelledHeuristic() {
        Position p = Position.initial(Rules.standardConnect4());
        SearchResult res = new ConnectKSolver().solve(p, SolveRequest.depthLimited(6, 10_000));
        assertEquals(SearchResult.Certainty.HEURISTIC_DEPTH_LIMITED, res.certainty(), "labelled heuristic");
        assertTrue(!res.moveIsProvenOptimal(), "move not claimed proven");
        assertTrue(res.bestMove().isPresent(), "a move is still recommended");
    }

    private static void theoryStandard() {
        TheoryResult t = TheoryDatabase.lookup(Position.initial(Rules.standardConnect4())).orElseThrow();
        assertEquals(Outcome.WIN, t.outcomeForSideToMove(), "first player wins 7x6");
        assertEquals(41, t.pliesToEnd().orElseThrow(), "win on ply 41");
        assertEquals(3, t.provenBestMove().orElseThrow().column(), "centre opening");
    }

    private static void theory8x8() {
        Rules r = Rules.gravity(8, 8, 4, true);
        TheoryResult t = TheoryDatabase.lookup(Position.initial(r)).orElseThrow();
        assertEquals(Outcome.LOSS, t.outcomeForSideToMove(), "8x8: first player loses (2nd player win)");
    }

    private static void theoryAfterCorner() {
        Position p = playCols(Rules.standardConnect4(), 0);
        TheoryResult t = TheoryDatabase.lookup(p).orElseThrow();
        assertEquals(Outcome.WIN, t.outcomeForSideToMove(), "after corner opening, player to move (P2) wins");
        assertEquals(39, t.pliesToEnd().orElseThrow(), "game ends on ply 40");
    }

    private static void theoryNotForNoDiagonals() {
        assertTrue(TheoryDatabase.lookup(Position.initial(Rules.gravity(7, 6, 4, false))).isEmpty(),
                "no theory claimed for no-diagonals variant");
    }

    private static void facadeAutoTheory() {
        SearchResult res = new ConnectKSolver().solve(
                Position.initial(Rules.standardConnect4()), SolveRequest.defaults());
        assertEquals(SearchResult.Certainty.PROVEN_BY_THEORY, res.certainty(), "theory certainty");
        assertEquals(Outcome.WIN, res.outcomeForSideToMove().orElseThrow(), "outcome");
        assertEquals(3, res.bestMove().orElseThrow().column(), "centre");
        assertTrue(res.moveIsProvenOptimal(), "theory move proven");
        assertEquals(0, res.nodes(), "no search needed");
    }

    private static void facadeExact() {
        Position p = Position.fromBoard(Rules.standardConnect4(),
                ".......", ".......", ".......", ".......", ".......", "XXX.OOO");
        SearchResult res = new ConnectKSolver().solve(p, SolveRequest.exact(10_000, 1_000_000));
        assertEquals(SearchResult.Certainty.PROVEN_EXACT, res.certainty(), "proven by search");
        assertEquals(3, res.bestMove().orElseThrow().column(), "winning move");
    }

    private static void evaluatorBasics() {
        Rules r = Rules.standardConnect4();
        Evaluator ev = new Evaluator(r);
        assertEquals(69, ev.windowCount(), "7x6 K4 has 69 potential lines");
        Position empty = Position.initial(r);
        assertEquals(0, ev.evaluate(empty), "empty board evaluates to 0");
        Position terminal = Position.fromBoard(r,
                ".......", ".......", ".......", ".......", ".......", "XXXXOOO");
        int s = ev.evaluate(terminal);
        assertTrue(s < 0, "side to move has lost");
        assertTrue(Evaluator.isLosingScore(s), "terminal magnitude");
    }

    // ------------------------------------------------------------------
    // Tiny framework
    // ------------------------------------------------------------------

    private static void test(String name, Runnable body) {
        try {
            body.run();
            passed++;
            System.out.println("PASS  " + name);
        } catch (Throwable t) {
            failed++;
            failures.add(name + " — " + t);
            System.out.println("FAIL  " + name + " — " + t);
        }
    }

    private static void assertTrue(boolean cond, String msg) {
        if (!cond) throw new AssertionError(msg);
    }

    private static void assertEquals(Object expected, Object actual, String msg) {
        if (!expected.equals(actual)) {
            throw new AssertionError(msg + " (expected=" + expected + ", actual=" + actual + ")");
        }
    }

    private static void assertEquals(long expected, long actual, String msg) {
        if (expected != actual) {
            throw new AssertionError(msg + " (expected=" + expected + ", actual=" + actual + ")");
        }
    }

    private static void assertEquals(boolean expected, boolean actual, String msg) {
        if (expected != actual) {
            throw new AssertionError(msg + " (expected=" + expected + ", actual=" + actual + ")");
        }
    }

    private static void assertThrows(Runnable body, String msg) {
        try {
            body.run();
        } catch (RuntimeException expected) {
            return;
        }
        throw new AssertionError("expected an exception: " + msg);
    }
}
