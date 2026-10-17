package connectk.core;

import connectk.core.model.Move;
import connectk.core.model.Position;
import connectk.core.model.Rules;
import connectk.core.solver.CancellationToken;
import connectk.core.solver.ExactSolver;
import connectk.core.solver.FastExactSolver;
import connectk.core.solver.SearchListener;
import connectk.core.solver.SearchResult;

import java.util.List;
import java.util.Random;

/**
 * Debug/validation harness for the claim-even bound: plays many random
 * and claim-style games, and whenever {@link FastExactSolver#claimEvenUpperBound}
 * fires on a position with few enough empties, checks it against the
 * generic exact solver. Prints the first violation found (if any).
 *
 * Usage: java connectk.core.ClaimFuzzMain [games] [maxEmptiesForExactCheck]
 */
public final class ClaimFuzzMain {

    public static void main(String[] args) {
        int games = args.length > 0 ? Integer.parseInt(args[0]) : 3000;
        int maxEmpties = args.length > 1 ? Integer.parseInt(args[1]) : 18;
        Rules rules = Rules.standardConnect4();
        Random rng = new Random(1234567);
        long positions = 0;
        long fired = 0;
        long checked = 0;
        long negative = 0;
        for (int g = 0; g < games; g++) {
            Position pos = Position.initial(rules);
            int lastColumn = -1;
            int mode = g % 3; // 0 random, 1 P2 claims, 2 P1 claims (roles swapped by parity of play)
            while (!pos.isTerminal()) {
                int empties = rules.cells() - pos.movesPlayed();
                if (pos.movesPlayed() >= 16) {
                    positions++;
                    int bound = FastExactSolver.claimEvenUpperBound(pos);
                    if (bound != Integer.MAX_VALUE) {
                        fired++;
                        if (bound < 0) negative++;
                        if (empties <= maxEmpties) {
                            SearchResult exact = new ExactSolver().solve(pos, 0, 60_000,
                                    CancellationToken.NONE, SearchListener.NONE);
                            checked++;
                            if (exact.certainty() != SearchResult.Certainty.PROVEN_EXACT) {
                                System.out.println("oracle UNKNOWN at empties=" + empties);
                            } else if (exact.score() > bound) {
                                System.out.println("VIOLATION: bound=" + bound
                                        + " exact=" + exact.score()
                                        + " movesPlayed=" + pos.movesPlayed()
                                        + " sideToMove=" + pos.sideToMove());
                                System.out.println(pos);
                                System.out.println("heights parity: cols with odd height:");
                                for (int c = 0; c < rules.width(); c++) {
                                    System.out.printf("col %d height %d (%s)%n", c + 1,
                                            pos.columnCount(c),
                                            pos.columnCount(c) % 2 == 1 ? "odd" : "even");
                                }
                                return;
                            }
                        }
                    }
                }
                List<Move> legal = pos.legalMoves();
                if (legal.isEmpty()) break;
                Move chosen = null;
                boolean claimerTurn = (mode == 1 && pos.movesPlayed() % 2 == 1)
                        || (mode == 2 && pos.movesPlayed() % 2 == 0);
                if (claimerTurn && lastColumn >= 0) {
                    for (Move m : legal) {
                        if (m.column() == lastColumn) {
                            chosen = m;
                            break;
                        }
                    }
                }
                if (chosen == null) chosen = legal.get(rng.nextInt(legal.size()));
                pos.play(chosen);
                lastColumn = chosen.column();
            }
        }
        System.out.printf("done: games=%d positions=%d fired=%d negative=%d exact-checked=%d violations=0%n",
                games, positions, fired, negative, checked);
    }
}
