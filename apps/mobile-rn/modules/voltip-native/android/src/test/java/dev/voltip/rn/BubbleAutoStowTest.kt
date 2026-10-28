package dev.voltip.rn

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * 空闲时自动收起 (docs/dictation.md §20.12): the bubble tucks itself in once it sat idle for the
 * chosen time, never while a finger, a take, a card or a take started elsewhere holds it out, and
 * never with the option off. A fake clock stands in for the main thread's Handler.
 */
class BubbleAutoStowTest {
    private class FakeScheduler : BubbleAutoStow.Scheduler {
        var now = 0L
        private val tasks = mutableListOf<Pair<Long, Runnable>>()

        override fun postDelayed(task: Runnable, delayMs: Long) {
            tasks += (now + delayMs) to task
        }

        override fun remove(task: Runnable) {
            tasks.removeAll { it.second === task }
        }

        /** Run what falls due within the next [ms], in order. */
        fun advance(ms: Long) {
            val until = now + ms
            while (true) {
                val next = tasks.filter { it.first <= until }.minByOrNull { it.first } ?: break
                tasks.remove(next)
                now = next.first
                next.second.run()
            }
            now = until
        }
    }

    private val idle = BubbleAutoStow.State(
        shown = true,
        stowed = false,
        idle = true,
        own = false,
        elsewhere = false,
        pressed = false,
        dragging = false,
    )

    private val clock = FakeScheduler()
    private var seconds = 5
    private var state = idle
    private var stows = 0
    private val autoStow = BubbleAutoStow(clock, { seconds }, { state }, {
        stows += 1
        // What stow() does to the bubble.
        state = state.copy(stowed = true)
    })

    @Test
    fun `an idle bubble tucks itself in once after five seconds`() {
        autoStow.restart()
        clock.advance(4_999)
        assertEquals(0, stows)
        clock.advance(1)
        assertEquals(1, stows)
        // Tucked away: nothing more to do, even when asked to count again.
        autoStow.restart()
        clock.advance(60_000)
        assertEquals(1, stows)
    }

    @Test
    fun `ten and thirty seconds count as long`() {
        seconds = 10
        autoStow.restart()
        clock.advance(9_999)
        assertEquals(0, stows)
        clock.advance(1)
        assertEquals(1, stows)
        state = idle
        seconds = 30
        autoStow.restart()
        clock.advance(29_999)
        assertEquals(1, stows)
        clock.advance(1)
        assertEquals(2, stows)
    }

    @Test
    fun `switched off it never tucks the bubble in`() {
        seconds = 0
        autoStow.restart()
        clock.advance(3_600_000)
        assertEquals(0, stows)
        // A value the settings page does not offer counts as off.
        seconds = 7
        autoStow.restart()
        clock.advance(3_600_000)
        assertEquals(0, stows)
    }

    /** Every state that holds the bubble out: counting does not start in it, and a timer that falls
     *  due while the bubble is in it does nothing. */
    private val heldOut = mapOf(
        "a finger pressing the bubble" to idle.copy(pressed = true),
        "a drag" to idle.copy(pressed = true, dragging = true),
        "its own take recording" to idle.copy(pressed = true, own = true, idle = false),
        "its own take being recognised" to idle.copy(own = true, idle = false),
        "the result card" to idle.copy(idle = false),
        "a message on the card" to idle.copy(idle = false),
        "a take started elsewhere" to idle.copy(elsewhere = true),
        "already tucked away" to idle.copy(stowed = true),
        "not on the screen" to idle.copy(shown = false),
    )

    @Test
    fun `nothing that holds the bubble out lets it tuck in`() {
        for ((what, held) in heldOut) {
            state = held
            autoStow.restart()
            clock.advance(60_000)
            assertEquals(what, 0, stows)
            // Counting began while idle, and the state changed before the time was up.
            state = idle
            autoStow.restart()
            clock.advance(2_000)
            state = held
            clock.advance(60_000)
            assertEquals(what, 0, stows)
            autoStow.cancel()
        }
    }

    @Test
    fun `a touch starts the count again from its release`() {
        autoStow.restart()
        clock.advance(3_000)
        // The press cancels the count; the release (a tap, a short drag) restarts it.
        state = idle.copy(pressed = true)
        autoStow.cancel()
        clock.advance(1_000)
        state = idle
        autoStow.restart()
        clock.advance(4_999)
        assertEquals(0, stows)
        clock.advance(1)
        assertEquals(1, stows)
        assertEquals(9_000L, clock.now)
    }

    @Test
    fun `the end of a take, its card or a take elsewhere starts the count`() {
        // Recording, then its result card for five seconds, then idle: counted from the card's end.
        state = idle.copy(own = true, idle = false)
        autoStow.restart()
        clock.advance(8_000)
        state = idle.copy(idle = false)
        autoStow.restart()
        clock.advance(5_000)
        state = idle
        autoStow.restart()
        clock.advance(4_999)
        assertEquals(0, stows)
        clock.advance(1)
        assertEquals(1, stows)
    }
}
