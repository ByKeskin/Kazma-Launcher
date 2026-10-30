package app.kazma.theme.fx;

/** Minecraft'tan bağımsız çizim yüzeyi: dolu dikdörtgen (ARGB). Testler sahte bir yüzey kullanır. */
public interface Canvas {
    void fill(int x1, int y1, int x2, int y2, int argb);
}
