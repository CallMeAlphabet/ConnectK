package connectk.core.solver;

/** Cooperative cancellation for searches (e.g. a GUI "cancel" button). */
@FunctionalInterface
public interface CancellationToken {
    boolean isCancelled();

    CancellationToken NONE = () -> false;
}
