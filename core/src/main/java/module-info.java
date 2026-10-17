/**
 * Pure Connect-k engine: model, exact solver, depth-limited solver, theory
 * database. This module deliberately depends on nothing but java.base so it
 * can be embedded in any front end (CLI, GUI, server, tests) or used alone.
 */
module connectk.core {
    exports connectk.core.model;
    exports connectk.core.eval;
    exports connectk.core.solver;
    exports connectk.core.theory;
}
