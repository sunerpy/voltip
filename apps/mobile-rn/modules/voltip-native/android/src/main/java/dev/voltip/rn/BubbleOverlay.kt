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
import dev.voltip.rn.uniffi.ShellException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import org.json.JSONObject
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/**
 * The floating bubble itself (docs/dictation.md §20.12; Penpot 「用量与额度」 boards 17a–17d): a round
 * talk button docked at an edge of the screen over other apps, and beside it a card for the take
 * it started: the recording with its level and time, the recognition, the copied result with
 * 再说一次 and 不复制 (shown [RESULT_MS]), or why it did not finish. Its takes are the phone's own
 * (`dictation_start`), recognised as the 说话 tab's are and copied to the clipboard by the shell's
 * injector; the card follows the core's dictation events for the take the bubble started, and
 * the bubble waits while a take started elsewhere runs.
 *
 * Holding (`mode` hold): a press that stays put for [HOLD_DELAY_MS] starts the take, releasing
 * finishes it, and a release [cancelDistance] away from the press cancels it; a press that moves
 * first drags the bubble, which docks at the nearer edge when let go. Tapping (`mode` tap): a tap
 * starts, the next finishes, the card's 取消 discards. The texts are the app's (`BubblePrefs`).
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
    private val params = WindowManager.LayoutParams(
        WindowManager.LayoutParams.WRAP_CONTENT,
        WindowManager.LayoutParams.WRAP_CONTENT,
        WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
        WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,
        PixelFormat.TRANSLUCENT,
    )
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
    private var startedAt = 0L
    private var stage = ""
    private var resultText = ""
    private var message = ""
    private var meter: Long? = null
    private val levels = FloatArray(WAVE_BARS)

    /** The edge the bubble is docked at, and the one its card was last arranged for. */
    private var dockRight = true
    private var arrangedRight: Boolean? = null

    private var downX = 0f
    private var downY = 0f
    private var dragging = false
    private var dragOffsetX = 0f
    private var dragOffsetY = 0f
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
        params.gravity = Gravity.TOP or if (right) Gravity.END else Gravity.START
        params.x = EDGE_MARGIN_DP.let(::dp)
        params.y = (y * screenHeight()).roundToInt().coerceIn(0, max(0, screenHeight() - dp(BUBBLE_DP)))
        try {
            windows.addView(root, params)
            shown = true
        } catch (e: RuntimeException) {
            // No permission any more (revoked while it ran): nothing to show.
            Log.w(TAG, "the bubble could not be shown: ${e.message}")
        }
        VoltipRuntime.addListener(this)
        render()
    }

    /** The options or texts changed. */
    fun refresh() {
        main.post {
            bubble.contentDescription = prefs.string("label", "")
            render()
        }
    }

    /** Take the bubble off the screen; a take it started is cancelled. */
    fun close() {
        VoltipRuntime.removeListener(this)
        main.removeCallbacksAndMessages(null)
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
            elsewhere = name == "listening" || name == "processing"
            render()
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
                resultText = state.optString("text")
                haptic(confirm = true)
                if (prefs.showResult) {
                    phase = Phase.RESULT
                    settleIn(RESULT_MS)
                } else {
                    // The bubble's check alone, a moment.
                    phase = Phase.RESULT
                    settleIn(CHECK_MS)
                }
            }
            "failed" -> {
                stopMeter()
                own = false
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
                toIdle()
                return
            }
            else -> return
        }
        render()
    }

    // ---- takes ----

    private fun startTake(tap: Boolean, held: Boolean) {
        if (own || elsewhere || phase == Phase.PROCESSING) return
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            showMessage(prefs.string("microphone", ""))
            return
        }
        main.removeCallbacks(settle)
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
        render()
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
                    val away = hypot(dx, dy) > cancelDistance
                    if (away != slidAway) {
                        slidAway = away
                        render()
                    }
                } else if (!dragging && hypot(dx, dy) > touchSlop && !(own && phase == Phase.LISTENING)) {
                    main.removeCallbacks(press)
                    beginDrag(e)
                }
                if (dragging) dragTo(e)
            }
            MotionEvent.ACTION_UP -> {
                main.removeCallbacks(press)
                when {
                    dragging -> dock()
                    holding -> if (slidAway) cancelTake() else stopTake()
                    else -> tapped()
                }
                dragging = false
            }
            MotionEvent.ACTION_CANCEL -> {
                main.removeCallbacks(press)
                if (dragging) dock()
                if (holding) cancelTake()
                dragging = false
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
        val right = params.x + dp(BUBBLE_DP) / 2 > screenWidth() / 2
        dockRight = right
        params.gravity = Gravity.TOP or if (right) Gravity.END else Gravity.START
        params.x = dp(EDGE_MARGIN_DP)
        update()
        prefs.dock = Pair(right, params.y.toFloat() / max(1, screenHeight()))
        main.post { render() }
    }

    // ---- views ----

    private var wave: WaveView? = null
    private var timer: TextView? = null
    /** What the card shows now; it is rebuilt when this changes. */
    private var cardKey: String? = null

    private fun buildViews() {
        root.orientation = LinearLayout.HORIZONTAL
        root.gravity = Gravity.CENTER_VERTICAL
        root.clipChildren = false
        root.setPadding(dp(4), dp(8), dp(4), dp(8))

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
    }

    /** Lay the bubble and its card out for the current state. */
    private fun render() {
        if (!shown) return
        arrange()
        val showCard = phase != Phase.IDLE && !dragging && !(phase == Phase.RESULT && !prefs.showResult)
        if (showCard) {
            val key = listOf(phase, slidAway, tapTake, stage, message, resultText, screenWidth()).joinToString("|")
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
        bubble.alpha = if (elsewhere && !own) 0.45f else if (active || phase != Phase.IDLE) 1f else 0.9f
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
                    slidAway -> prefs.string("releaseCancel", "")
                    tapTake -> prefs.string("recordingTap", "")
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
                row.addView(text(prefs.string("copied", ""), 13f, Color.WHITE, bold = true).apply { setPadding(dp(8), 0, 0, 0) })
                card.addView(row)
                card.addView(
                    text(resultText, 14f, Color.WHITE).apply {
                        maxLines = 3
                        ellipsize = TextUtils.TruncateAt.END
                        setMaxWidth(cardWidth - dp(28))
                        setPadding(0, dp(8), 0, 0)
                    },
                )
                card.addView(
                    chips(
                        listOf(
                            prefs.string("again", "") to { startTake(tap = true, held = false) },
                            prefs.string("discard", "") to { discard() },
                        ),
                    ),
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
        /** A core message's machine prefix (`asr: `), as `coreMessageText` drops it. */
        private val MACHINE_PREFIX = Regex("^[a-z][a-z_]*(?:\\.[a-z_]+)*(?: [a-z]+)?: ")
        private const val TAG = "VoltipBubble"

        /** `00:04`. */
        fun elapsed(ms: Long): String {
            val s = max(0L, ms) / 1000
            return "%02d:%02d".format(s / 60, s % 60)
        }
    }
}
