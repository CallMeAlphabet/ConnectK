package connectk.core.solver;

import connectk.core.model.Move;

import java.util.Optional;

/**
 * Progress callbacks from a running search. Called from the search thread;
 * implementations (especially GUIs) must be thread-safe and cheap.
 */
@FunctionalInterface
public interface SearchListener {
    void onProgress(long nodes, int depth, Optional<Move> bestMoveSoFar);

    SearchListener NONE = (nodes, depth, best) -> {
    };
}
