package app.kazma.theme.mixin;

import app.kazma.theme.KazmaTheme;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(targets = "net.minecraft.client.gui.screens.LoadingOverlay")
public abstract class LoadingOverlayMixin {
    @Inject(method = "extractRenderState", at = @At("TAIL"), require = 0)
    private void kazma$loading(GuiGraphicsExtractor graphics, int mouseX, int mouseY, float delta, CallbackInfo ci) {
        KazmaTheme.onLoading(graphics);
    }
}
