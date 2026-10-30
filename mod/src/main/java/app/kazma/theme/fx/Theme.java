package app.kazma.theme.fx;

/** Launcher temasıyla aynı iki renk düzeni. */
public final class Theme {
    public final int bg, panel, line, ink, accent, body;

    private Theme(int bg, int panel, int line, int ink, int accent, int body) {
        this.bg = bg; this.panel = panel; this.line = line; this.ink = ink; this.accent = accent; this.body = body;
    }

    public static final Theme OBSIDIAN = new Theme(0xFF080C14, 0xFF121A26, 0xFF223044, 0xFFE8EEF2, 0xFF4EE0C4, 0xFF9AA7B4);
    public static final Theme TERMINAL = new Theme(0xFF070503, 0xFF120D05, 0xFF3A2A0A, 0xFFFFE2A8, 0xFFFFB028, 0xFFC9964A);

    public static Theme of(String name) {
        return "terminal".equalsIgnoreCase(name == null ? "" : name.trim()) ? TERMINAL : OBSIDIAN;
    }

    public static int withAlpha(int argb, int a) {
        return (argb & 0x00FFFFFF) | ((a & 0xFF) << 24);
    }
}
