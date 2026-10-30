package app.kazma.theme.fx;

/** Ekran düzenleri: menülerin üstündeki ince Kazma şeridi ve oyun açılırken görünen yükleme sahnesi. */
public final class Scenes {
    private Scenes() {}

    public static final int STRIP_H = 26;

    /** Menü şeridi. Yürüme alanı logodan sağa kadar. */
    public static void strip(Canvas c, Critters crit, Theme t, int w, long now) {
        c.fill(0, 0, w, STRIP_H, Theme.withAlpha(t.bg, 0xD8));
        c.fill(0, STRIP_H - 1, w, STRIP_H, Theme.withAlpha(t.accent, 0xB0));
        int scale = 2;
        PixelFont.draw(c, "KAZMA", 8, (STRIP_H - PixelFont.height(scale)) / 2 - 1, scale, t.accent);
        int logoR = 8 + PixelFont.width("KAZMA", scale) + 12;
        crit.frame(c, logoR, w - 8, STRIP_H - 3, 2, now, -1f);
    }

    /** Yükleme sahnesi: tam ekran, ortada logo, altında zaman bazlı ilerleme çubuğu ve üstünde yürüyen yaratık. */
    public static void loading(Canvas c, Critters crit, Theme t, int w, int h, long startedMs, long now) {
        c.fill(0, 0, w, h, t.bg);
        int scale = Math.max(3, Math.min(10, w / 90));
        int lw = PixelFont.width("KAZMA", scale), lh = PixelFont.height(scale);
        int lx = (w - lw) / 2, ly = h / 2 - lh - 16;
        PixelFont.draw(c, "KAZMA", lx + scale / 2, ly + scale / 2, scale, Theme.withAlpha(t.accent, 0x40));
        PixelFont.draw(c, "KAZMA", lx, ly, scale, t.accent);
        int barW = Math.min(w - 40, Math.max(160, w / 2)), bx = (w - barW) / 2, by = h / 2 + 22, bh = Math.max(4, scale);
        float p = 0.95f * (1f - (float) Math.exp(-(now - startedMs) / 2500.0));
        c.fill(bx - 1, by - 1, bx + barW + 1, by + bh + 1, t.line);
        c.fill(bx, by, bx + barW, by + bh, t.panel);
        c.fill(bx, by, bx + Math.round(barW * p), by + bh, t.accent);
        crit.frame(c, bx, bx + barW, by - 2, Math.max(2, scale / 2), now, p);
    }
}
