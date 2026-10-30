package app.kazma.theme.fx;

import java.util.ArrayList;
import java.util.List;
import java.util.Random;

/**
 * Küçük blok yaratıklar: zeminde yürür, tıklanınca patlar, biraz sonra başka biri doğar.
 * Yaratıklar Kazma'ya özgü tasarımlardır. Tüm durum burada; çizim {@link Canvas} üzerinden yapılır.
 */
public final class Critters {
    private static final String[][] SHAPES = {
        {"  bbbbb  ", " bbbbbbb ", "bbbbbbbbb", "bbgbbbgbb", "bbgbbbgbb", "bbbbbbbbb", "bbbddbbbb", " bbbbbbb ", "  d   d  "},
        {"   aaa   ", "  aaaaa  ", " aawaawaa", " aaaaaaa ", " aabbbaa ", "  aaaaa  ", " aa   aa ", " aa   aa ", "dd     dd"},
        {" dddddd  ", "dbbbbbbd ", "dbgbbgbd ", "dbbbbbbd ", " dbbbbd  ", " dbddbd  ", " dd  dd  ", " d    d  ", "dd    dd "},
    };

    private static final class Particle {
        float x, y, vx, vy, life; int color; int size;
    }

    private final Random rnd = new Random(12345);
    private final List<Particle> parts = new ArrayList<>();
    private Theme theme = Theme.OBSIDIAN;
    private int type = 0;
    private float x = 8;
    private int dir = 1;
    private long deadAt = 0;
    private long bornAt = 0;
    private long last = 0;
    private float shake = 0;
    private int px = 2;
    private int groundY = 0;

    public void setTheme(Theme t) { this.theme = t; }
    public int particleCount() { return parts.size(); }
    public boolean isDead() { return deadAt != 0; }
    public int type() { return type; }
    public float x() { return x; }

    private int body(char ch) {
        switch (ch) {
            case 'b': return theme.body;
            case 'd': return theme.panel;
            case 'g': return theme.accent;
            case 'a': return theme.accent;
            case 'w': return theme.ink;
            default: return theme.body;
        }
    }

    /** Yaratığın şu anki çarpışma kutusu: {x, y, w, h}. */
    public int[] hitbox() {
        return new int[]{Math.round(x), groundY - 9 * px, 9 * px, 9 * px};
    }

    /** Tıklama (kullanıcı koordinatında). Yaratığa değdiyse patlatır. */
    public boolean click(int mx, int my, long now) {
        if (deadAt != 0) return false;
        int[] h = hitbox();
        int pad = 3 * px;
        if (mx >= h[0] - pad && mx <= h[0] + h[2] + pad && my >= h[1] - pad && my <= h[1] + h[3] + pad) {
            explode(now);
            return true;
        }
        return false;
    }

    public void explode(long now) {
        int[] h = hitbox();
        int[] cols = {theme.accent, theme.ink, theme.body, theme.accent};
        for (int i = 0; i < 46; i++) {
            double ang = rnd.nextDouble() * Math.PI * 2;
            double sp = 30 + rnd.nextDouble() * 110;
            Particle p = new Particle();
            p.x = h[0] + h[2] / 2f; p.y = h[1] + h[3] / 2f;
            p.vx = (float) (Math.cos(ang) * sp); p.vy = (float) (Math.sin(ang) * sp - 45);
            p.life = 0.7f + rnd.nextFloat() * 0.6f; p.color = cols[i % cols.length];
            p.size = rnd.nextFloat() < 0.3f ? px : Math.max(1, px / 2);
            parts.add(p);
        }
        deadAt = now;
        shake = 0.25f;
    }

    /**
     * Bir kare çizer. {@code left..right} yürüme aralığı, {@code ground} zemin çizgisinin y'si, {@code target} 0..1 hedef konum
     * (ilerleme çubuğu için) ya da negatifse kendi kendine gezinir.
     */
    public void frame(Canvas c, int left, int right, int ground, int pixel, long now, float target) {
        this.px = Math.max(1, pixel);
        this.groundY = ground;
        float dt = last == 0 ? 0.016f : Math.min(0.05f, (now - last) / 1000f);
        last = now;
        float span = Math.max(1, right - left - 9 * px);
        float want = target >= 0 ? left + span * Math.min(1f, target) : left + span * (float) (0.5 + 0.5 * Math.sin(now / 2600.0));
        if (deadAt == 0) {
            float dx = want - x;
            dir = dx >= 0 ? 1 : -1;
            x += dx * Math.min(1f, dt * 3.2f);
            boolean moving = Math.abs(dx) > 1.5f;
            int bob = moving && ((now / 160) % 2 == 0) ? 0 : (moving ? Math.max(1, px / 2) : 0);
            float age = Math.min(1f, (now - bornAt) / 220f);
            drawShape(c, SHAPES[type], Math.round(x), ground - 9 * px - bob, dir < 0, 0.6f + 0.4f * age);
        } else if (now - deadAt > 900) {
            deadAt = 0;
            type = (type + 1) % SHAPES.length;
            bornAt = now;
        }
        for (int i = parts.size() - 1; i >= 0; i--) {
            Particle p = parts.get(i);
            p.life -= dt;
            if (p.life <= 0) { parts.remove(i); continue; }
            p.vy += 260 * dt; p.x += p.vx * dt; p.y += p.vy * dt;
            if (p.y > ground) { p.y = ground; p.vy *= -0.35f; p.vx *= 0.7f; }
            int a = Math.round(255 * Math.min(1f, p.life * 2));
            c.fill(Math.round(p.x), Math.round(p.y), Math.round(p.x) + p.size, Math.round(p.y) + p.size, Theme.withAlpha(p.color, a));
        }
        if (shake > 0) shake = Math.max(0, shake - dt);
    }

    private void drawShape(Canvas c, String[] sh, int ox, int oy, boolean flip, float squish) {
        int h = Math.max(1, Math.round(px * squish));
        for (int r = 0; r < 9; r++) {
            for (int col = 0; col < 9; col++) {
                char ch = sh[r].charAt(flip ? 8 - col : col);
                if (ch == ' ') continue;
                int y = oy + Math.round(r * px * squish) + Math.round((1 - squish) * 9 * px);
                c.fill(ox + col * px, y, ox + (col + 1) * px, y + h, body(ch));
            }
        }
    }
}
