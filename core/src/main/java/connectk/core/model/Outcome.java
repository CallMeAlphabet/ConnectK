package connectk.core.model;

/**
 * Game-theoretic outcome, always expressed from the point of view of one
 * specified player or of the side to move (documented at each use site).
 */
public enum Outcome {
    WIN, DRAW, LOSS;

    public Outcome negated() {
        return switch (this) {
            case WIN -> LOSS;
            case DRAW -> DRAW;
            case LOSS -> WIN;
        };
    }

    public static Outcome fromScoreSign(int score) {
        if (score > 0) return WIN;
        if (score < 0) return LOSS;
        return DRAW;
    }
}
