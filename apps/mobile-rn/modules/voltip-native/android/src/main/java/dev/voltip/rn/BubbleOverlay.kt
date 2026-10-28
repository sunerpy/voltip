package dev.voltip.rn

import android.Manifest
import android.annotation.SuppressLint
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.PixelFormat
import android.graphics.Rect
import android.graphics.RectF
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.text.TextUtils
import android.util.Log
import android.util.TypedValue
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import dev.voltip.rn.uniffi.ShellException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.async
import kotlinx.coroutines.launch
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/**
 * The floating bubble itself (docs/dictation.md §20.12; Penpot 「用量与额度」 boards 17a–17d): a round
 * talk button docked at an edge of the screen over other apps, and beside it a card for the take
 * it started: the recording with its level and time, the recognition, the copied result with
 * 再说一次 (and 不复制 when the user asked for it; shown [RESULT_MS]), or why it did not finish. Its
 * takes are the phone's own (`dictation_start`), recognised as the 说话 tab's are and copied to the
 * clipboard by the shell's injector; the card follows the core's dictation events for the take the
 * bubble started, and the bubble waits while a take started elsewhere runs.
 *
 * Holding (`mode` hold): a press that stays put for [HOLD_DELAY_MS] starts the take, releasing
 * finishes it, and a release [cancelDistance] away from the press cancels it; a press that moves
 * first drags the bubble, which docks at the nearer edge when let go. Tapping (`mode` tap): a tap
 * starts, the next finishes, the card's 取消 discards. The texts are the app's (`BubblePrefs`).
 *
 * Tucked away (2026-10-28): a drag let go with the finger within [EDGE_ZONE_DP] of an edge tucks
 * the bubble into that edge, where only a small handle shows; on the way the bubble sinks half into
 * the edge, faded, so the release is announced. A tap on the handle, or a pull inward, brings the
 * bubble back; dragged up or down, the handle moves along its edge. With 空闲时自动收起 chosen
 * ([BubbleAutoStow]) the bubble tucks itself in after sitting idle that long.
 *
 * 手势粘贴 (optional, [BubblePasteService]): while the take is held, a swipe toward the middle of the
 * screen ([BubbleSwipeDecision.PASTE]) arms a paste; the focused text field is looked up the first
 * time the paste position is reached, and letting go there pastes the result into that field if it
 * still has focus when the result arrives ([BubblePasteDecision]); otherwise the result stays on the
 * clipboard as always. Without the service connected the gesture is not offered at all.
 */
@SuppressLint("ClickableViewAccessibility", "SetTextI18n")
internal class BubbleOverlay(
    private val context: Context,
    private val prefs: BubblePrefs,
    private val scope: CoroutineScope,
) : VoltipRuntime.Listener {
    private enum class Phase { IDLE, LISTENING, PROCESSING, RESULT, MESSAGE }

    private val main = Handler(Looper.getMainLooper())
    private val windows = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
    private val density = context.resources.displayMetrics.density
    private val touchSlop = ViewConfiguration.get(context).scaledTouchSlop
    private val cancelDistance = dp(88)

    private val root = LinearLayout(context)
    private val bubble = FrameLayout(context)
    private val bubbleFace = GradientDrawable()
    private val icon = ImageView(context)
    private val card = LinearLayout(context)
    /** What shows of a tucked-away bubble: a tab on the edge, in a larger touch area. */
    private val handle = FrameLayout(context)
    private val handleTab = FrameLayout(context)
    private val handleFace = GradientDrawable()
    private val params = WindowManager.LayoutParams(
        WindowManager.LayoutParams.WRAP_CONTENT,
        WindowManager.LayoutParams.WRAP_CONTENT,
        WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
        WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,
        PixelFormat.TRANSLUCENT,
    ).apply {
        // The window's name in `dumpsys window`, where the device acceptance reads its frame
        // (apps/mobile-rn/devicefarm/acceptance.sh); users never see it.
        title = WINDOW_TITLE
    }
    private var shown = false

    private var phase = Phase.IDLE
    /** The take running is the bubble's own. */
    private var own = false
    /** A take started elsewhere (the app's 说话 tab) runs: the bubble waits. */
    private var elsewhere = false
    /** The take ends with a tap on the bubble (tap mode, or 再说一次). */
    private var tapTake = false
    /** A held take: the finger that started it is still down. */
    private var holding = false
    /** The held finger is [cancelDistance] away from where it pressed: letting go cancels. */
    private var slidAway = false
    /** 手势粘贴: the held finger is in the paste position ([BubbleSwipeDecision.PASTE]). */
    private var pasteArmed = false
    /** The take was let go in the paste position: its result goes to [pasteTarget]. */
    private var pasteRequested = false
    /** The text field focused when this take's paste position was first reached, looked up off
     *  the main thread (the query goes to that field's app). */
    private var pasteTarget: Deferred<BubblePasteService.Target?>? = null
    /** The result card's first line: copied, paste requested, or not pasted. */
    private var resultLabel = ""
    /** A finger is on the bubble, from its press to its release. */
    private var pressed = false
    private val autoStow = BubbleAutoStow(
        object : BubbleAutoStow.Scheduler {
            override fun postDelayed(task: Runnable, delayMs: Long) {
                main.postDelayed(task, delayMs)
            }

            override fun remove(task: Runnable) {
                main.removeCallbacks(task)
            }
        },
        seconds = { prefs.autoStowSeconds },
        state = { BubbleAutoStow.State(shown, stowed, phase == Phase.IDLE, own, elsewhere, pressed, dragging) },
        stow = { stow(dockRight) },
    )
    private var startedAt = 0L
    private var stage = ""
    private var resultText = ""
    private var message = ""
    private var meter: Long? = null
    private val levels = FloatArray(WAVE_BARS)

    /** The edge the bubble is docked at, and the one its card was last arranged for. */
    private var dockRight = true
    private var arrangedRight: Boolean? = null
    /** Tucked into [dockRight]'s edge: only [handle] shows. */
    private var stowed = false
    /** Let go now, the drag going on tucks the bubble into [stowRight]'s edge. */
    private var stowArmed = false
    private var stowRight = true
    /** The screen reader's action that tucks the bubble away. */
    private var stowAction: Int? = null

    private var downX = 0f
    private var downY = 0f
    private var dragging = false
    private var dragOffsetX = 0f
    private var dragOffsetY = 0f
    private var handleDownX = 0f
    private var handleDownY = 0f
    private var handleStartY = 0
    private var handleMoved = false
    /** The handle's drag goes up or down its edge, not out of it. */
    private var handleAlong = false
    private val press = Runnable { startTake(tap = false, held = true) }
    /** The recording's time, counted on its card in place: the card is rebuilt only when what it
     *  says changes, so a finger on 取消 never loses the chip under it. */
    private val tick = object : Runnable {
        override fun run() {
            if (phase != Phase.LISTENING) return
            timer?.text = elapsed(if (startedAt > 0) System.currentTimeMillis() - startedAt else 0L)
            main.postDelayed(this, TICK_MS)
        }
    }
    private val settle = Runnable { toIdle() }

    /** Put the bubble on the screen where it was last let go. */
    fun show() {
        if (shown) return
        buildViews()
        val (right, y) = prefs.dock
        dockRight = right
        stowed = prefs.stowed
        params.gravity = Gravity.TOP or if (right) Gravity.END else Gravity.START
        val top = (y * screenHeight()).roundToInt().coerceIn(0, max(0, screenHeight() - dp(BUBBLE_DP)))
        if (stowed) {
            padForHandle()
            params.x = 0
            params.y = handleTop(top)
        } else {
            padForBubble()
            params.x = dp(EDGE_MARGIN_DP)
            params.y = top
        }
        try {
            windows.addView(root, params)
            shown = true
        } catch (e: RuntimeException) {
            // No permission any more (revoked while it ran): nothing to show.
            Log.w(TAG, "the bubble could not be shown: ${e.message}")
        }
        VoltipRuntime.addListener(this)
        render()
        autoStow.restart()
    }

    /** The options or texts changed. */
    fun refresh() {
        main.post {
            bubble.contentDescription = prefs.string("label", "")
            handle.contentDescription = prefs.string("expand", "")
            offerStow()
            // Another screen size: the window stays on the screen.
            params.y = params.y.coerceIn(0, max(0, screenHeight() - dp(if (stowed) HANDLE_H_DP else BUBBLE_DP)))
            render()
            autoStow.restart()
        }
    }

    /** Take the bubble off the screen; a take it started is cancelled. */
    fun close() {
        VoltipRuntime.removeListener(this)
        main.removeCallbacksAndMessages(null)
        bubble.animate().cancel()
        if (own) invoke("dictation_cancel")
        stopMeter()
        if (shown) {
            try {
                windows.removeView(root)
            } catch (_: RuntimeException) {
                // Already gone.
            }
            shown = false
        }
    }

    // ---- the core's events (on the shell's threads) ----

    override fun onEvent(json: String) {
        if (!json.contains("\"type\":\"dictation\"")) return
        val event = try {
            JSONObject(json)
        } catch (_: Exception) {
            return
        }
        if (event.optString("type") != "dictation") return
        val state = event.optJSONObject("phase") ?: return
        main.post { onPhase(state) }
    }

    override fun onChannel(channel: ULong, json: String) {
        if (channel != CHANNEL) return
        val rms = try {
            JSONObject(json).optDouble("rms_dbfs", -60.0)
        } catch (_: Exception) {
            return
        }
        main.post {
            System.arraycopy(levels, 1, levels, 0, WAVE_BARS - 1)
            levels[WAVE_BARS - 1] = ((rms + 60.0) / 60.0).toFloat().coerceIn(0f, 1f)
            wave?.invalidate()
        }
    }

    private fun onPhase(state: JSONObject) {
        val name = state.optString("phase")
        if (!own) {
            val before = elsewhere
            elsewhere = name == "listening" || name == "processing"
            render()
            // The take started elsewhere ended: the idle time counts from now.
            if (before && !elsewhere) autoStow.restart()
            return
        }
        when (name) {
            "listening" -> {
                phase = Phase.LISTENING
                if (state.optBoolean("ready")) startedAt = state.optLong("started_at")
                startMeter()
                main.removeCallbacks(tick)
                main.post(tick)
            }
            "processing" -> {
                stopMeter()
                phase = Phase.PROCESSING
                stage = state.optString("stage")
            }
            "done" -> {
                stopMeter()
                own = false
                val text = state.optString("text")
                val requested = pasteRequested
                val target = pasteTarget.takeIf { requested }
                // Handed over to the paste below; otherwise the lookup is no longer needed.
                if (requested) pasteTarget = null
                clearPaste()
                if (!requested) {
                    showResult(text, prefs.string("copied", ""))
                    return
                }
                // 手势粘贴: the field is asked off the main thread (the request goes to its app); the
                // card keeps 正在识别… for the few milliseconds that takes. No take starts meanwhile.
                scope.launch {
                    val pasted = runCatching { BubblePasteService.pasteText(text, target?.await()) }.getOrDefault(false)
                    // What happened, not what was said: the text is never logged.
                    Log.i(TAG, if (pasted) "paste requested in the field the gesture was armed over" else "not pasted; the result stays on the clipboard")
                    main.post { showResult(text, prefs.string(if (pasted) "pasted" else "pasteFallback", "")) }
                }
                return
            }
            "failed" -> {
                stopMeter()
                own = false
                clearPaste()
                showMessage(
                    when (state.optString("code")) {
                        "no_speech" -> prefs.string("noSpeech", "")
                        "quota" -> prefs.string("quota", "")
                        else -> failed(state.optString("message"))
                    },
                )
                return
            }
            "cancelled" -> {
                stopMeter()
                own = false
                clearPaste()
                toIdle()
                return
            }
            else -> return
        }
        render()
    }

    /** The take's result on its card (or the check alone, a moment), under [label]. */
    private fun showResult(text: String, label: String) {
        resultText = text
        resultLabel = label
        haptic(confirm = true)
        phase = Phase.RESULT
        settleIn(if (prefs.showResult) RESULT_MS else CHECK_MS)
        render()
    }

    /** No paste for this take: a failure, a cancel, a refusal, or the paste was handed over. */
    private fun clearPaste() {
        pasteTarget?.cancel()
        pasteTarget = null
        pasteRequested = false
        pasteArmed = false
    }

    // ---- takes ----

    private fun startTake(tap: Boolean, held: Boolean) {
        if (own || elsewhere || phase == Phase.PROCESSING) return
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            showMessage(prefs.string("microphone", ""))
            return
        }
        main.removeCallbacks(settle)
        autoStow.cancel()
        clearPaste()
        resultLabel = ""
        own = true
        tapTake = tap
        holding = held
        slidAway = false
        startedAt = 0L
        levels.fill(0f)
        phase = Phase.LISTENING
        haptic(confirm = false)
        render()
        main.removeCallbacks(tick)
        main.post(tick)
        scope.launch {
            val shell = VoltipRuntime.shell
            if (shell == null) {
                main.post { refuse("") }
                return@launch
            }
            try {
                shell.invoke("dictation_start", "")
            } catch (e: ShellException) {
                main.post { refuse(e.message ?: "") }
            }
        }
    }

    /** The core refused the take before it began. */
    private fun refuse(reason: String) {
        own = false
        holding = false
        clearPaste()
        showMessage(failed(reason))
    }

    private fun stopTake() {
        holding = false
        invoke("dictation_stop")
    }

    private fun cancelTake() {
        holding = false
        haptic(confirm = false)
        invoke("dictation_cancel")
    }

    /** `command` without waiting for its answer; it reaches the shell even as the bubble closes. */
    private fun invoke(command: String, args: String = "") {
        VoltipRuntime.fire(command, args)
    }

    /** The take's level, from its own capture (the meter never opens the microphone). */
    private fun startMeter() {
        if (meter != null) return
        meter = -1L
        scope.launch {
            try {
                val answer = VoltipRuntime.shell?.invoke(
                    "audio_meter_start",
                    JSONObject().put("onFrame", JSONObject().put(CHANNEL_KEY, CHANNEL.toLong())).toString(),
                ) ?: return@launch
                val id = answer.trim().toLongOrNull() ?: return@launch
                main.post {
                    if (meter == -1L) meter = id else stopMeterId(id)
                }
            } catch (e: ShellException) {
                Log.w(TAG, "no level for the bubble: ${e.message}")
            }
        }
    }

    private fun stopMeter() {
        val id = meter ?: return
        meter = null
        if (id >= 0) stopMeterId(id)
    }

    private fun stopMeterId(id: Long) {
        invoke("audio_meter_stop", JSONObject().put("id", id).toString())
    }

    // ---- the card's states ----

    private fun showMessage(text: String) {
        phase = Phase.MESSAGE
        message = text
        render()
        settleIn(MESSAGE_MS)
    }

    private fun settleIn(ms: Long) {
        main.removeCallbacks(settle)
        main.postDelayed(settle, ms)
    }

    private fun toIdle() {
        main.removeCallbacks(settle)
        phase = Phase.IDLE
        holding = false
        slidAway = false
        pasteArmed = false
        render()
        autoStow.restart()
    }

    /** 不复制: the result leaves the clipboard (API 28 clears it; before that it is replaced by
     *  nothing). */
    private fun discard() {
        val manager = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                manager.clearPrimaryClip()
            } else {
                manager.setPrimaryClip(ClipData.newPlainText("", ""))
            }
        } catch (e: RuntimeException) {
            Log.w(TAG, "the clipboard was not cleared: ${e.message}")
        }
        toIdle()
    }

    private fun failed(reason: String): String =
        prefs.string("failed", "{message}").replace("{message}", reason.replace(MACHINE_PREFIX, ""))

    // ---- touch ----

    private val touch = View.OnTouchListener { _, e ->
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                pressed = true
                autoStow.cancel()
                downX = e.rawX
                downY = e.rawY
                dragging = false
                main.removeCallbacks(press)
                if (prefs.mode != BubblePrefs.MODE_TAP && !own && !elsewhere && phase != Phase.PROCESSING) {
                    main.postDelayed(press, HOLD_DELAY_MS)
                }
            }
            MotionEvent.ACTION_MOVE -> {
                val dx = e.rawX - downX
                val dy = e.rawY - downY
                if (holding) {
                    // 手势粘贴 only with its service connected: without it the swipe copies or cancels.
                    val pasteAvailable = prefs.pasteEnabled && BubblePasteService.connected
                    val choice = BubbleSwipeDecision.decide(dx, dy, dockRight, pasteAvailable, density, cancelDistance.toFloat())
                    val armed = choice == BubbleSwipeDecision.PASTE
                    val away = choice == BubbleSwipeDecision.CANCEL
                    if (armed != pasteArmed || away != slidAway) {
                        if (armed && !pasteArmed) {
                            haptic(confirm = true)
                            // The focused field, looked up once per take: when the paste position is
                            // first reached, never before (docs/dictation.md §20.12).
                            if (pasteTarget == null) pasteTarget = scope.async { BubblePasteService.captureTarget() }
                        }
                        pasteArmed = armed
                        slidAway = away
                        render()
                    }
                } else if (!dragging && hypot(dx, dy) > touchSlop && !(own && phase == Phase.LISTENING)) {
                    main.removeCallbacks(press)
                    beginDrag(e)
                }
                if (dragging) {
                    dragTo(e)
                    armStow(e.rawX)
                }
            }
            MotionEvent.ACTION_UP -> {
                pressed = false
                main.removeCallbacks(press)
                when {
                    dragging -> if (stowArmed) stow(stowRight) else dock()
                    holding -> when {
                        pasteArmed -> {
                            pasteRequested = true
                            stopTake()
                        }
                        slidAway -> cancelTake()
                        else -> stopTake()
                    }
                    else -> tapped()
                }
                dragging = false
                autoStow.restart()
            }
            MotionEvent.ACTION_CANCEL -> {
                pressed = false
                main.removeCallbacks(press)
                if (dragging) dock()
                if (holding) cancelTake()
                pasteArmed = false
                dragging = false
                autoStow.restart()
            }
        }
        true
    }

    private fun tapped() {
        when {
            own && phase == Phase.LISTENING && tapTake -> stopTake()
            own || elsewhere || phase == Phase.PROCESSING -> Unit
            prefs.mode == BubblePrefs.MODE_TAP -> startTake(tap = true, held = false)
            else -> showMessage(prefs.string("holdHint", ""))
        }
    }

    /** From here the window follows the finger by its top-left corner. */
    private fun beginDrag(e: MotionEvent) {
        dragging = true
        val location = IntArray(2)
        root.getLocationOnScreen(location)
        val bubbleAt = IntArray(2)
        bubble.getLocationOnScreen(bubbleAt)
        dragOffsetX = e.rawX - bubbleAt[0] + root.paddingLeft
        dragOffsetY = e.rawY - location[1]
        // While dragged the card is out of the way and the window is the bubble alone.
        if (phase == Phase.RESULT || phase == Phase.MESSAGE) toIdle() else render()
        params.gravity = Gravity.TOP or Gravity.START
    }

    private fun dragTo(e: MotionEvent) {
        params.x = (e.rawX - dragOffsetX).roundToInt().coerceIn(0, max(0, screenWidth() - dp(BUBBLE_DP)))
        params.y = (e.rawY - dragOffsetY).roundToInt().coerceIn(0, max(0, screenHeight() - dp(BUBBLE_DP)))
        update()
    }

    /** Let go: to the nearer edge, where it stays (also across starts). The card moves to the
     *  inner side once this touch is over. */
    private fun dock() {
        disarmStow()
        val right = params.x + dp(BUBBLE_DP) / 2 > screenWidth() / 2
        dockRight = right
        params.gravity = Gravity.TOP or if (right) Gravity.END else Gravity.START
        params.x = dp(EDGE_MARGIN_DP)
        update()
        prefs.dock = Pair(right, params.y.toFloat() / max(1, screenHeight()))
        main.post {
            render()
            autoStow.restart()
        }
    }

    /** While dragged: with the finger within [EDGE_ZONE_DP] of an edge the bubble sinks half into
     *  it, faded, with a tick, and letting go there tucks it away. Not while its own take runs. */
    private fun armStow(fingerX: Float) {
        val zone = dp(EDGE_ZONE_DP)
        val side = when {
            own -> null
            fingerX <= zone -> false
            fingerX >= screenWidth() - zone -> true
            else -> null
        }
        if ((side != null) == stowArmed && (side == null || side == stowRight)) return
        stowArmed = side != null
        if (side != null) stowRight = side
        bubble.translationX = when {
            !stowArmed -> 0f
            stowRight -> tuckOffset()
            else -> -tuckOffset()
        }
        if (stowArmed) bubble.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
        render()
    }

    private fun disarmStow() {
        stowArmed = false
        bubble.translationX = 0f
    }

    /** How far the bubble moves to sink half into the edge beside it; the window's bound, at the
     *  edge, cuts it off there. */
    private fun tuckOffset(): Float = dp(BUBBLE_DP) / 2f + dp(PAD_H_DP)

    /** Into the edge `right` says: the handle where the bubble was, its middle level with the
     *  bubble's (also across starts). The window changes once this touch is over. */
    private fun stow(right: Boolean) {
        autoStow.cancel()
        disarmStow()
        val top = params.y.coerceIn(0, max(0, screenHeight() - dp(BUBBLE_DP)))
        stowed = true
        dockRight = right
        prefs.dock = Pair(right, top.toFloat() / max(1, screenHeight()))
        prefs.stowed = true
        main.post {
            padForHandle()
            params.gravity = Gravity.TOP or if (right) Gravity.END else Gravity.START
            params.x = 0
            params.y = handleTop(top)
            render()
        }
    }

    /** Out of the edge again, level with the handle, sliding out as it comes. */
    private fun unstow() {
        if (!stowed) return
        val top = bubbleTop(params.y)
        stowed = false
        prefs.stowed = false
        prefs.dock = Pair(dockRight, top.toFloat() / max(1, screenHeight()))
        padForBubble()
        params.gravity = Gravity.TOP or if (dockRight) Gravity.END else Gravity.START
        params.x = dp(EDGE_MARGIN_DP)
        params.y = top
        render()
        bubble.translationX = if (dockRight) tuckOffset() else -tuckOffset()
        bubble.animate().translationX(0f).setDuration(SLIDE_MS).start()
        autoStow.restart()
    }

    /** The handle: a tap, or a pull inward past [PULL_DP], brings the bubble back; dragged up or
     *  down it moves along its edge. Once the bubble is back the rest of the touch is ignored. */
    private val handleTouch = View.OnTouchListener { view, e ->
        if (!stowed) return@OnTouchListener true
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                handleDownX = e.rawX
                handleDownY = e.rawY
                handleStartY = params.y
                handleMoved = false
                handleAlong = false
            }
            MotionEvent.ACTION_MOVE -> {
                val dx = e.rawX - handleDownX
                val dy = e.rawY - handleDownY
                if (!handleMoved && hypot(dx, dy) > touchSlop) {
                    handleMoved = true
                    handleAlong = abs(dy) > abs(dx)
                }
                when {
                    !handleMoved -> Unit
                    handleAlong -> {
                        params.y = (handleStartY + dy).roundToInt().coerceIn(0, max(0, screenHeight() - dp(HANDLE_H_DP)))
                        update()
                    }
                    (if (dockRight) -dx else dx) >= dp(PULL_DP) -> unstow()
                }
            }
            MotionEvent.ACTION_UP -> when {
                !handleMoved -> view.performClick()
                handleAlong -> keepHandle()
            }
            MotionEvent.ACTION_CANCEL -> if (handleMoved && handleAlong) keepHandle()
        }
        true
    }

    /** The handle moved along its edge: the bubble comes back there (also across starts). */
    private fun keepHandle() {
        prefs.dock = Pair(dockRight, bubbleTop(params.y).toFloat() / max(1, screenHeight()))
    }

    /** Screen readers tuck the bubble away with an action of its own; the handle's click brings
     *  it back. */
    private fun offerStow() {
        stowAction?.let { ViewCompat.removeAccessibilityAction(bubble, it) }
        stowAction = ViewCompat.addAccessibilityAction(bubble, prefs.string("stow", "")) { _, _ ->
            if (own || stowed) {
                false
            } else {
                stow(dockRight)
                true
            }
        }
    }

    private fun padForBubble() = root.setPadding(dp(PAD_H_DP), dp(PAD_V_DP), dp(PAD_H_DP), dp(PAD_V_DP))

    private fun padForHandle() = root.setPadding(0, 0, 0, 0)

    /** The handle's window top for a bubble window at [top]: its middle level with the bubble's. */
    private fun handleTop(top: Int): Int =
        (top + dp(PAD_V_DP) + (dp(BUBBLE_DP) - dp(HANDLE_H_DP)) / 2).coerceIn(0, max(0, screenHeight() - dp(HANDLE_H_DP)))

    /** The bubble window's top for a handle at [top]. */
    private fun bubbleTop(top: Int): Int =
        (top - dp(PAD_V_DP) - (dp(BUBBLE_DP) - dp(HANDLE_H_DP)) / 2).coerceIn(0, max(0, screenHeight() - dp(BUBBLE_DP)))

    // ---- views ----

    private var wave: WaveView? = null
    private var timer: TextView? = null
    /** What the card shows now; it is rebuilt when this changes. */
    private var cardKey: String? = null

    private fun buildViews() {
        root.orientation = LinearLayout.HORIZONTAL
        root.gravity = Gravity.CENTER_VERTICAL
        root.clipChildren = false
        // The bubble sinks into the edge through the padding (armStow).
        root.clipToPadding = false

        bubbleFace.shape = GradientDrawable.OVAL
        bubble.background = bubbleFace
        bubble.elevation = dp(6).toFloat()
        bubble.isClickable = true
        bubble.contentDescription = prefs.string("label", "")
        bubble.setOnTouchListener(touch)
        icon.scaleType = ImageView.ScaleType.CENTER_INSIDE
        bubble.addView(icon, FrameLayout.LayoutParams(dp(26), dp(26), Gravity.CENTER))

        card.orientation = LinearLayout.VERTICAL
        card.setPadding(dp(14), dp(10), dp(14), dp(10))
        card.background = GradientDrawable().apply {
            cornerRadius = dp(16).toFloat()
            setColor(CARD)
        }
        card.elevation = dp(4).toFloat()

        handleFace.setColor(IDLE)
        handleFace.setStroke(dp(1), HANDLE_EDGE)
        handleTab.background = handleFace
        handleTab.addView(
            View(context).apply {
                background = GradientDrawable().apply {
                    cornerRadius = dp(1).toFloat()
                    setColor(MUTED)
                }
            },
            FrameLayout.LayoutParams(dp(2), dp(18), Gravity.CENTER),
        )
        handle.isClickable = true
        handle.contentDescription = prefs.string("expand", "")
        handle.setOnTouchListener(handleTouch)
        handle.setOnClickListener { unstow() }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            // Out of the way of the system's back gesture, which starts at the same edge.
            handle.addOnLayoutChangeListener { v, _, _, _, _, _, _, _, _ ->
                v.systemGestureExclusionRects = listOf(Rect(0, 0, v.width, v.height))
            }
        }
        offerStow()
    }

    /** The card on the inner side of the bubble. The bubble is never taken out of the window
     *  while a finger is on it (that would cancel the gesture): the order changes only once the
     *  bubble docked at the other edge, after the finger let go. */
    private fun arrange() {
        if (arrangedRight == dockRight) return
        arrangedRight = dockRight
        root.removeAllViews()
        val bubbleParams = LinearLayout.LayoutParams(dp(BUBBLE_DP), dp(BUBBLE_DP))
        val cardParams = LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT)
        if (dockRight) {
            cardParams.marginEnd = dp(10)
            root.addView(card, cardParams)
            root.addView(bubble, bubbleParams)
        } else {
            cardParams.marginStart = dp(10)
            root.addView(bubble, bubbleParams)
            root.addView(card, cardParams)
        }
        // The handle on the edge itself, rounded on the inner side.
        root.addView(handle, LinearLayout.LayoutParams(dp(HANDLE_W_DP), dp(HANDLE_H_DP)))
        handle.removeAllViews()
        handle.addView(
            handleTab,
            FrameLayout.LayoutParams(dp(HANDLE_TAB_W_DP), dp(HANDLE_TAB_H_DP), Gravity.CENTER_VERTICAL or if (dockRight) Gravity.END else Gravity.START),
        )
        val r = dp(HANDLE_TAB_W_DP / 2).toFloat()
        handleFace.cornerRadii = if (dockRight) floatArrayOf(r, r, 0f, 0f, 0f, 0f, r, r) else floatArrayOf(0f, 0f, r, r, r, r, 0f, 0f)
    }

    /** Lay the bubble and its card out for the current state. */
    private fun render() {
        if (!shown) return
        arrange()
        if (stowed) {
            cardKey = null
            card.visibility = View.GONE
            bubble.visibility = View.GONE
            handle.visibility = View.VISIBLE
            handleTab.alpha = if (elsewhere) 0.5f else 0.92f
            update()
            return
        }
        handle.visibility = View.GONE
        bubble.visibility = View.VISIBLE
        val showCard = phase != Phase.IDLE && !dragging && !(phase == Phase.RESULT && !prefs.showResult)
        if (showCard) {
            val key = listOf(phase, slidAway, pasteArmed, pasteRequested, tapTake, stage, message, resultText, resultLabel, prefs.showDiscard, screenWidth()).joinToString("|")
            if (key != cardKey) {
                fillCard()
                cardKey = key
            }
        } else {
            cardKey = null
        }
        card.visibility = if (showCard) View.VISIBLE else View.GONE
        val active = own && (phase == Phase.LISTENING || phase == Phase.PROCESSING)
        bubbleFace.setColor(
            when {
                active && slidAway -> DANGER
                active -> ACCENT
                else -> IDLE
            },
        )
        icon.setImageResource(
            when {
                active && slidAway -> R.drawable.voltip_bubble_close
                phase == Phase.RESULT -> R.drawable.voltip_bubble_check
                else -> R.drawable.voltip_bubble_mic
            },
        )
        bubble.alpha = when {
            stowArmed -> 0.6f
            elsewhere && !own -> 0.45f
            active || phase != Phase.IDLE -> 1f
            else -> 0.9f
        }
        update()
    }

    private fun fillCard() {
        card.removeAllViews()
        wave = null
        timer = null
        val cardWidth = min(dp(CARD_MAX_DP), screenWidth() - dp(BUBBLE_DP + EDGE_MARGIN_DP + 28))
        when (phase) {
            Phase.LISTENING -> {
                val row = LinearLayout(context).apply {
                    orientation = LinearLayout.HORIZONTAL
                    gravity = Gravity.CENTER_VERTICAL
                }
                row.addView(dot(if (slidAway) DANGER else ACCENT_LIGHT))
                val waveView = WaveView(context)
                wave = waveView
                row.addView(waveView, LinearLayout.LayoutParams(dp(WAVE_BARS * 4), dp(24)).apply { marginStart = dp(8) })
                val since = if (startedAt > 0) System.currentTimeMillis() - startedAt else 0L
                val time = text(elapsed(since), 13f, Color.WHITE, mono = true).apply { setPadding(dp(10), 0, 0, 0) }
                timer = time
                row.addView(time)
                card.addView(row)
                val hint = when {
                    pasteArmed -> prefs.string("releasePaste", "")
                    slidAway -> prefs.string("releaseCancel", "")
                    tapTake -> prefs.string("recordingTap", "")
                    // 手势粘贴 can be armed: the hint names the inward swipe as well.
                    prefs.pasteEnabled && BubblePasteService.connected ->
                        prefs.string("recordingHoldPaste", prefs.string("recordingHold", ""))
                    else -> prefs.string("recordingHold", "")
                }
                card.addView(text(hint, 12f, if (slidAway) DANGER_LIGHT else MUTED).apply { setPadding(0, dp(4), 0, 0) })
                if (tapTake) {
                    card.addView(
                        chips(listOf(prefs.string("cancel", "") to { cancelTake() })),
                        LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(8) },
                    )
                }
            }
            Phase.PROCESSING -> {
                val row = LinearLayout(context).apply {
                    orientation = LinearLayout.HORIZONTAL
                    gravity = Gravity.CENTER_VERTICAL
                }
                row.addView(dot(ACCENT_LIGHT))
                row.addView(
                    text(
                        if (stage == "refining") prefs.string("refining", "") else prefs.string("processing", ""),
                        13f,
                        Color.WHITE,
                    ).apply { setPadding(dp(8), 0, 0, 0) },
                )
                card.addView(row)
            }
            Phase.RESULT -> {
                val row = LinearLayout(context).apply {
                    orientation = LinearLayout.HORIZONTAL
                    gravity = Gravity.CENTER_VERTICAL
                }
                row.addView(checkBadge())
                row.addView(text(resultLabel.ifEmpty { prefs.string("copied", "") }, 13f, Color.WHITE, bold = true).apply { setPadding(dp(8), 0, 0, 0) })
                card.addView(row)
                card.addView(
                    text(resultText, 14f, Color.WHITE).apply {
                        maxLines = 3
                        ellipsize = TextUtils.TruncateAt.END
                        setMaxWidth(cardWidth - dp(28))
                        setPadding(0, dp(8), 0, 0)
                    },
                )
                val actions = mutableListOf<Pair<String, () -> Unit>>(prefs.string("again", "") to { startTake(tap = true, held = false) })
                if (prefs.showDiscard) actions += prefs.string("discard", "") to { discard() }
                card.addView(
                    chips(actions),
                    LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(10) },
                )
            }
            Phase.MESSAGE -> {
                card.addView(
                    text(message, 13f, Color.WHITE).apply {
                        maxLines = 3
                        ellipsize = TextUtils.TruncateAt.END
                        setMaxWidth(cardWidth - dp(28))
                    },
                )
            }
            Phase.IDLE -> Unit
        }
    }

    private fun text(value: String, sizeSp: Float, color: Int, bold: Boolean = false, mono: Boolean = false) =
        TextView(context).apply {
            text = value
            setTextColor(color)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, sizeSp)
            if (bold) setTypeface(typeface, Typeface.BOLD)
            if (mono) typeface = Typeface.MONOSPACE
            includeFontPadding = false
        }

    private fun dot(color: Int) = View(context).apply {
        background = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(color)
        }
        layoutParams = LinearLayout.LayoutParams(dp(8), dp(8))
    }

    private fun checkBadge() = FrameLayout(context).apply {
        background = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(ACCENT_LIGHT)
        }
        layoutParams = LinearLayout.LayoutParams(dp(18), dp(18))
        addView(
            ImageView(context).apply { setImageResource(R.drawable.voltip_bubble_check) },
            FrameLayout.LayoutParams(dp(12), dp(12), Gravity.CENTER),
        )
    }

    private fun chips(actions: List<Pair<String, () -> Unit>>) = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        for ((index, action) in actions.withIndex()) {
            val chip = text(action.first, 12f, Color.WHITE).apply {
                background = GradientDrawable().apply {
                    cornerRadius = dp(8).toFloat()
                    setColor(CHIP)
                }
                setPadding(dp(12), dp(6), dp(12), dp(6))
                isClickable = true
                setOnClickListener { action.second() }
            }
            addView(
                chip,
                LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
                    if (index > 0) marginStart = dp(8)
                },
            )
        }
    }

    /** The take's level over its last seconds: the newer half in the accent, the older faint. */
    private inner class WaveView(context: Context) : View(context) {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        private val bar = RectF()

        override fun onDraw(canvas: Canvas) {
            val w = dp(2).toFloat()
            val gap = dp(2).toFloat()
            val mid = height / 2f
            for (i in 0 until WAVE_BARS) {
                val level = levels[i]
                val h = max(dp(2).toFloat(), level * height)
                val x = i * (w + gap)
                bar.set(x, mid - h / 2, x + w, mid + h / 2)
                paint.color = if (i >= WAVE_BARS / 2) ACCENT_LIGHT else FAINT
                canvas.drawRoundRect(bar, w / 2, w / 2, paint)
            }
        }
    }

    private fun haptic(confirm: Boolean) {
        val constant = when {
            confirm && Build.VERSION.SDK_INT >= Build.VERSION_CODES.R -> HapticFeedbackConstants.CONFIRM
            confirm -> HapticFeedbackConstants.KEYBOARD_TAP
            else -> HapticFeedbackConstants.LONG_PRESS
        }
        bubble.performHapticFeedback(constant)
    }

    private fun update() {
        if (!shown) return
        try {
            windows.updateViewLayout(root, params)
        } catch (e: RuntimeException) {
            Log.w(TAG, "the bubble did not move: ${e.message}")
        }
    }

    private fun screenWidth(): Int = context.resources.displayMetrics.widthPixels

    private fun screenHeight(): Int = context.resources.displayMetrics.heightPixels

    private fun dp(value: Int): Int = (value * density).roundToInt()

    companion object {
        /** The bubble's stream id for the level meter: far above the ids the app's JavaScript
         *  numbers its own from (src/backend/transport.ts), so neither reads the other's frames. */
        val CHANNEL: ULong = 1uL shl 40
        private const val CHANNEL_KEY = "__voltipChannel"
        const val HOLD_DELAY_MS = 180L
        const val RESULT_MS = 5_000L
        private const val CHECK_MS = 1_200L
        private const val MESSAGE_MS = 3_000L
        private const val TICK_MS = 250L
        private const val WAVE_BARS = 24
        private const val BUBBLE_DP = 56
        private const val EDGE_MARGIN_DP = 8
        /** The window's padding around the bubble, room for its shadow. */
        private const val PAD_H_DP = 4
        private const val PAD_V_DP = 8
        /** A drag let go with the finger this close to an edge tucks the bubble into it. */
        const val EDGE_ZONE_DP = 24
        /** A pull on the handle this far inward brings the bubble back. */
        private const val PULL_DP = 16
        /** The handle's touch area, and the tab that shows of it. */
        private const val HANDLE_W_DP = 32
        private const val HANDLE_H_DP = 80
        private const val HANDLE_TAB_W_DP = 12
        private const val HANDLE_TAB_H_DP = 56
        private const val SLIDE_MS = 180L
        private const val CARD_MAX_DP = 300
        private val IDLE = Color.parseColor("#262A35")
        private val ACCENT = Color.parseColor("#1F6FEB")
        private val ACCENT_LIGHT = Color.parseColor("#4C9DFF")
        private val DANGER = Color.parseColor("#D93C3C")
        private val DANGER_LIGHT = Color.parseColor("#FF8A80")
        private val CARD = Color.parseColor("#F01F2430")
        private val CHIP = Color.parseColor("#3A404D")
        private val MUTED = Color.parseColor("#B6BCC8")
        private val FAINT = Color.parseColor("#5F6675")
        private val HANDLE_EDGE = Color.parseColor("#33FFFFFF")
        /** A core message's machine prefix (`asr: `), as `coreMessageText` drops it. */
        private val MACHINE_PREFIX = Regex("^[a-z][a-z_]*(?:\\.[a-z_]+)*(?: [a-z]+)?: ")
        private const val TAG = "VoltipBubble"
        private const val WINDOW_TITLE = "VoltipBubble"

        /** `00:04`. */
        fun elapsed(ms: Long): String {
            val s = max(0L, ms) / 1000
            return "%02d:%02d".format(s / 60, s % 60)
        }
    }
}
