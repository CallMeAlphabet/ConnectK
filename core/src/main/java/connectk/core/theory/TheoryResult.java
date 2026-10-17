package connectk.core.theory;

import connectk.core.model.Move;
import connectk.core.model.Outcome;

import java.util.Optional;
import java.util.OptionalInt;

/**
 * A published, mathematically proven result about a position.
 *
 * @param outcomeForSideToMove proven outcome under perfect play
 * @param pliesToEnd           plies until the game ends under perfect play, if known
 * @param provenBestMove       a move known to achieve the outcome, if published
 * @param source               where the result comes from
 */
public record TheoryResult(
        Outcome outcomeForSideToMove,
        OptionalInt pliesToEnd,
        Optional<Move> provenBestMove,
        String source) {
}
