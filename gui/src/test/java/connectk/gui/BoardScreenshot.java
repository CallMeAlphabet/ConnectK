package connectk.gui;

import connectk.core.model.Move;
import connectk.core.model.Position;
import connectk.core.model.Rules;

import javax.imageio.ImageIO;
import java.awt.Graphics2D;
import java.awt.image.BufferedImage;
import java.io.File;

/**
 * Dev helper: render a board mid-game to a PNG without a display
 * (java -Djava.awt.headless=true ... BoardScreenshot out.png).
 */
public final class BoardScreenshot {
    public static void main(String[] args) throws Exception {
        Position p = Position.initial(Rules.standardConnect4());
        for (int c : new int[]{3, 4, 3, 3, 0, 4, 0, 2, 2, 4, 4, 2, 1, 1}) {
            p.playColumn(c);
        }
        BoardPanel panel = new BoardPanel(p);
        panel.setPosition(p);
        panel.setSuggestedMove(Move.cell(5, 0));
        panel.setSize(780, 700);
        BufferedImage img = new BufferedImage(780, 700, BufferedImage.TYPE_INT_ARGB);
        Graphics2D g = img.createGraphics();
        panel.printAll(g);
        g.dispose();
        ImageIO.write(img, "png", new File(args.length > 0 ? args[0] : "board.png"));
        System.out.println("wrote screenshot");
    }
}
