package dev.voltip.rn

import kotlin.math.abs
import kotlin.math.hypot

/** Classifies the motion of the SAME press used to start the recording. */
internal enum class BubbleSwipeDecision { COPY, PASTE, CANCEL;
    companion object {
        fun decide(
            dx: Float, dy: Float, dockRight: Boolean, pasteEnabled: Boolean,
            density: Float, cancelDistancePx: Float,
        ): BubbleSwipeDecision {
            val inward = if (dockRight) -dx else dx
            if (pasteEnabled && inward >= 64f * density && abs(dy) <= 48f * density) {
                return PASTE
            }
            return if (hypot(dx, dy) > cancelDistancePx) CANCEL else COPY
        }
    }
}
