package dev.voltip.rn

/**
 * The app and the window of a text field the paste gesture was armed over. With the node itself
 * (`AccessibilityNodeInfo.equals`: the same source view in the same window) it tells whether the
 * field focused when the result arrives is that one; where the field is on the screen does not
 * count, since the keyboard and the layout move it.
 */
internal data class PasteTargetId(val packageName: String, val windowId: Int)

/** What the bubble does with a finished result (docs/dictation.md §20.12). */
internal enum class PasteOutcome {
    /** Copied only: the user let go without the paste gesture. */
    COPIED,

    /** Asked the field to paste: the gesture was armed over this field and it still has focus. */
    PASTE,

    /** The gesture was asked for, but the result stays on the clipboard: no service, no field
     *  when the gesture was armed, or another field (or none) has focus now. */
    NOT_PASTED,
}

internal object BubblePasteDecision {
    /**
     * [requested]: the finger let go in the paste position; [connected]: the accessibility service
     * is connected; [captured]: the field focused when the gesture was first armed; [current]: the
     * field focused now; [sameNode]: both are the same node.
     */
    fun outcome(
        requested: Boolean,
        connected: Boolean,
        captured: PasteTargetId?,
        current: PasteTargetId?,
        sameNode: Boolean,
    ): PasteOutcome = when {
        !requested -> PasteOutcome.COPIED
        !connected || captured == null || current == null -> PasteOutcome.NOT_PASTED
        captured != current || !sameNode -> PasteOutcome.NOT_PASTED
        else -> PasteOutcome.PASTE
    }
}
