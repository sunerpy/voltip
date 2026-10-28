package dev.voltip.rn

/**
 * 空闲时自动收起 (docs/dictation.md §20.12; off unless the user picks 5, 10 or 30 seconds): the
 * bubble tucks itself into its edge after it sat idle that long. Idle means shown and not tucked
 * away, no card (recording, recognition, the result or a message), no take of its own or started
 * elsewhere, and no finger on it: a press, a drag or a held take holds it out. [restart] counts
 * from now again, when something that held it out ends; the check runs again when the time is up,
 * so a state that changed meanwhile without a [restart] still holds it out.
 */
internal class BubbleAutoStow(
    private val scheduler: Scheduler,
    private val seconds: () -> Int,
    private val state: () -> State,
    private val stow: () -> Unit,
) {
    /** What the timer reads of the bubble. */
    data class State(
        val shown: Boolean,
        val stowed: Boolean,
        /** No card: not recording, recognising, showing a result or a message. */
        val idle: Boolean,
        val own: Boolean,
        val elsewhere: Boolean,
        /** A finger is on the bubble (a press, a held take, a drag). */
        val pressed: Boolean,
        val dragging: Boolean,
    )

    /** The main thread's `Handler` in the bubble; a fake clock in the tests. */
    interface Scheduler {
        fun postDelayed(task: Runnable, delayMs: Long)

        fun remove(task: Runnable)
    }

    private val task = Runnable { if (allowed(state())) stow() }

    /** Count the idle time from now; nothing is counted while the bubble may not tuck away. */
    fun restart() {
        scheduler.remove(task)
        val after = seconds()
        if (after in CHOICES && after > 0 && allowed(state())) scheduler.postDelayed(task, after * 1_000L)
    }

    fun cancel() {
        scheduler.remove(task)
    }

    companion object {
        /** The settings page's choices: off, 5, 10 or 30 seconds. */
        val CHOICES = listOf(0, 5, 10, 30)

        fun allowed(state: State): Boolean = with(state) {
            shown && !stowed && idle && !own && !elsewhere && !pressed && !dragging
        }
    }
}
