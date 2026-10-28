package dev.voltip.rn

import dev.voltip.rn.PasteOutcome.COPIED
import dev.voltip.rn.PasteOutcome.NOT_PASTED
import dev.voltip.rn.PasteOutcome.PASTE
import org.junit.Assert.assertEquals
import org.junit.Test

/** What a finished result does under 手势粘贴 (docs/dictation.md §20.12): it is pasted only into the
 *  field the gesture was armed over, while the service is connected and that field has focus. */
class BubblePasteDecisionTest {
    private val wechat = PasteTargetId("com.tencent.mm", 41)

    private fun outcome(
        requested: Boolean = true,
        connected: Boolean = true,
        captured: PasteTargetId? = wechat,
        current: PasteTargetId? = wechat,
        sameNode: Boolean = true,
    ) = BubblePasteDecision.outcome(requested, connected, captured, current, sameNode)

    @Test
    fun `the same field, still focused, gets the paste`() {
        assertEquals(PASTE, outcome())
    }

    @Test
    fun `a release without the gesture only copies`() {
        assertEquals(COPIED, outcome(requested = false))
        assertEquals(COPIED, outcome(requested = false, connected = false, captured = null, current = null, sameNode = false))
    }

    @Test
    fun `without the service the result stays on the clipboard`() {
        assertEquals(NOT_PASTED, outcome(connected = false))
    }

    @Test
    fun `no field when the gesture was armed, or none focused now, leaves it on the clipboard`() {
        assertEquals(NOT_PASTED, outcome(captured = null))
        assertEquals(NOT_PASTED, outcome(current = null, sameNode = false))
    }

    @Test
    fun `another field, window or app with focus now is not pasted into`() {
        // Another node in the same window (the user tapped a second field meanwhile).
        assertEquals(NOT_PASTED, outcome(sameNode = false))
        // The same app in another window, or another app.
        assertEquals(NOT_PASTED, outcome(current = wechat.copy(windowId = 42)))
        assertEquals(NOT_PASTED, outcome(current = PasteTargetId("com.whatsapp", 41)))
    }
}
