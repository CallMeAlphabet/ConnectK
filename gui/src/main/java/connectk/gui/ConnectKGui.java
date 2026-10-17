package connectk.gui;

import connectk.core.model.Move;
import connectk.core.model.Placement;
import connectk.core.model.Player;
import connectk.core.model.Position;
import connectk.core.model.Rules;
import connectk.core.solver.ConnectKSolver;
import connectk.core.solver.SearchResult;
import connectk.core.solver.SolveMode;
import connectk.core.solver.SolveRequest;
import connectk.core.theory.OpeningBook;
import connectk.core.theory.TheoryDatabase;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JButton;
import javax.swing.JCheckBox;
import javax.swing.JComboBox;
import javax.swing.JFrame;
import javax.swing.JLabel;
import javax.swing.JOptionPane;
import javax.swing.JPanel;
import javax.swing.JScrollPane;
import javax.swing.JSpinner;
import javax.swing.JTextArea;
import javax.swing.SpinnerNumberModel;
import javax.swing.SwingUtilities;
import javax.swing.SwingWorker;
import javax.swing.Timer;
import javax.swing.UIManager;
import java.awt.BorderLayout;
import java.awt.Color;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.Font;
import java.awt.GraphicsEnvironment;
import java.awt.GridLayout;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import java.util.stream.Stream;

/**
 * Swing front end for the Connect-k engine.
 *
 * <p>Wayland notes (see README): stock OpenJDK renders Swing through
 * XWayland; with a JetBrains Runtime (or a Wakefield JDK build) the same
 * jar runs natively on Wayland via {@code -Dawt.toolkit.name=WLToolkit}.
 * The UI is written to be safe in both worlds: one top-level window, no
 * absolute screen positioning ({@code setLocationByPlatform}), no Robot,
 * no system tray, no always-on-top.</p>
 */
public final class ConnectKGui {

    private enum PlayMode { HUMAN_HUMAN, HUMAN_AI, AI_HUMAN, AI_AI }

    private enum Purpose { HINT, AI_MOVE }

    private ConnectKSolver solver = new ConnectKSolver();
    private String activeBookName;
    private final Map<String, OpeningBook> bookCache = new HashMap<>();
    private boolean applyingPreset;
    private Position position = Position.initial(Rules.standardConnect4());

    private JFrame frame;
    private BoardPanel boardPanel;
    private JComboBox<String> presetBox;
    private JSpinner widthSpinner;
    private JSpinner heightSpinner;
    private JSpinner kSpinner;
    private JComboBox<String> placementBox;
    private JCheckBox diagonalsBox;
    private JCheckBox misereBox;
    private JComboBox<String> playModeBox;
    private JComboBox<String> modeBox;
    private JSpinner depthSpinner;
    private JSpinner timeSpinner;
    private JCheckBox theoryBox;
    private JLabel statusLabel;
    private JLabel theoryLabel;
    private JLabel certaintyLabel;
    private JTextArea resultArea;
    private JTextArea movesArea;
    private JButton hintButton;
    private JButton aiButton;
    private JButton cancelButton;

    private SwingWorker<SearchResult, Void> worker;
    private AtomicBoolean cancelFlag;
    private boolean searching;
    private Timer aiTimer;

    public static void main(String[] args) {
        if (GraphicsEnvironment.isHeadless()) {
            System.err.println("No display available. Use the CLI module instead "
                    + "(java --module-path dist --module connectk.cli/connectk.cli.Main help).");
            System.exit(2);
        }
        installLookAndFeel();
        SwingUtilities.invokeLater(() -> new ConnectKGui().show());
    }

    private static void installLookAndFeel() {
        // Prefer GTK on Linux (matches the desktop), fall back to the system LAF.
        for (String candidate : new String[]{
                "com.sun.java.swing.plaf.gtk.GTKLookAndFeel",
                UIManager.getSystemLookAndFeelClassName()}) {
            try {
                UIManager.setLookAndFeel(candidate);
                return;
            } catch (Exception ignored) {
                // try next
            }
        }
    }

    private void show() {
        frame = new JFrame("Connect-k Solver");
        frame.setDefaultCloseOperation(JFrame.EXIT_ON_CLOSE);
        frame.setLayout(new BorderLayout());

        boardPanel = new BoardPanel(position);
        boardPanel.setMoveHandler(this::onBoardMove);

        frame.add(buildTopBar(), BorderLayout.NORTH);
        JPanel centre = new JPanel(new BorderLayout());
        centre.setBackground(boardPanel.getBackground());
        centre.add(boardPanel, BorderLayout.CENTER);
        frame.add(centre, BorderLayout.CENTER);
        frame.add(buildSidePanel(), BorderLayout.EAST);

        statusLabel = new JLabel(" ");
        statusLabel.setBorder(BorderFactory.createEmptyBorder(4, 8, 4, 8));
        frame.add(statusLabel, BorderLayout.SOUTH);

        aiTimer = new Timer(350, e -> startSearch(Purpose.AI_MOVE));
        aiTimer.setRepeats(false);

        frame.setMinimumSize(new Dimension(920, 620));
        frame.setSize(1220, 780);
        frame.setLocationByPlatform(true); // let the compositor/WM place the window
        frame.setVisible(true);

        newGame();
    }

    // ------------------------------------------------------------------
    // UI construction
    // ------------------------------------------------------------------

    private JPanel buildTopBar() {
        JPanel top = new JPanel(new FlowLayout(FlowLayout.LEFT, 8, 6));
        JLabel header = new JLabel("Rules");
        header.setFont(header.getFont().deriveFont(Font.BOLD));
        top.add(header);

        presetBox = new JComboBox<>(new String[]{
                "Standard 7x6, K=4",
                "Small (exact-friendly) 5x4, K=4",
                "Gravity 9x6, K=5",
                "Gravity 10x7, K=6",
                "No diagonals 7x6, K=4",
                "Misere 7x6, K=4",
                "Free 5x5, K=4",
                "Free 9x9, K=5 (Gomoku-lite)",
                "Custom"});
        presetBox.addActionListener(e -> applyPreset());
        top.add(new JLabel("Preset:"));
        top.add(presetBox);

        widthSpinner = new JSpinner(new SpinnerNumberModel(7, 1, 25, 1));
        heightSpinner = new JSpinner(new SpinnerNumberModel(6, 1, 25, 1));
        kSpinner = new JSpinner(new SpinnerNumberModel(4, 2, 12, 1));
        top.add(new JLabel("W:"));
        top.add(widthSpinner);
        top.add(new JLabel("H:"));
        top.add(heightSpinner);
        top.add(new JLabel("K:"));
        top.add(kSpinner);

        placementBox = new JComboBox<>(new String[]{"Gravity (drop)", "Free placement"});
        top.add(placementBox);
        diagonalsBox = new JCheckBox("Diagonals", true);
        misereBox = new JCheckBox("Misere (line loses)");
        top.add(diagonalsBox);
        top.add(misereBox);
        // Any manual rule edit turns the preset into "Custom".
        Runnable markCustom = () -> {
            if (!applyingPreset) presetBox.setSelectedItem("Custom");
        };
        widthSpinner.addChangeListener(e -> markCustom.run());
        heightSpinner.addChangeListener(e -> markCustom.run());
        kSpinner.addChangeListener(e -> markCustom.run());
        placementBox.addActionListener(e -> markCustom.run());
        diagonalsBox.addActionListener(e -> markCustom.run());
        misereBox.addActionListener(e -> markCustom.run());

        JButton newGame = new JButton("New game");
        newGame.addActionListener(e -> newGame());
        top.add(newGame);
        JButton undo = new JButton("Undo");
        undo.addActionListener(e -> undo());
        top.add(undo);
        return top;
    }

    private JPanel buildSidePanel() {
        JPanel side = new JPanel();
        side.setLayout(new BoxLayout(side, BoxLayout.Y_AXIS));
        side.setPreferredSize(new Dimension(360, 10));
        side.setBorder(BorderFactory.createEmptyBorder(8, 8, 8, 8));

        theoryLabel = new JLabel(" ");
        side.add(section("Published mathematical status", theoryLabel));

        playModeBox = new JComboBox<>(new String[]{
                "Human vs Human", "Human vs AI", "AI vs Human", "AI vs AI"});
        playModeBox.setSelectedIndex(1);
        playModeBox.addActionListener(e -> {
            updateStatus();
            maybeScheduleAi();
        });
        side.add(section("Players", playModeBox));

        JPanel solverPanel = new JPanel(new GridLayout(0, 2, 6, 4));
        modeBox = new JComboBox<>(new String[]{
                "Auto (theory > exact > depth)",
                "Exact proof only",
                "Depth-limited only"});
        depthSpinner = new JSpinner(new SpinnerNumberModel(10, 1, 40, 1));
        timeSpinner = new JSpinner(new SpinnerNumberModel(3.0, 0.1, 600.0, 0.5));
        theoryBox = new JCheckBox("Use published theory", true);
        solverPanel.add(new JLabel("Mode:"));
        solverPanel.add(modeBox);
        solverPanel.add(new JLabel("Depth (plies):"));
        solverPanel.add(depthSpinner);
        solverPanel.add(new JLabel("Time limit (s):"));
        solverPanel.add(timeSpinner);
        solverPanel.add(theoryBox);
        solverPanel.add(new JLabel(""));
        side.add(section("Solver", solverPanel));

        JPanel buttons = new JPanel(new FlowLayout(FlowLayout.LEFT, 6, 4));
        hintButton = new JButton("Analyse / Hint");
        hintButton.addActionListener(e -> startSearch(Purpose.HINT));
        aiButton = new JButton("AI move now");
        aiButton.addActionListener(e -> startSearch(Purpose.AI_MOVE));
        cancelButton = new JButton("Cancel");
        cancelButton.setEnabled(false);
        cancelButton.addActionListener(e -> {
            if (cancelFlag != null) cancelFlag.set(true);
        });
        buttons.add(hintButton);
        buttons.add(aiButton);
        buttons.add(cancelButton);
        side.add(wrapFullWidth(buttons));

        certaintyLabel = new JLabel("No analysis yet");
        certaintyLabel.setOpaque(true);
        certaintyLabel.setBorder(BorderFactory.createEmptyBorder(6, 8, 6, 8));
        certaintyLabel.setFont(certaintyLabel.getFont().deriveFont(Font.BOLD));
        side.add(wrapFullWidth(certaintyLabel));

        resultArea = new JTextArea(10, 34);
        resultArea.setEditable(false);
        resultArea.setLineWrap(true);
        resultArea.setWrapStyleWord(true);
        resultArea.setFont(new Font(Font.MONOSPACED, Font.PLAIN, 12));
        JScrollPane resultScroll = new JScrollPane(resultArea);
        side.add(section("Analysis", resultScroll));

        movesArea = new JTextArea(3, 34);
        movesArea.setEditable(false);
        movesArea.setLineWrap(true);
        movesArea.setWrapStyleWord(true);
        movesArea.setFont(new Font(Font.MONOSPACED, Font.PLAIN, 12));
        JScrollPane movesScroll = new JScrollPane(movesArea);
        side.add(section("Moves", movesScroll));
        side.add(Box.createVerticalGlue());
        return side;
    }

    /** A clean side-panel section: small bold header over full-width content. */
    private static JPanel section(String title, java.awt.Component content) {
        JPanel p = new JPanel(new BorderLayout(0, 4));
        JLabel header = new JLabel(title);
        header.setFont(header.getFont().deriveFont(Font.BOLD));
        p.add(header, BorderLayout.NORTH);
        p.add(content, BorderLayout.CENTER);
        p.setMaximumSize(new Dimension(Integer.MAX_VALUE,
                content.getPreferredSize().height + header.getPreferredSize().height + 18));
        p.setBorder(BorderFactory.createEmptyBorder(8, 0, 2, 0));
        return p;
    }

    private static JPanel wrapFullWidth(java.awt.Component c) {
        JPanel p = new JPanel(new BorderLayout());
        p.add(c, BorderLayout.CENTER);
        p.setMaximumSize(new Dimension(Integer.MAX_VALUE, c.getPreferredSize().height + 10));
        return p;
    }

    // ------------------------------------------------------------------
    // Game flow
    // ------------------------------------------------------------------

    private void applyPreset() {
        String preset = (String) presetBox.getSelectedItem();
        if (preset == null || preset.startsWith("Custom")) return;
        applyingPreset = true;
        try {
        misereBox.setSelected(false);
        diagonalsBox.setSelected(true);
        placementBox.setSelectedIndex(0);
        if (preset.startsWith("Standard")) setDims(7, 6, 4);
        else if (preset.startsWith("Small")) setDims(5, 4, 4);
        else if (preset.startsWith("Gravity 9x6")) setDims(9, 6, 5);
        else if (preset.startsWith("Gravity 10x7")) setDims(10, 7, 6);
        else if (preset.startsWith("No diagonals")) {
            setDims(7, 6, 4);
            diagonalsBox.setSelected(false);
        } else if (preset.startsWith("Misere")) {
            setDims(7, 6, 4);
            misereBox.setSelected(true);
        } else if (preset.startsWith("Free 5x5")) {
            setDims(5, 5, 4);
            placementBox.setSelectedIndex(1);
        } else if (preset.startsWith("Free 9x9")) {
            setDims(9, 9, 5);
            placementBox.setSelectedIndex(1);
        }
        } finally {
            applyingPreset = false;
        }
        newGame();
    }

    private void setDims(int w, int h, int k) {
        widthSpinner.setValue(w);
        heightSpinner.setValue(h);
        kSpinner.setValue(k);
    }

    private Rules rulesFromControls() {
        int w = (Integer) widthSpinner.getValue();
        int h = (Integer) heightSpinner.getValue();
        int k = (Integer) kSpinner.getValue();
        Placement placement = placementBox.getSelectedIndex() == 0 ? Placement.GRAVITY : Placement.FREE;
        return new Rules(w, h, k, placement,
                diagonalsBox.isSelected() ? Rules.allDirections() : Rules.orthogonalDirections(),
                misereBox.isSelected());
    }

    private static final Pattern BOOK_NAME =
            Pattern.compile("^(std|nodiag)(\\d+)x(\\d+)(?:-k(\\d+))?-d(\\d+)\\.book$");

    /**
     * The facade for a game, with a generated opening book attached when
     * {@code ./books} contains one whose file name encodes exactly these
     * rules (see books/README.md). Books are never used across rulesets:
     * their keys are bitboards of one specific board shape.
     */
    private ConnectKSolver solverFor(Rules rules) {
        activeBookName = null;
        Path dir = Path.of("books");
        if (!Files.isDirectory(dir)) return new ConnectKSolver();
        Path best = null;
        int bestDepth = -1;
        try (Stream<Path> files = Files.list(dir)) {
            for (Path f : files.toList()) {
                Matcher m = BOOK_NAME.matcher(f.getFileName().toString());
                if (!m.matches()) continue;
                int w = Integer.parseInt(m.group(2));
                int h = Integer.parseInt(m.group(3));
                int k = m.group(4) == null ? 4 : Integer.parseInt(m.group(4));
                boolean diag = m.group(1).equals("std");
                Rules bookRules = new Rules(w, h, k, Placement.GRAVITY,
                        diag ? Rules.allDirections() : Rules.orthogonalDirections(), false);
                int depth = Integer.parseInt(m.group(5));
                if (bookRules.equals(rules) && depth > bestDepth) {
                    best = f;
                    bestDepth = depth;
                }
            }
        } catch (IOException ignored) {
            return new ConnectKSolver();
        }
        if (best == null) return new ConnectKSolver();
        final Path chosen = best;
        OpeningBook book = bookCache.computeIfAbsent(chosen.toString(), key -> {
            try {
                return OpeningBook.load(chosen);
            } catch (IOException e) {
                return null;
            }
        });
        if (book == null) return new ConnectKSolver();
        activeBookName = chosen.getFileName() + " (" + book.size() + " entries)";
        return new ConnectKSolver(book);
    }

    private void newGame() {
        if (aiTimer != null) aiTimer.stop();
        cancelRunningSearch();
        Rules rules;
        try {
            rules = rulesFromControls();
        } catch (IllegalArgumentException e) {
            JOptionPane.showMessageDialog(frame, e.getMessage(), "Invalid rules", JOptionPane.ERROR_MESSAGE);
            return;
        }
        position = Position.initial(rules);
        solver = solverFor(rules);
        boardPanel.setSuggestedMove(null);
        boardPanel.setPosition(position);
        resultArea.setText("");
        certaintyLabel.setText("No analysis yet");
        certaintyLabel.setBackground(new Color(220, 220, 220));
        certaintyLabel.setForeground(Color.DARK_GRAY);
        updateStatus();
        maybeScheduleAi();
    }

    private PlayMode playMode() {
        return PlayMode.values()[Math.max(0, playModeBox.getSelectedIndex())];
    }

    private boolean isAiTurn() {
        return switch (playMode()) {
            case HUMAN_HUMAN -> false;
            case HUMAN_AI -> position.sideToMove() == Player.TWO;
            case AI_HUMAN -> position.sideToMove() == Player.ONE;
            case AI_AI -> true;
        };
    }

    private void maybeScheduleAi() {
        if (searching || position.isTerminal()) return;
        if (isAiTurn()) aiTimer.restart();
    }

    private void onBoardMove(Move move) {
        if (searching || position.isTerminal()) return;
        if (isAiTurn()) return;
        applyMove(move);
    }

    private void applyMove(Move move) {
        try {
            position.play(move);
        } catch (RuntimeException e) {
            statusLabel.setText(e.getMessage());
            return;
        }
        boardPanel.setSuggestedMove(null);
        boardPanel.setPosition(position);
        updateStatus();
        maybeScheduleAi();
    }

    private void undo() {
        if (searching) return;
        aiTimer.stop();
        if (position.moveHistory().isEmpty()) return;
        position.undo();
        // In human-vs-AI modes, undo the AI reply as well so the human is to move.
        if ((playMode() == PlayMode.HUMAN_AI || playMode() == PlayMode.AI_HUMAN)
                && !position.moveHistory().isEmpty() && isAiTurn()) {
            position.undo();
        }
        boardPanel.setSuggestedMove(null);
        boardPanel.setPosition(position);
        updateStatus();
    }

    private void updateStatus() {
        StringBuilder sb = new StringBuilder();
        if (position.winner().isPresent()) {
            Player w = position.winner().get();
            String how = position.rules().misere()
                    ? " wins (opponent was forced to connect " + position.rules().connect() + ")"
                    : " wins";
            sb.append(w).append(" (").append(w.symbol()).append(")").append(how)
                    .append(" on ply ").append(position.movesPlayed());
        } else if (position.isDraw()) {
            sb.append("Draw — board full on ply ").append(position.movesPlayed());
        } else {
            Player side = position.sideToMove();
            sb.append(side).append(" (").append(side.symbol()).append(") to move · ply ")
                    .append(position.movesPlayed() + 1);
        }
        statusLabel.setText(sb.toString() + "   ·   " + position.rules());

        String bookNote = activeBookName == null ? ""
                : "<br><small>Opening book: " + activeBookName + "</small>";
        TheoryDatabase.lookup(position).ifPresentOrElse(
                t -> theoryLabel.setText("<html>Proven: <b>" + t.outcomeForSideToMove()
                        + "</b> for the side to move"
                        + (t.pliesToEnd().isPresent() ? ", ends in " + t.pliesToEnd().getAsInt() + " plies" : "")
                        + "<br><small>" + t.source() + "</small>" + bookNote + "</html>"),
                () -> theoryLabel.setText("<html>No published mathematical result for this position/ruleset."
                        + bookNote + "</html>"));

        StringBuilder moves = new StringBuilder();
        var history = position.moveHistory();
        for (int i = 0; i < history.size(); i++) {
            if (i % 2 == 0) moves.append(i / 2 + 1).append(". ");
            moves.append(history.get(i).notation()).append(' ');
        }
        movesArea.setText(moves.toString());
    }

    // ------------------------------------------------------------------
    // Search
    // ------------------------------------------------------------------

    private SolveRequest requestFromControls() {
        SolveMode mode = switch (modeBox.getSelectedIndex()) {
            case 1 -> SolveMode.EXACT;
            case 2 -> SolveMode.DEPTH_LIMITED;
            default -> SolveMode.AUTO;
        };
        int depth = (Integer) depthSpinner.getValue();
        long millis = Math.round(((Number) timeSpinner.getValue()).doubleValue() * 1000.0);
        long exactMillis = mode == SolveMode.AUTO ? Math.min(2_000, millis) : millis;
        // GUI searches are time-governed; node limits live in the CLI (0 = unlimited).
        return new SolveRequest(mode, depth, millis, 0, exactMillis, 0, theoryBox.isSelected());
    }

    private void cancelRunningSearch() {
        if (cancelFlag != null) cancelFlag.set(true);
    }

    private void startSearch(Purpose purpose) {
        if (searching) return;
        if (position.isTerminal() && purpose == Purpose.AI_MOVE) return;
        SolveRequest request;
        try {
            request = requestFromControls();
        } catch (IllegalArgumentException e) {
            JOptionPane.showMessageDialog(frame, e.getMessage(), "Invalid solver settings", JOptionPane.ERROR_MESSAGE);
            return;
        }
        Position snapshot = position.copy();
        cancelFlag = new AtomicBoolean(false);
        AtomicBoolean flag = cancelFlag;
        searching = true;
        boardPanel.setInputEnabled(false);
        hintButton.setEnabled(false);
        aiButton.setEnabled(false);
        cancelButton.setEnabled(true);
        statusLabel.setText("Searching… (" + request.mode() + ")");

        worker = new SwingWorker<>() {
            @Override
            protected SearchResult doInBackground() {
                return solver.solve(snapshot, request, flag::get,
                        (nodes, depth, best) -> SwingUtilities.invokeLater(() ->
                                statusLabel.setText("Searching… nodes=" + nodes
                                        + (depth >= 0 ? " depth=" + depth : "")
                                        + best.map(m -> " best=" + m.notation()).orElse(""))));
            }

            @Override
            protected void done() {
                searching = false;
                boardPanel.setInputEnabled(true);
                hintButton.setEnabled(true);
                aiButton.setEnabled(true);
                cancelButton.setEnabled(false);
                if (flag.get()) {
                    statusLabel.setText("Search cancelled");
                    return;
                }
                SearchResult result;
                try {
                    result = get();
                } catch (Exception e) {
                    statusLabel.setText("Search failed: " + e.getCause());
                    return;
                }
                displayResult(result);
                if (purpose == Purpose.AI_MOVE) {
                    if (result.bestMove().isPresent() && position.movesPlayed() == snapshot.movesPlayed()) {
                        applyMove(result.bestMove().get());
                    } else {
                        statusLabel.setText("AI has no move to play (" + result.certainty()
                                + "). In Exact mode, raise the budget or switch to Auto.");
                    }
                }
            }
        };
        worker.execute();
    }

    private void displayResult(SearchResult result) {
        String label;
        Color bg;
        switch (result.certainty()) {
            case PROVEN_EXACT -> {
                label = "PROVEN — exact search";
                bg = new Color(46, 125, 50);
            }
            case PROVEN_BY_THEORY -> {
                label = "PROVEN — published theory"
                        + (result.moveIsProvenOptimal() ? " (move proven optimal)" : " (move from heuristic search)");
                bg = new Color(0, 105, 120);
            }
            case HEURISTIC_DEPTH_LIMITED -> {
                label = "Heuristic (depth " + result.depthReached() + ") — NOT a proof";
                bg = new Color(230, 126, 34);
            }
            default -> {
                label = "Unknown — budget exhausted";
                bg = new Color(183, 28, 28);
            }
        }
        certaintyLabel.setText(label);
        certaintyLabel.setBackground(bg);
        certaintyLabel.setForeground(Color.WHITE);
        resultArea.setText(result.toString());
        boardPanel.setSuggestedMove(result.bestMove().orElse(null));
        updateStatusKeepingResult();
    }

    /** updateStatus() refreshes the status line; keep the board/moves in sync too. */
    private void updateStatusKeepingResult() {
        updateStatus();
        boardPanel.setPosition(position);
        // setPosition() must not clear the suggestion; restore it from the panel's own state.
        boardPanel.repaint();
    }
}
