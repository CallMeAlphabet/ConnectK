package connectk.gui;

import javax.imageio.ImageIO;
import java.awt.Rectangle;
import java.awt.Robot;
import java.awt.Toolkit;
import java.awt.image.BufferedImage;
import java.io.File;

/** Dev helper: open the real GUI (needs a display/Xvfb), screenshot it, exit. */
public final class GuiScreenshot {
    public static void main(String[] args) throws Exception {
        ConnectKGui.main(new String[0]);
        Thread.sleep(3000);
        Robot robot = new Robot();
        robot.waitForIdle();
        Rectangle screen = new Rectangle(Toolkit.getDefaultToolkit().getScreenSize());
        BufferedImage img = robot.createScreenCapture(screen);
        ImageIO.write(img, "png", new File(args.length > 0 ? args[0] : "gui.png"));
        System.out.println("GUI screenshot written");
        System.exit(0);
    }
}
