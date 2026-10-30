package app.kazma.theme.fx;

/** "KAZMA" yazısı için 5x7 piksel harfler (kendi çizimimiz; oyunun yazı tipine bağlı değil). */
public final class PixelFont {
    private PixelFont() {}

    private static String[] glyph(char c) {
        switch (c) {
            case 'K': return new String[]{"X...X", "X..X.", "X.X..", "XX...", "X.X..", "X..X.", "X...X"};
            case 'A': return new String[]{".XXX.", "X...X", "X...X", "XXXXX", "X...X", "X...X", "X...X"};
            case 'Z': return new String[]{"XXXXX", "....X", "...X.", "..X..", ".X...", "X....", "XXXXX"};
            case 'M': return new String[]{"X...X", "XX.XX", "X.X.X", "X...X", "X...X", "X...X", "X...X"};
            default: return new String[]{".....", ".....", ".....", ".....", ".....", ".....", "....."};
        }
    }

    public static int width(String text, int scale) {
        return text.isEmpty() ? 0 : (text.length() * 6 - 1) * scale;
    }

    public static int height(int scale) {
        return 7 * scale;
    }

    public static void draw(Canvas c, String text, int x, int y, int scale, int argb) {
        int cx = x;
        for (char ch : text.toCharArray()) {
            String[] g = glyph(Character.toUpperCase(ch));
            for (int r = 0; r < 7; r++) {
                for (int col = 0; col < 5; col++) {
                    if (g[r].charAt(col) == 'X') {
                        c.fill(cx + col * scale, y + r * scale, cx + (col + 1) * scale, y + (r + 1) * scale, argb);
                    }
                }
            }
            cx += 6 * scale;
        }
    }
}
