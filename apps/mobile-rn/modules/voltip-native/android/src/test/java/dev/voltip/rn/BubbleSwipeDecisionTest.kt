package dev.voltip.rn

import dev.voltip.rn.BubbleSwipeDecision.CANCEL
import dev.voltip.rn.BubbleSwipeDecision.COPY
import dev.voltip.rn.BubbleSwipeDecision.PASTE
import org.junit.Assert.assertEquals
import org.junit.Test

/** Letting go of a held take (docs/dictation.md §20.12): copy, paste (a swipe toward the middle of
 *  the screen, with the gesture available) or cancel (up, down or away). */
class BubbleSwipeDecisionTest {
    /** A phone at density 2.75 (a 412 dp wide Pixel): the thresholds are dp, the motion pixels. */
    private val density = 2.75f
    private val cancel = 88 * density

    private fun px(dp: Float) = dp * density

    private fun decide(dxDp: Float, dyDp: Float, dockRight: Boolean = true, paste: Boolean = true) =
        BubbleSwipeDecision.decide(px(dxDp), px(dyDp), dockRight, paste, density, cancel)

    @Test
    fun `a swipe toward the middle pastes, whichever side the bubble sits on`() {
        // Docked right, the middle is to the left; docked left, to the right.
        assertEquals(PASTE, decide(-70f, 10f, dockRight = true))
        assertEquals(PASTE, decide(70f, -10f, dockRight = false))
    }

    @Test
    fun `a swipe toward the edge does not paste`() {
        assertEquals(COPY, decide(70f, 0f, dockRight = true))
        assertEquals(COPY, decide(-70f, 0f, dockRight = false))
        assertEquals(CANCEL, decide(100f, 0f, dockRight = true))
    }

    @Test
    fun `the paste position starts at 64 dp inward and allows 48 dp up or down`() {
        assertEquals(PASTE, decide(-64f, 48f))
        assertEquals(PASTE, decide(-64f, -48f))
        assertEquals(COPY, decide(-63.9f, 0f))
        // Too far up or down for the paste position: within the cancel distance it copies, beyond
        // it cancels.
        assertEquals(COPY, decide(-70f, 49f))
        assertEquals(CANCEL, decide(-70f, 60f))
    }

    @Test
    fun `with paste available a long swipe inward still pastes rather than cancels`() {
        assertEquals(PASTE, decide(-300f, 20f))
        assertEquals(PASTE, decide(-120f, 0f))
    }

    @Test
    fun `without the gesture the same swipe copies or cancels as before`() {
        // The switch off, or its service not connected: the overlay passes paste = false.
        assertEquals(COPY, decide(-70f, 10f, paste = false))
        assertEquals(CANCEL, decide(-120f, 0f, paste = false))
    }

    @Test
    fun `up or down beyond the cancel distance cancels, with or without paste`() {
        assertEquals(CANCEL, decide(0f, -100f))
        assertEquals(CANCEL, decide(0f, 100f, paste = false))
        assertEquals(COPY, decide(0f, -88f))
        assertEquals(COPY, decide(0f, 0f))
    }

    @Test
    fun `the thresholds scale with the screen density`() {
        val dense = 4f
        assertEquals(PASTE, BubbleSwipeDecision.decide(-64f * dense, 0f, true, true, dense, 88f * dense))
        assertEquals(COPY, BubbleSwipeDecision.decide(-64f * 2.75f, 0f, true, true, dense, 88f * dense))
    }
}
