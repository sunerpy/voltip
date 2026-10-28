package dev.voltip.rn

import android.accessibilityservice.AccessibilityService
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo

/**
 * 手势粘贴 (docs/dictation.md §20.12): the accessibility service behind the floating bubble's
 * optional paste gesture, off until the user switches the gesture on in Voltip, agrees to what it
 * does and enables this service in the system's Accessibility settings. It takes no events
 * (res/xml/voltip_bubble_accessibility.xml names none) and reads nothing on its own: the bubble
 * asks it twice per take, both from a background thread. Once, when the held finger first reaches
 * the paste position, for the focused text field (its node, app and window; not its text). Again
 * when the result arrives: if that same field still has focus, the result goes onto the clipboard
 * and the field is asked to paste it (`ACTION_PASTE`; never `ACTION_SET_TEXT`, which would replace
 * what the field holds). Password fields and Voltip's own windows are never a target.
 */
class BubblePasteService : AccessibilityService() {
    /** The field the gesture was armed over. */
    class Target internal constructor(internal val node: AccessibilityNodeInfo, internal val id: PasteTargetId)

    override fun onServiceConnected() {
        super.onServiceConnected()
        active = this
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) = Unit

    override fun onInterrupt() = Unit

    override fun onUnbind(intent: Intent?): Boolean {
        if (active === this) active = null
        return super.onUnbind(intent)
    }

    override fun onDestroy() {
        if (active === this) active = null
        super.onDestroy()
    }

    /** The text field with input focus in the focused app window, unless it is a password field or
     *  Voltip's own. */
    private fun focusedField(): AccessibilityNodeInfo? {
        val window = windows.firstOrNull { it.type == AccessibilityWindowInfo.TYPE_APPLICATION && it.isFocused } ?: return null
        val field = window.root?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT) ?: return null
        return field.takeIf {
            it.isEditable && !it.isPassword && it.windowId == window.id && it.packageName?.toString() != packageName
        }
    }

    private fun idOf(node: AccessibilityNodeInfo) = PasteTargetId(node.packageName?.toString().orEmpty(), node.windowId)

    private fun capture(): Target? = focusedField()?.let { Target(it, idOf(it)) }

    private fun paste(text: String, target: Target?): Boolean {
        if (text.isBlank()) return false
        val current = if (target == null) null else focusedField()
        val outcome = BubblePasteDecision.outcome(
            requested = true,
            connected = true,
            captured = target?.id,
            current = current?.let(::idOf),
            sameNode = current != null && current == target?.node,
        )
        if (outcome != PasteOutcome.PASTE || current == null) return false
        return try {
            // The result itself, written just before the paste: the clipboard is never read.
            val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("Voltip", text))
            // `true` means the field accepted the request, not that the text went in.
            current.performAction(AccessibilityNodeInfo.ACTION_PASTE)
        } catch (_: RuntimeException) {
            false
        }
    }

    companion object {
        @Volatile private var active: BubblePasteService? = null

        /** The user enabled the service and the system connected it. */
        val connected: Boolean get() = active != null

        /** The focused text field, or null (no service, no field, a password field). Off the main
         *  thread: the query goes to the app the field is in. */
        fun captureTarget(): Target? = active?.capture()

        /** Paste [text] into [target] if it still has focus; false when nothing was asked of it.
         *  Off the main thread, as [captureTarget]. */
        fun pasteText(text: String, target: Target?): Boolean = active?.paste(text, target) ?: false
    }
}
