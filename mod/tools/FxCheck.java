import app.kazma.theme.fx.*;
import java.util.*;

/** Oyun gerektirmeyen duman testi: javac -d /tmp/fx src/main/java/app/kazma/theme/fx/*.java tools/FxCheck.java && java -cp /tmp/fx FxCheck */
public class FxCheck {
    static int fails = 0;
    static void check(boolean ok, String what) { if (!ok) { fails++; System.out.println("BAŞARISIZ: " + what); } else System.out.println("ok: " + what); }

    static class Rec implements Canvas {
        int calls, minX = Integer.MAX_VALUE, maxX = Integer.MIN_VALUE, minY = Integer.MAX_VALUE, maxY = Integer.MIN_VALUE; boolean bad;
        public void fill(int x1, int y1, int x2, int y2, int argb) {
            calls++;
            if (x2 < x1 || y2 < y1) bad = true;
            minX = Math.min(minX, x1); maxX = Math.max(maxX, x2); minY = Math.min(minY, y1); maxY = Math.max(maxY, y2);
        }
    }

    public static void main(String[] a) {
        Theme t = Theme.of("terminal");
        check(t == Theme.TERMINAL && Theme.of(null) == Theme.OBSIDIAN && Theme.of("x") == Theme.OBSIDIAN, "tema seçimi");
        check(PixelFont.width("KAZMA", 2) == 58 && PixelFont.height(2) == 14, "yazı ölçüsü");

        Critters c = new Critters(); c.setTheme(Theme.OBSIDIAN);
        long now = 1000;
        Rec r = new Rec();
        for (int i = 0; i < 120; i++) { now += 16; Scenes.strip(r, c, Theme.OBSIDIAN, 854, now); }
        check(!r.bad && r.calls > 1000, "şerit çizimi hatasız (" + r.calls + " çağrı)");
        check(r.minX >= 0 && r.maxX <= 854 + 1 && r.minY >= 0 && r.maxY <= Scenes.STRIP_H, "şerit sınırlar içinde x " + r.minX + ".." + r.maxX + " y " + r.minY + ".." + r.maxY);

        int[] h = c.hitbox();
        check(!c.click(5000, 5000, now), "uzak tıklama kaçırır");
        check(c.click(h[0] + 3, h[1] + 3, now), "yaratığa tıklama vurur");
        check(c.isDead() && c.particleCount() == 46, "patladı, 46 parçacık");
        check(!c.click(h[0] + 3, h[1] + 3, now), "ölüyken tekrar vurulmaz");
        for (int i = 0; i < 20; i++) { now += 16; Scenes.strip(new Rec(), c, Theme.OBSIDIAN, 854, now); }
        check(c.particleCount() > 0, "parçacıklar uçuyor");
        for (int i = 0; i < 120; i++) { now += 16; Scenes.strip(new Rec(), c, Theme.OBSIDIAN, 854, now); }
        check(!c.isDead() && c.type() == 1, "yeni yaratık doğdu (tür " + c.type() + ")");
        check(c.particleCount() == 0, "parçacıklar söndü");

        Critters c2 = new Critters();
        long t0 = 5000, n2 = t0;
        float lastX = 0;
        for (int i = 0; i < 400; i++) { n2 += 16; Rec rr = new Rec(); Scenes.loading(rr, c2, Theme.TERMINAL, 1280, 720, t0, n2); if (rr.bad) { check(false, "yükleme çizimi bozuk"); break; } lastX = c2.x(); }
        check(lastX > 640 - 100 && lastX < 1280, "yaratık ilerlemeyle sağa yürüdü (x=" + lastX + ")");
        for (int[] sz : new int[][]{{320, 240}, {854, 480}, {3840, 2160}, {100, 60}}) {
            Rec rr = new Rec(); Scenes.loading(rr, new Critters(), Theme.OBSIDIAN, sz[0], sz[1], 0, 3000);
            check(!rr.bad && rr.calls > 100, "yükleme " + sz[0] + "x" + sz[1]);
            Rec r3 = new Rec(); Scenes.strip(r3, new Critters(), Theme.OBSIDIAN, sz[0], 0);
            check(!r3.bad, "şerit " + sz[0]);
        }
        System.out.println(fails == 0 ? "TÜM KONTROLLER GEÇTİ" : fails + " BAŞARISIZ");
        System.exit(fails == 0 ? 0 : 1);
    }
}
