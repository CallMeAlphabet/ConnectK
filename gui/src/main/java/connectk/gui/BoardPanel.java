package connectk.gui;

import connectk.core.model.Move;
import connectk.core.model.Placement;
import connectk.core.model.Player;
import connectk.core.model.Position;
import connectk.core.model.Rules;

import javax.swing.JPanel;
import java.awt.AlphaComposite;
import java.awt.BasicStroke;
import java.awt.Color;
import java.awt.Dimension;
import java.awt.Font;
import java.awt.GradientPaint;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.util.List;
import java.util.function.Consumer;

/**
 * Renders a {@link Position} and translates clicks into moves.
 * Rendering is pure Java2D vector drawing (crisp at any scale, no image
 * assets) and uses no absolute screen coordinates, popups anchored to the
 * screen, Robot or tray APIs — all of which behave differently (or not at
 * all) under Wayland.
 */
public final class BoardPanel extends JPanel {

    private static final Color BACKGROUND = new Color(27, 32, 40);
    private static final Color BOARD = new Color(33, 84, 158);
    private static final Color BOARD_TOP = new Color(62, 124, 204);
    private static final Color BOARD_EDGE = new Color(22, 58, 112);
    private static final Color HOLE = new Color(16, 42, 78);
    private static final Color HOLE_SHADOW = new Color(8, 22, 44);
    private static final Color P1 = new Color(224, 62, 58);
    private static final Color P1_LIGHT = new Color(248, 150, 140);
    private static final Color P1_DARK = new Color(150, 32, 28);
    private static final Color P2 = new Color(242, 198, 54);
    private static final Color P2_LIGHT = new Color(255, 232, 150);
    private static final Color P2_DARK = new Color(170, 128, 20);
    private static final Color LABELS = new Color(170, 182, 200);

    private Position position;
    private Consumer<Move> moveHandler = m -> {
    };
    private boolean inputEnabled = true;
    private int hoverColumn = -1;
    private int hoverRow = -1;
    private Move suggestedMove;

    public BoardPanel(Position initial) {
        this.position = initial.copy();
        setBackground(BACKGROUND);
        setOpaque(true);
        updatePreferredSize();

        MouseAdapter mouse = new MouseAdapter() {
            @Override
            public void mouseMoved(MouseEvent e) {
                int[] cell = cellAt(e.getX(), e.getY());
                int c = cell == null ? -1 : cell[0];
                int r = cell == null ? -1 : cell[1];
                if (c != hoverColumn || r != hoverRow) {
                    hoverColumn = c;
                    hoverRow = r;
                    repaint();
                }
            }

            @Override
            public void mouseExited(MouseEvent e) {
                hoverColumn = -1;
                hoverRow = -1;
                repaint();
            }

            @Override
            public void mousePressed(MouseEvent e) {
                if (!inputEnabled || position.isTerminal()) return;
                Move m = hoverMove();
                if (m != null) moveHandler.accept(m);
            }
        };
        addMouseListener(mouse);
        addMouseMotionListener(mouse);
    }

    public void setPosition(Position p) {
        this.position = p.copy();
        updatePreferredSize();
        repaint();
    }

    public void setMoveHandler(Consumer<Move> handler) {
        this.moveHandler = handler == null ? m -> {
        } : handler;
    }

    public void setInputEnabled(boolean enabled) {
        this.inputEnabled = enabled;
        repaint();
    }

    public void setSuggestedMove(Move move) {
        this.suggestedMove = move;
        repaint();
    }

    private void updatePreferredSize() {
        Rules r = position.rules();
        int cell = Math.max(24, Math.min(64, 640 / Math.max(r.width(), r.height())));
        setPreferredSize(new Dimension(r.width() * cell + 72, r.height() * cell + 86));
        revalidate();
    }

    private record Geometry(int cell, int originX, int originY) {
    }

    private Geometry geometry() {
        Rules r = position.rules();
        int availW = Math.max(50, getWidth() - 62);
        int availH = Math.max(50, getHeight() - 76);
        int cell = Math.max(14, Math.min(availW / r.width(), availH / r.height()));
        int boardW = cell * r.width();
        int boardH = cell * r.height();
        int ox = (getWidth() - boardW) / 2 + 12;
        int oy = (getHeight() - boardH) / 2 - 8;
        return new Geometry(cell, ox, oy);
    }

    /** Board cell at a pixel position as [column, row] (row 0 = bottom), or null. */
    private int[] cellAt(int x, int y) {
        Rules r = position.rules();
        Geometry g = geometry();
        int c = (x - g.originX()) / g.cell();
        int displayRow = (y - g.originY()) / g.cell();
        if (x < g.originX() || y < g.originY() || c < 0 || c >= r.width() || displayRow < 0 || displayRow >= r.height()) {
            return null;
        }
        return new int[]{c, r.height() - 1 - displayRow};
    }

    private Move hoverMove() {
        if (hoverColumn < 0) return null;
        try {
            if (position.rules().placement() == Placement.GRAVITY) {
                return position.resolve(Move.column(hoverColumn));
            }
            if (hoverRow < 0 || !position.isEmpty(hoverColumn, hoverRow)) return null;
            return Move.cell(hoverColumn, hoverRow);
        } catch (RuntimeException e) {
            return null;
        }
    }

    @Override
    protected void paintComponent(Graphics graphics) {
        super.paintComponent(graphics);
        Graphics2D g = (Graphics2D) graphics.create();
        try {
            g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
            Rules r = position.rules();
            Geometry geo = geometry();
            int cell = geo.cell();
            int ox = geo.originX();
            int oy = geo.originY();
            int boardW = cell * r.width();
            int boardH = cell * r.height();

            // Soft shadow, stand legs, then the board body (vertical gradient).
            g.setComposite(AlphaComposite.getInstance(AlphaComposite.SRC_OVER, 0.35f));
            g.setColor(Color.BLACK);
            g.fillRoundRect(ox - 8, oy - 4, boardW + 24, boardH + 24, 22, 22);
            g.setComposite(AlphaComposite.SrcOver);
            g.setColor(BOARD_EDGE);
            int legTop = oy + boardH + 6;
            int legBottom = oy + boardH + 24;
            int legW = Math.max(14, cell / 2);
            g.fillPolygon(new int[]{ox + 2, ox + 2 + legW, ox + 2 + legW + 6, ox - 4},
                    new int[]{legTop, legTop, legBottom, legBottom}, 4);
            g.fillPolygon(new int[]{ox + boardW - 2 - legW, ox + boardW - 2, ox + boardW + 4, ox + boardW - 2 - legW - 6},
                    new int[]{legTop, legTop, legBottom, legBottom}, 4);
            g.setColor(BOARD_EDGE);
            g.fillRoundRect(ox - 12, oy - 12, boardW + 24, boardH + 24, 22, 22);
            g.setPaint(new GradientPaint(ox, oy - 8, BOARD_TOP, ox, oy + boardH + 8, BOARD));
            g.fillRoundRect(ox - 8, oy - 8, boardW + 16, boardH + 16, 18, 18);
            g.setPaint(null);

            // Hover column highlight (gravity)
            Move hover = inputEnabled ? hoverMove() : null;
            if (hover != null && r.placement() == Placement.GRAVITY) {
                g.setComposite(AlphaComposite.getInstance(AlphaComposite.SRC_OVER, 0.14f));
                g.setColor(Color.WHITE);
                g.fillRoundRect(ox + hover.column() * cell - 2, oy - 6, cell + 4, boardH + 12, 12, 12);
                g.setComposite(AlphaComposite.SrcOver);
            }

            List<Move> winCells = position.completedLineCells();
            Move last = position.lastMove().orElse(null);

            for (int c = 0; c < r.width(); c++) {
                for (int row = 0; row < r.height(); row++) {
                    int displayRow = r.height() - 1 - row;
                    int x = ox + c * cell;
                    int y = oy + displayRow * cell;
                    int pad = Math.max(2, cell / 10);
                    // hole with a subtle inner shadow crescent at the top
                    g.setColor(HOLE);
                    g.fillOval(x + pad, y + pad, cell - 2 * pad, cell - 2 * pad);
                    g.setColor(HOLE_SHADOW);
                    g.setStroke(new BasicStroke(Math.max(1.5f, cell / 22f)));
                    g.drawArc(x + pad + 1, y + pad + 1, cell - 2 * pad - 2, cell - 2 * pad - 2, 25, 130);
                    g.setStroke(new BasicStroke(1f));

                    Player owner = position.ownerAt(c, row).orElse(null);
                    if (owner != null) {
                        drawDisc(g, x, y, cell, owner, 1.0f);
                    }

                    Move thisCell = Move.cell(c, row);
                    if (winCells.contains(thisCell)) {
                        ring(g, x, y, cell, new Color(90, 240, 130), Math.max(3f, cell / 12f));
                    } else if (thisCell.equals(last)) {
                        ring(g, x, y, cell, new Color(255, 255, 255, 190), Math.max(2f, cell / 18f));
                    }
                }
            }

            // Ghost disc for the hovered move
            if (hover != null && !position.isTerminal() && inputEnabled) {
                int displayRow = r.height() - 1 - hover.row();
                drawDisc(g, ox + hover.column() * cell, oy + displayRow * cell, cell,
                        position.sideToMove(), 0.5f);
            }

            // Suggested move marker
            if (suggestedMove != null && suggestedMove.isResolved()
                    && suggestedMove.column() < r.width() && suggestedMove.row() < r.height()) {
                int displayRow = r.height() - 1 - suggestedMove.row();
                ring(g, ox + suggestedMove.column() * cell, oy + displayRow * cell, cell,
                        new Color(80, 220, 255), Math.max(3f, cell / 13f));
            }

            // Labels: columns below (1-based), rows on the left (1 = bottom)
            g.setColor(LABELS);
            g.setFont(new Font(Font.SANS_SERIF, Font.PLAIN, Math.max(10, cell / 4)));
            for (int c = 0; c < r.width(); c++) {
                String s = Integer.toString(c + 1);
                int sw = g.getFontMetrics().stringWidth(s);
                g.drawString(s, ox + c * cell + (cell - sw) / 2, oy + boardH + 42);
            }
            for (int row = 0; row < r.height(); row++) {
                int displayRow = r.height() - 1 - row;
                String s = Integer.toString(row + 1);
                int sw = g.getFontMetrics().stringWidth(s);
                g.drawString(s, ox - 22 - sw, oy + displayRow * cell + cell / 2 + g.getFontMetrics().getAscent() / 2 - 2);
            }
        } finally {
            g.dispose();
        }
    }

    private void ring(Graphics2D g, int x, int y, int cell, Color color, float width) {
        int pad = Math.max(1, cell / 16);
        g.setColor(color);
        g.setStroke(new BasicStroke(width));
        g.drawOval(x + pad, y + pad, cell - 2 * pad, cell - 2 * pad);
        g.setStroke(new BasicStroke(1f));
    }

    private void drawDisc(Graphics2D g, int x, int y, int cell, Player owner, float alpha) {
        int pad = Math.max(2, cell / 9);
        int d = cell - 2 * pad;
        Color base = owner == Player.ONE ? P1 : P2;
        Color light = owner == Player.ONE ? P1_LIGHT : P2_LIGHT;
        Color dark = owner == Player.ONE ? P1_DARK : P2_DARK;
        g.setComposite(AlphaComposite.getInstance(AlphaComposite.SRC_OVER, alpha));
        // Rim (darker edge), then the face shifted up-left: cheap 3D shading.
        g.setColor(dark);
        g.fillOval(x + pad, y + pad, d, d);
        int rim = Math.max(1, d / 14);
        g.setPaint(new GradientPaint(x + pad, y + pad, light, x + pad + d, y + pad + d, base));
        g.fillOval(x + pad, y + pad, d - rim, d - rim);
        g.setPaint(null);
        // Classic inner ring.
        g.setColor(dark);
        g.setStroke(new BasicStroke(Math.max(1f, d / 16f)));
        int ringPad = Math.max(2, d / 4);
        g.drawOval(x + pad + ringPad, y + pad + ringPad, d - rim - 2 * ringPad, d - rim - 2 * ringPad);
        g.setStroke(new BasicStroke(1f));
        // Specular highlight.
        g.setColor(new Color(255, 255, 255, 170));
        int hd = Math.max(2, d / 4);
        g.fillOval(x + pad + d / 5, y + pad + d / 9, hd, Math.max(2, hd * 2 / 3));
        g.setComposite(AlphaComposite.SrcOver);
    }
}
