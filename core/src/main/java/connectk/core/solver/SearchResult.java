package connectk.core.solver;

import connectk.core.model.Move;
import connectk.core.model.Outcome;

import java.util.List;
import java.util.Optional;
import java.util.OptionalInt;

/**
 * The result of analysing a position. The {@link Certainty} field is the
 * most important part: it distinguishes a mathematical proof from a
 * depth-limited heuristic opinion.
 */
public final class SearchResult {

    public enum Certainty {
        /** Outcome (and best move) proved by exhaustive search in this run. */
        PROVEN_EXACT,
        /** Outcome taken from published mathematical results. The recommended move may still be heuristic — see {@link SearchResult#moveIsProvenOptimal()}. */
        PROVEN_BY_THEORY,
        /** Depth-limited search with a heuristic evaluation. Not a proof. */
        HEURISTIC_DEPTH_LIMITED,
        /** Budget exhausted before any conclusion. */
        UNKNOWN_LIMIT_REACHED
    }

    private final Certainty certainty;
    private final Optional<Outcome> outcomeForSideToMove;
    private final Optional<Move> bestMove;
    private final boolean moveIsProvenOptimal;
    private final int score;
    private final OptionalInt pliesToEnd;
    private final int depthReached;
    private final long nodes;
    private final long elapsedMillis;
    private final List<Move> principalVariation;
    private final String explanation;

    public SearchResult(Certainty certainty,
                        Optional<Outcome> outcomeForSideToMove,
                        Optional<Move> bestMove,
                        boolean moveIsProvenOptimal,
                        int score,
                        OptionalInt pliesToEnd,
                        int depthReached,
                        long nodes,
                        long elapsedMillis,
                        List<Move> principalVariation,
                        String explanation) {
        this.certainty = certainty;
        this.outcomeForSideToMove = outcomeForSideToMove;
        this.bestMove = bestMove;
        this.moveIsProvenOptimal = moveIsProvenOptimal;
        this.score = score;
        this.pliesToEnd = pliesToEnd;
        this.depthReached = depthReached;
        this.nodes = nodes;
        this.elapsedMillis = elapsedMillis;
        this.principalVariation = List.copyOf(principalVariation);
        this.explanation = explanation;
    }

    public Certainty certainty() {
        return certainty;
    }

    public boolean isProvenOutcome() {
        return certainty == Certainty.PROVEN_EXACT || certainty == Certainty.PROVEN_BY_THEORY;
    }

    /** Proven outcome if proven; heuristic prediction otherwise; empty if unknown. */
    public Optional<Outcome> outcomeForSideToMove() {
        return outcomeForSideToMove;
    }

    public Optional<Move> bestMove() {
        return bestMove;
    }

    public boolean moveIsProvenOptimal() {
        return moveIsProvenOptimal;
    }

    /**
     * Exact score if proven by search (sign = outcome; magnitude encodes
     * distance to the end: |score| = cells - finalPly + 1), heuristic
     * evaluation otherwise.
     */
    public int score() {
        return score;
    }

    public OptionalInt pliesToEnd() {
        return pliesToEnd;
    }

    public int depthReached() {
        return depthReached;
    }

    public long nodes() {
        return nodes;
    }

    public long elapsedMillis() {
        return elapsedMillis;
    }

    public List<Move> principalVariation() {
        return principalVariation;
    }

    public String explanation() {
        return explanation;
    }

    public static SearchResult unknown(long nodes, long elapsedMillis, String explanation) {
        return new SearchResult(Certainty.UNKNOWN_LIMIT_REACHED, Optional.empty(), Optional.empty(),
                false, 0, OptionalInt.empty(), 0, nodes, elapsedMillis, List.of(), explanation);
    }

    @Override
    public String toString() {
        StringBuilder sb = new StringBuilder();
        sb.append(switch (certainty) {
            case PROVEN_EXACT -> "PROVEN (exact search)";
            case PROVEN_BY_THEORY -> "PROVEN (published theory)";
            case HEURISTIC_DEPTH_LIMITED -> "Heuristic, depth " + depthReached + " (not a proof)";
            case UNKNOWN_LIMIT_REACHED -> "Unknown (limit reached)";
        });
        outcomeForSideToMove.ifPresent(o -> sb.append(" | outcome for side to move: ").append(o));
        bestMove.ifPresent(m -> sb.append(" | best move: ").append(m.notation())
                .append(moveIsProvenOptimal ? " (proven optimal)" : ""));
        sb.append(" | score: ").append(score);
        pliesToEnd.ifPresent(p -> sb.append(" | ends in ").append(p).append(" plies"));
        sb.append(" | nodes: ").append(nodes).append(" | ").append(elapsedMillis).append(" ms");
        if (!principalVariation.isEmpty()) {
            sb.append("\n  PV: ");
            for (int i = 0; i < principalVariation.size(); i++) {
                if (i > 0) sb.append(' ');
                sb.append(principalVariation.get(i).notation());
            }
        }
        sb.append("\n  ").append(explanation);
        return sb.toString();
    }
}
