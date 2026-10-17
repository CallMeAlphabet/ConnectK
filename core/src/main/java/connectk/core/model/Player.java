package connectk.core.model;

/** The two players. {@link #ONE} always moves first. */
public enum Player {
    ONE('X'),
    TWO('O');

    private final char symbol;

    Player(char symbol) {
        this.symbol = symbol;
    }

    public char symbol() {
        return symbol;
    }

    public Player other() {
        return this == ONE ? TWO : ONE;
    }
}
