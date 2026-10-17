package connectk.core.solver;

/** Internal control-flow exception: a search hit its node/time/cancel budget. */
final class SearchAbortedException extends RuntimeException {

    enum Reason { NODE_LIMIT, TIME_LIMIT, CANCELLED }

    private final Reason reason;

    SearchAbortedException(Reason reason) {
        super(reason.name());
        this.reason = reason;
    }

    Reason reason() {
        return reason;
    }
}
