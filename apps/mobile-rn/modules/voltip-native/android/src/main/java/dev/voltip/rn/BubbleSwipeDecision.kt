package dev.voltip.rn

import kotlin.math.abs
import kotlin.math.hypot

/**
 * What letting go of a held take does, from how far the finger that started it moved
 * (docs/dictation.md §20.12): it copies the result (the default); it pastes the result into the text
 * field the gesture was armed over when the finger moved at least [PASTE_INWARD_DP] toward the
 * middle of the screen and stayed within [PASTE_SLACK_DP] of the press's height, with the gesture
 * available (switched on, its accessibility service connected); otherwise it cancels once the
 * finger is farther than the cancel distance from the press. With paste available a sideways
 * swipe inward therefore pastes rather than cancels: cancelling is up, down or away.
 */
internal enum class BubbleSwipeDecision {
    COPY,
    PASTE,
    CANCEL,
    ;

    companion object {
        const val PASTE_INWARD_DP = 64f
        const val PASTE_SLACK_DP = 48f

        fun decide(
            dx: Float,
            dy: Float,
            dockRight: Boolean,
            pasteAvailable: Boolean,
            density: Float,
            cancelDistancePx: Float,
        ): BubbleSwipeDecision {
            val inward = if (dockRight) -dx else dx
            if (pasteAvailable && inward >= PASTE_INWARD_DP * density && abs(dy) <= PASTE_SLACK_DP * density) {
                return PASTE
            }
            return if (hypot(dx, dy) > cancelDistancePx) CANCEL else COPY
        }
    }
}
