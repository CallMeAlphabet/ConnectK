package connectk.cli;

import connectk.core.model.Direction;
import connectk.core.model.Move;
import connectk.core.model.Placement;
import connectk.core.model.Player;
import connectk.core.model.Position;
import connectk.core.model.Rules;
import connectk.core.solver.ConnectKSolver;
import connectk.core.solver.SearchResult;
import connectk.core.solver.SolveMode;
import connectk.core.solver.SolveRequest;
import connectk.core.theory.TheoryDatabase;

import java.util.ArrayList;
import java.util.EnumSet;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

/**
 * Command-line interface to the Connect-k engine. No GUI classes are
 * involved; this module works on a headless machine.
 *
 * <pre>
 *   solve    [--width 7 --height 6 --k 4 --placement gravity|free
 *             --diagonals true|false --misere] [--moves 4,4,5]
 *             [--mode auto|exact|depth] [--depth 10] [--time 5000]
 *             [--nodes 5000000] [--exact-time 1500] [--no-theory]
 *   selfplay [same rule options] [--depth 8] [--move-time 300]
 *   theory   [--width 7 --height 6]          (published result for empty board)
 * </pre>
 * Moves are 1-based. Gravity: comma-separated columns ("4,4,5").
 * Free placement: semicolon-separated column:row pairs ("5:5;6:5"), row 1 = bottom.
 */
public final class Main {

    private Main() {
    }

    public static void main(String[] argv) {
        if (argv.length == 0) {
            printHelp();
            return;
        }
        String command = argv[0];
        Map<String, String> opts = parseOpts(argv);
        try {
            switch (command) {
                case "solve" -> cmdSolve(opts);
                case "selfplay" -> cmdSelfplay(opts);
                case "theory" -> cmdTheory(opts);
                case "help", "--help", "-h" -> printHelp();
                default -> {
                    System.err.println("unknown command: " + command);
                    printHelp();
                    System.exit(2);
                }
            }
        } catch (IllegalArgumentException | IllegalStateException e) {
            System.err.println("error: " + e.getMessage());
            System.exit(2);
        }
    }

    private static void printHelp() {
        System.out.println("""
                Connect-k solver CLI

                Usage:
                  solve    [rule options] [--moves ...] [--mode auto|exact|depth]
                           [--depth N] [--time MS] [--nodes N] [--exact-time MS] [--no-theory]
                  selfplay [rule options] [--depth N] [--move-time MS]
                  theory   [--width W --height H]

                Rule options:
                  --width W (default 7)   --height H (default 6)   --k K (default 4)
                  --placement gravity|free (default gravity)
                  --diagonals true|false (default true)
                  --directions H,V,DU,DD (overrides --diagonals)
                  --misere (making a line loses)

                Moves are 1-based. Gravity: --moves 4,4,5
                Free placement:  --moves "5:5;6:5"   (column:row, row 1 = bottom)

                Modes:
                  auto   theory where published, exact proof when tractable,
                         otherwise depth-limited search (labelled heuristic)
                  exact  proof or UNKNOWN (budget: --time / --nodes)
                  depth  depth-limited heuristic search only""");
    }

    private static Map<String, String> parseOpts(String[] argv) {
        Map<String, String> out = new HashMap<>();
        for (int i = 1; i < argv.length; i++) {
            String a = argv[i];
            if (!a.startsWith("--")) throw new IllegalArgumentException("unexpected argument: " + a);
            String key = a.substring(2);
            if (key.contains("=")) {
                String[] kv = key.split("=", 2);
                out.put(kv[0], kv[1]);
            } else if (key.equals("misere") || key.equals("no-theory")) {
                out.put(key, "true");
            } else {
                if (i + 1 >= argv.length) throw new IllegalArgumentException("missing value for --" + key);
                out.put(key, argv[++i]);
            }
        }
        return out;
    }

    private static int intOpt(Map<String, String> o, String key, int def) {
        String v = o.get(key);
        if (v == null) return def;
        try {
            return Integer.parseInt(v.trim());
        } catch (NumberFormatException e) {
            throw new IllegalArgumentException("--" + key + " must be an integer, got '" + v + "'");
        }
    }

    private static long longOpt(Map<String, String> o, String key, long def) {
        String v = o.get(key);
        if (v == null) return def;
        try {
            return Long.parseLong(v.trim());
        } catch (NumberFormatException e) {
            throw new IllegalArgumentException("--" + key + " must be an integer, got '" + v + "'");
        }
    }

    private static Rules buildRules(Map<String, String> o) {
        int w = intOpt(o, "width", 7);
        int h = intOpt(o, "height", 6);
        int k = intOpt(o, "k", 4);
        Placement placement = switch (o.getOrDefault("placement", "gravity").toLowerCase()) {
            case "gravity" -> Placement.GRAVITY;
            case "free" -> Placement.FREE;
            default -> throw new IllegalArgumentException("--placement must be gravity|free");
        };
        EnumSet<Direction> directions;
        String dirs = o.get("directions");
        if (dirs != null) {
            directions = EnumSet.noneOf(Direction.class);
            for (String part : dirs.split(",")) {
                switch (part.trim().toUpperCase()) {
                    case "H" -> directions.add(Direction.HORIZONTAL);
                    case "V" -> directions.add(Direction.VERTICAL);
                    case "DU" -> directions.add(Direction.DIAGONAL_UP);
                    case "DD" -> directions.add(Direction.DIAGONAL_DOWN);
                    default -> throw new IllegalArgumentException("unknown direction '" + part + "' (use H,V,DU,DD)");
                }
            }
        } else {
            boolean diag = !"false".equalsIgnoreCase(o.getOrDefault("diagonals", "true"));
            directions = diag ? Rules.allDirections() : Rules.orthogonalDirections();
        }
        boolean misere = o.containsKey("misere");
        return new Rules(w, h, k, placement, directions, misere);
    }

    private static Position buildPosition(Rules rules, Map<String, String> o) {
        Position p = Position.initial(rules);
        String moves = o.get("moves");
        if (moves == null || moves.isBlank()) return p;
        if (rules.placement() == Placement.GRAVITY) {
            for (String part : moves.split(",")) {
                int col = Integer.parseInt(part.trim()) - 1;
                p.playColumn(col);
            }
        } else {
            for (String part : moves.split(";")) {
                String[] cr = part.trim().split(":");
                if (cr.length != 2) throw new IllegalArgumentException("free move must be column:row, got '" + part + "'");
                p.play(Move.cell(Integer.parseInt(cr[0].trim()) - 1, Integer.parseInt(cr[1].trim()) - 1));
            }
        }
        return p;
    }

    private static SolveRequest buildRequest(Map<String, String> o) {
        SolveMode mode = switch (o.getOrDefault("mode", "auto").toLowerCase()) {
            case "auto" -> SolveMode.AUTO;
            case "exact" -> SolveMode.EXACT;
            case "depth", "depth-limited", "heuristic" -> SolveMode.DEPTH_LIMITED;
            default -> throw new IllegalArgumentException("--mode must be auto|exact|depth");
        };
        return new SolveRequest(
                mode,
                intOpt(o, "depth", 10),
                longOpt(o, "time", 5_000),
                longOpt(o, "nodes", 20_000_000),
                longOpt(o, "exact-time", 1_500),
                longOpt(o, "exact-nodes", longOpt(o, "nodes", 5_000_000)),
                !o.containsKey("no-theory"));
    }

    private static void cmdSolve(Map<String, String> o) {
        Rules rules = buildRules(o);
        Position p = buildPosition(rules, o);
        System.out.println("Rules: " + rules);
        System.out.println(p);
        System.out.println("Side to move: " + p.sideToMove() + " (" + p.sideToMove().symbol() + ")"
                + " after " + p.movesPlayed() + " plies");
        SearchResult result = new ConnectKSolver().solve(p, buildRequest(o));
        System.out.println(result);
    }

    private static void cmdSelfplay(Map<String, String> o) {
        Rules rules = buildRules(o);
        Position p = Position.initial(rules);
        int depth = intOpt(o, "depth", 8);
        long moveTime = longOpt(o, "move-time", 300);
        ConnectKSolver solver = new ConnectKSolver();
        List<String> played = new ArrayList<>();
        System.out.println("Rules: " + rules + "  (both sides: depth " + depth + ", " + moveTime + " ms/move)");
        while (!p.isTerminal()) {
            SearchResult r = solver.solve(p, new SolveRequest(
                    SolveMode.AUTO, depth, moveTime, 0, Math.min(500, moveTime), 2_000_000, true));
            Optional<Move> move = r.bestMove();
            if (move.isEmpty()) {
                System.out.println("No move available (" + r.certainty() + "); stopping.");
                break;
            }
            Player mover = p.sideToMove();
            Move playedMove = p.play(move.get());
            played.add(playedMove.notation());
            System.out.printf("%3d. %s plays %-6s [%s, score %d, depth %d, %d nodes]%n",
                    p.movesPlayed(), mover, playedMove.notation(), r.certainty(), r.score(),
                    r.depthReached(), r.nodes());
        }
        System.out.println(p);
        System.out.println("Moves: " + String.join(" ", played));
        p.winner().ifPresentOrElse(
                w -> System.out.println("Result: " + w + " (" + w.symbol() + ") wins"),
                () -> System.out.println(p.isDraw() ? "Result: draw" : "Result: unfinished"));
    }

    private static void cmdTheory(Map<String, String> o) {
        Rules rules = buildRules(o);
        Position p = buildPosition(rules, o);
        TheoryDatabase.lookup(p).ifPresentOrElse(
                t -> {
                    System.out.println("Published result for " + rules
                            + " after " + p.movesPlayed() + " plies:");
                    System.out.println("  outcome for side to move: " + t.outcomeForSideToMove());
                    t.pliesToEnd().ifPresent(pl -> System.out.println("  game ends in " + pl + " plies under perfect play"));
                    t.provenBestMove().ifPresent(m -> System.out.println("  published optimal move: " + m.notation()));
                    System.out.println("  source: " + t.source());
                },
                () -> System.out.println("No published mathematical result in the database for: " + rules
                        + " after " + p.movesPlayed() + " plies."));
    }
}
