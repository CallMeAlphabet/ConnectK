# Published mathematical results encoded in this project

The engine never claims a position is "solved" unless one of these holds:

1. the exact solver proved it in this run, or
2. the result is in the table below (encoded in
   `core/src/main/java/connectk/core/theory/TheoryDatabase.java`).

Everything else is reported as a depth-limited heuristic estimate.

## Gravity Connect-4 (all directions, normal rules): empty-board values

`1st` = first-player win, `2nd` = second-player win, `draw` = draw.
Rows are board height, columns are board width.

| H \ W | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 |
|---|---|---|---|---|---|---|---|---|---|
| 4 | draw | draw | 2nd | draw | 2nd | 2nd | 2nd | 2nd | 2nd |
| 5 | draw | draw | draw | draw | 1st | 1st | 1st | 1st | |
| 6 | draw | draw | 2nd | **1st** | 2nd | 2nd | 1st | | |
| 7 | draw | draw | 1st | draw | 1st | 1st | | | |
| 8 | draw | draw | 2nd | 1st | 2nd | | | | |
| 9 | draw | draw | 1st | draw | 1st | | | | |
| 10 | draw | draw | 2nd | 1st | | | | | |
| 11 | draw | draw | 1st | | | | | | |
| 12 | draw | draw | 2nd | | | | | | |
| 13 | draw | draw | | | | | | | |

Also encoded: the ply on which the game ends under perfect play for most
of these sizes, and the exact first-move values of the standard 7x6 game:

| First move (column) | 1 | 2 | 3 | 4 (centre) | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|
| Outcome | loss | loss | draw | **win** | draw | loss | loss |
| Game ends on ply | 40 | 42 | 42 | 41 | 42 | 42 | 40 |

Second-player wins are possible under gravity (e.g. 8x8, 6x6) because
gravity creates Zugzwang — an extra stone can hurt — which breaks the
strategy-stealing argument that rules out second-player wins in
free-placement (m,n,k) games.

## Sources

- James D. Allen (1 Oct 1988) and Victor Allis (16 Oct 1988), independent
  solutions of 7x6 Connect-Four: first-player win by opening in the centre
  column. Allis, *A Knowledge-Based Approach of Connect-Four*, MSc thesis,
  Vrije Universiteit Amsterdam — <https://tromp.github.io/c4/connect4_thesis.pdf>
- John Tromp, *Solving connect-4 on medium board sizes*, ICGA Journal 31(2),
  110–112 (2008); results for width+height ≤ 15 (2006) and 8x8 (2015) —
  <https://tromp.github.io/c4/c4.html>
- Christophe Steininger, extension to width+height = 16 and beyond,
  tables reproduced at <https://github.com/ChristopheSteininger/c4>
- Exact 7x6 scores (distances to the end) are as computed by modern strong
  solvers, e.g. Pascal Pons's solver and tutorial —
  <http://blog.gamesolver.org/solving-connect-four/01-introduction/>

## Known results that are NOT encoded (out of scope for this engine)

- **Free-placement Gomoku (15x15, k=5)**: weakly solved, first-player win
  (Victor Allis, 1993/94, via threat-space / dependency-based / proof-number
  search). Our exact negamax cannot reproduce that proof; free-placement
  positions here are handled by exact search on small boards and by the
  depth-limited solver on large ones, labelled heuristic.
- **Connect6** (k=6, 19x19, *two* stones per turn): unsolved; only
  individual openings have been proved. This engine models one stone per
  turn, so official Connect6 is not implemented.
- **Infinite Connect-Four**: draw (Yamaguchi et al., 2012, explicit
  cannot-lose strategies). No finite search can show this.
- **Misère Connect-k** is implemented as a rule flag, and although it has
  been solved analytically for all w,h,k (Steele & Larremore, 2024/25;
  standard 7x6 is a second-player win), those constructive strategies are
  not encoded — misère positions are proved by search or labelled
  heuristic.
- **No-diagonals and gravity Connect-5/6 variants**: no published solved
  tables were found (October 2026). The engine treats them as ordinary
  parameter combinations: exact search where feasible, depth-limited
  search elsewhere.
