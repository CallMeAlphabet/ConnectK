package connectk.core;

import connectk.core.model.Position;
import connectk.core.model.Rules;
import connectk.core.solver.CancellationToken;
import connectk.core.solver.ExactSolver;
import connectk.core.solver.FastExactSolver;
import connectk.core.solver.SearchListener;
import connectk.core.solver.SearchResult;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/**
 * Benchmark harness for the exact solvers, using Pascal Pons's public
 * Connect-4 test sets (the same protocol used by Fhourstones and Kite).
 *
 * <p>File format: one position per line, {@code <move-sequence> <score>},
 * where the move sequence is a string of 1-based column digits played on
 * the standard 7x6 board and the score uses Pons's convention
 * ({@code sign * ceil(|ourScore| / 2)} relative to this project's
 * {@code ±(cells - finalPly + 1)}). Non-matching lines (e.g. headers)
 * are skipped. The test sets themselves are not committed to this repo;
 * download them from http://blog.gamesolver.org/solving-connect-four/02-test-protocol/
 *
 * <p>Usage: {@code BenchMain <file> [maxPositions] [perPositionMillis]}
 *
 * <p>Run after {@code ./build.sh test} (which compiles the test sources):
 * <pre>
 * java -cp build/test:dist/connectk-core.jar connectk.core.BenchMain \
 *     path/to/midgame_easy.txt 200 10000
 * </pre>
 */
public final class BenchMain {

    private record BenchCase(Position position, int expectedPonsScore) {
    }

    public static void main(String[] args) throws IOException {
        if (args.length < 1) {
            System.err.println("usage: BenchMain <file> [maxPositions] [perPositionMillis] [generic|fast]");
            System.exit(2);
        }
        Path file = Path.of(args[0]);
        int maxPositions = args.length > 1 ? Integer.parseInt(args[1]) : Integer.MAX_VALUE;
        long perPositionMillis = args.length > 2 ? Long.parseLong(args[2]) : 10_000;
        boolean fast = args.length > 3 && args[3].equalsIgnoreCase("fast");

        List<BenchCase> cases = load(file, maxPositions);
        System.out.printf("Benchmark: %s (%d positions, %d ms/position budget, solver=%s)%n",
                file.getFileName(), cases.size(), perPositionMillis, fast ? "fast" : "generic");

        // JIT warm-up on the first few positions (results discarded).
        for (int i = 0; i < Math.min(10, cases.size()); i++) {
            solve(cases.get(i).position(), Math.min(perPositionMillis, 2_000), fast);
        }

        long totalNodes = 0;
        long totalSolverMillis = 0;
        int solved = 0;
        int correct = 0;
        int timeouts = 0;
        long wallStart = System.nanoTime();
        for (BenchCase c : cases) {
            SearchResult r = solve(c.position(), perPositionMillis, fast);
            totalNodes += r.nodes();
            totalSolverMillis += r.elapsedMillis();
            if (r.certainty() == SearchResult.Certainty.PROVEN_EXACT) {
                solved++;
                if (toPonsScore(r.score()) == c.expectedPonsScore()) {
                    correct++;
                } else {
                    System.out.printf("  MISMATCH: ours=%d (pons %d) expected pons %d%n",
                            r.score(), toPonsScore(r.score()), c.expectedPonsScore());
                }
            } else {
                timeouts++;
            }
        }
        double wallSeconds = (System.nanoTime() - wallStart) / 1e9;
        System.out.printf("solved %d/%d, correct %d, timeouts %d%n", solved, cases.size(), correct, timeouts);
        System.out.printf("total nodes %,d | solver time %,d ms | wall %.2f s%n",
                totalNodes, totalSolverMillis, wallSeconds);
        if (totalSolverMillis > 0) {
            System.out.printf("throughput: %,.0f nodes/s (solver-reported time)%n",
                    totalNodes * 1000.0 / totalSolverMillis);
        }
        if (solved > 0) {
            System.out.printf("mean per solved position: %,.0f nodes, %.3f ms%n",
                    (double) totalNodes / solved, (double) totalSolverMillis / solved);
        }
    }

    // Reused across positions: FastExactSolver clears its (lazily
    // allocated) transposition table at the start of every solve, like
    // the reset() in Pons's benchmark protocol.
    private static final FastExactSolver FAST = new FastExactSolver();

    private static SearchResult solve(Position p, long millis, boolean fast) {
        if (fast) {
            return FAST.solve(p, 0, millis, CancellationToken.NONE, SearchListener.NONE);
        }
        return new ExactSolver().solve(p, 0, millis, CancellationToken.NONE, SearchListener.NONE);
    }

    /** Our score convention -> Pons's: sign * ceil(|score| / 2). */
    static int toPonsScore(int score) {
        int a = Math.abs(score);
        int v = (a + 1) / 2;
        return score < 0 ? -v : v;
    }

    private static List<BenchCase> load(Path file, int maxPositions) throws IOException {
        List<BenchCase> out = new ArrayList<>();
        Rules rules = Rules.standardConnect4();
        for (String line : Files.readAllLines(file)) {
            if (out.size() >= maxPositions) break;
            String[] parts = line.trim().split("\\s+");
            if (parts.length != 2 || !parts[0].matches("[1-7]+")) continue;
            int expected;
            try {
                expected = Integer.parseInt(parts[1]);
            } catch (NumberFormatException e) {
                continue;
            }
            Position p = Position.initial(rules);
            boolean ok = true;
            for (char ch : parts[0].toCharArray()) {
                try {
                    p.playColumn(ch - '1');
                } catch (RuntimeException e) {
                    ok = false;
                    break;
                }
            }
            if (ok && !p.isTerminal()) out.add(new BenchCase(p, expected));
        }
        return out;
    }
}
