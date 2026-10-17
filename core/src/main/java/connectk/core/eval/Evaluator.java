package connectk.core.eval;

import connectk.core.model.Bits;
import connectk.core.model.Direction;
import connectk.core.model.Placement;
import connectk.core.model.Player;
import connectk.core.model.Position;
import connectk.core.model.Rules;

import java.util.ArrayList;
import java.util.List;

/**
 * Static heuristic evaluation, used <em>only</em> by the depth-limited
 * solver. Scores are from the perspective of the side to move; the
 * function is zero-sum and symmetric.
 *
 * <p>Method: sum over every potential winning line ("window") of k cells.
 * A window containing only one player's stones scores by stone count
 * (geometric weights), discounted under gravity by how many support
 * stones its empty cells still need before they become playable, and
 * boosted sharply when it is an immediately playable (k-1) threat.
 * Contested windows score 0. Terminal positions return
 * {@code ±WIN_SCORE} adjusted by game length so that faster wins (and
 * slower losses) are preferred.
 */
public final class Evaluator {

    public static final int WIN_SCORE = 1_000_000;
    private static final int HEURISTIC_CLAMP = WIN_SCORE / 2;

    private final Rules rules;
    private final List<long[]> windowMasks = new ArrayList<>();
    private final List<int[]> windowCells = new ArrayList<>();
    private final int[] weights;

    public Evaluator(Rules rules) {
        this.rules = rules;
        int k = rules.connect();
        this.weights = new int[k + 1];
        weights[0] = 0;
        int w = 1;
        for (int i = 1; i <= k; i++) {
            weights[i] = w;
            w *= 8;
        }
        for (Direction d : rules.directions()) {
            for (int c = 0; c < rules.width(); c++) {
                for (int r = 0; r < rules.height(); r++) {
                    int endC = c + d.dCol() * (k - 1);
                    int endR = r + d.dRow() * (k - 1);
                    if (endC < 0 || endC >= rules.width() || endR < 0 || endR >= rules.height()) continue;
                    long[] mask = Bits.create(rules.wordCount());
                    int[] cells = new int[k];
                    for (int i = 0; i < k; i++) {
                        int idx = rules.index(c + d.dCol() * i, r + d.dRow() * i);
                        Bits.set(mask, idx);
                        cells[i] = idx;
                    }
                    windowMasks.add(mask);
                    windowCells.add(cells);
                }
            }
        }
    }

    public int windowCount() {
        return windowMasks.size();
    }

    /** Exact terminal score for the side to move, or 0 for a draw. */
    public static int terminalScore(Position p) {
        return p.winner()
                .map(w -> w == p.sideToMove()
                        ? WIN_SCORE - p.movesPlayed() * 10
                        : -WIN_SCORE + p.movesPlayed() * 10)
                .orElse(0);
    }

    public static boolean isWinningScore(int score) {
        return score > HEURISTIC_CLAMP;
    }

    public static boolean isLosingScore(int score) {
        return score < -HEURISTIC_CLAMP;
    }

    /** Heuristic score for the side to move. Terminal positions get exact terminal scores. */
    public int evaluate(Position p) {
        if (p.isTerminal()) return terminalScore(p);
        Player side = p.sideToMove();
        long[] own = p.stoneBits(side);
        long[] opp = p.stoneBits(side.other());
        boolean gravity = rules.placement() == Placement.GRAVITY;
        int k = rules.connect();
        double score = 0.0;

        for (int wi = 0; wi < windowMasks.size(); wi++) {
            long[] window = windowMasks.get(wi);
            int ownCount = Bits.andCount(own, window);
            int oppCount = Bits.andCount(opp, window);
            if (ownCount > 0 && oppCount > 0) continue;
            if (ownCount == 0 && oppCount == 0) continue;
            boolean mine = ownCount > 0;
            int count = mine ? ownCount : oppCount;

            double factor = 1.0;
            int minSupport = Integer.MAX_VALUE;
            int totalSupport = 0;
            if (gravity) {
                for (int idx : windowCells.get(wi)) {
                    if (Bits.test(own, idx) || Bits.test(opp, idx)) continue;
                    int col = rules.columnOf(idx);
                    int row = rules.rowOf(idx);
                    int support = Math.max(0, row - p.columnCount(col));
                    totalSupport += support;
                    minSupport = Math.min(minSupport, support);
                }
                factor = 1.0 / (1.0 + 0.4 * totalSupport);
            } else {
                minSupport = 0;
            }
            if (count == k - 1) {
                if (minSupport == 0) factor *= 25.0;       // playable right now: huge threat
                else if (minSupport == 1) factor *= 5.0;   // becomes playable after one support stone
            }
            double value = weights[count] * factor;
            score += mine ? value : -1.05 * value; // defend slightly harder than attack
        }

        // Small positional tie-break: central columns participate in more
        // lines (only matters among otherwise equal windows).
        score += 0.05 * (centrality(own) - centrality(opp));

        int s = (int) Math.round(score);
        if (s > HEURISTIC_CLAMP) s = HEURISTIC_CLAMP;
        if (s < -HEURISTIC_CLAMP) s = -HEURISTIC_CLAMP;
        return s;
    }

    private int centrality(long[] stones) {
        int total = 0;
        int w = rules.width();
        for (int c = 0; c < w; c++) {
            int columnWeight = Math.min(c, w - 1 - c) + 1;
            for (int r = 0; r < rules.height(); r++) {
                if (Bits.test(stones, rules.index(c, r))) total += columnWeight;
            }
        }
        return total;
    }
}
