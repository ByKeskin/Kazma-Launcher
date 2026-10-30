package app.kazma.theme;

import app.kazma.theme.fx.Canvas;
import app.kazma.theme.fx.Critters;
import app.kazma.theme.fx.Scenes;
import app.kazma.theme.fx.Theme;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientLifecycleEvents;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import org.lwjgl.glfw.GLFW;
import org.lwjgl.glfw.GLFWImage;
import org.lwjgl.stb.STBImage;
import org.lwjgl.system.MemoryStack;
import org.lwjgl.system.MemoryUtil;

import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * Kazma Launcher teması (yalnızca istemci). Çizim hataları oyunu çökertmez: ilk hatada bu mod kendini susturur.
 * Tema, launcher'ın yazdığı config/kazma-theme.txt dosyasından ("obsidian" ya da "terminal") okunur.
 */
public final class KazmaTheme implements ClientModInitializer {
    public static final String ID = "kazma_theme";

    private static volatile boolean broken = false;
    private static Theme theme = Theme.OBSIDIAN;
    private static final Critters STRIP = new Critters();
    private static final Critters LOADING = new Critters();
    private static long loadingStart = 0;
    private static boolean prevDown = false;

    @Override
    public void onInitializeClient() {
        try {
            Path f = FabricLoader.getInstance().getConfigDir().resolve("kazma-theme.txt");
            if (Files.isRegularFile(f)) theme = Theme.of(Files.readString(f));
        } catch (Throwable ignored) {
        }
        STRIP.setTheme(theme);
        LOADING.setTheme(theme);
        ClientLifecycleEvents.CLIENT_STARTED.register(mc -> setIcon());
        ClientTickEvents.END_CLIENT_TICK.register(mc -> {
            if (mc.player == null && mc.level == null) setTitle();
        });
    }

    private static void fail(Throwable t) {
        broken = true;
        System.err.println("[kazma_theme] kapatıldı: " + t);
    }

    private static void setTitle() {
        try {
            long win = GLFW.glfwGetCurrentContext();
            if (win != 0L) GLFW.glfwSetWindowTitle(win, "Kazma Launcher");
        } catch (Throwable t) {
            // başlık değişmezse önemli değil
        }
    }

    private static void setIcon() {
        try (InputStream in = KazmaTheme.class.getResourceAsStream("/assets/kazma_theme/icon.png")) {
            if (in == null) return;
            byte[] bytes = in.readAllBytes();
            ByteBuffer raw = MemoryUtil.memAlloc(bytes.length);
            try {
                raw.put(bytes).flip();
                try (MemoryStack stack = MemoryStack.stackPush()) {
                    int[] w = new int[1], h = new int[1], c = new int[1];
                    ByteBuffer px = STBImage.stbi_load_from_memory(raw, w, h, c, 4);
                    if (px == null) return;
                    try {
                        GLFWImage.Buffer icons = GLFWImage.malloc(1, stack);
                        icons.position(0).width(w[0]).height(h[0]).pixels(px);
                        long win = GLFW.glfwGetCurrentContext();
                        if (win != 0L) GLFW.glfwSetWindowIcon(win, icons);
                    } finally {
                        STBImage.stbi_image_free(px);
                    }
                }
            } finally {
                MemoryUtil.memFree(raw);
            }
        } catch (Throwable t) {
            // simge değişmezse önemli değil
        }
    }

    private static Canvas canvas(GuiGraphicsExtractor g) {
        return (x1, y1, x2, y2, argb) -> g.fill(x1, y1, x2, y2, argb);
    }

    /** GLFW'den fare konumunu (arayüz koordinatında) ve sol tıklamanın yeni basılıp basılmadığını okur. {mx,my,pressedNow} */
    private static int[] mouse(int guiW) {
        long win = GLFW.glfwGetCurrentContext();
        if (win == 0L) return new int[]{-1, -1, 0};
        double[] cx = new double[1], cy = new double[1];
        int[] ww = new int[1], wh = new int[1];
        GLFW.glfwGetCursorPos(win, cx, cy);
        GLFW.glfwGetWindowSize(win, ww, wh);
        double scale = ww[0] > 0 ? (double) guiW / ww[0] : 1.0;
        boolean down = GLFW.glfwGetMouseButton(win, GLFW.GLFW_MOUSE_BUTTON_LEFT) == GLFW.GLFW_PRESS;
        int pressed = down && !prevDown ? 1 : 0;
        prevDown = down;
        return new int[]{(int) (cx[0] * scale), (int) (cy[0] * scale), pressed};
    }

    /** Menü ekranlarının üstündeki şerit (oyun dünyasında değilken). */
    public static void onScreen(GuiGraphicsExtractor g, int width) {
        if (broken) return;
        try {
            Minecraft mc = Minecraft.getInstance();
            if (mc.level != null) return;
            long now = System.currentTimeMillis();
            Scenes.strip(canvas(g), STRIP, theme, width, now);
            int[] m = mouse(width);
            if (m[2] == 1) STRIP.click(m[0], m[1], now);
        } catch (Throwable t) {
            fail(t);
        }
    }

    /** Oyun açılırken görünen yükleme ekranı. */
    public static void onLoading(GuiGraphicsExtractor g) {
        if (broken) return;
        try {
            Minecraft mc = Minecraft.getInstance();
            int w = mc.getWindow().getGuiScaledWidth(), h = mc.getWindow().getGuiScaledHeight();
            long now = System.currentTimeMillis();
            if (loadingStart == 0) loadingStart = now;
            Scenes.loading(canvas(g), LOADING, theme, w, h, loadingStart, now);
            int[] m = mouse(w);
            if (m[2] == 1) LOADING.click(m[0], m[1], now);
        } catch (Throwable t) {
            fail(t);
        }
    }
}
