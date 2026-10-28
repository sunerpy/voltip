package dev.voltip.rn

import android.accessibilityservice.AccessibilityService
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Rect
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo

/**
 * Optional, user-enabled paste channel for the floating bubble.
 * No accessibility event monitoring or collection of foreign text.
 */
class BubblePasteService : AccessibilityService() {
    data class Target(
        val packageName: String,
        val windowId: Int,
        val viewId: String?,
        val bounds: Rect,
        val className: String?,
    )

    override fun onServiceConnected() {
        super.onServiceConnected()
        active = this
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) = Unit
    override fun onInterrupt() = Unit

    override fun onDestroy() {
        if (active === this) active = null
        super.onDestroy()
    }

    override fun onUnbind(intent: android.content.Intent?): Boolean {
        if (active === this) active = null
        return super.onUnbind(intent)
    }

    /** Never use the Voltip overlay or IME itself as a paste target. */
    private fun focusedEditor(): AccessibilityNodeInfo? {
        val window = windows.firstOrNull {
            it.type == AccessibilityWindowInfo.TYPE_APPLICATION && it.isFocused
        } ?: return null
        val editor = window.root?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT) ?: return null
        return editor.takeIf {
            it.isEditable && !it.isPassword &&
                it.packageName?.toString() != packageName &&
                it.windowId == window.id
        }
    }

    private fun identify(node: AccessibilityNodeInfo): Target {
        val bounds = Rect()
        node.getBoundsInScreen(bounds)
        return Target(
            node.packageName.toString(), node.windowId, node.viewIdResourceName,
            bounds, node.className?.toString(),
        )
    }

    private fun capture(): Target? = focusedEditor()?.let(::identify)

    private fun paste(text: String, target: Target): Boolean {
        if (text.isBlank()) return false
        val current = focusedEditor() ?: return false
        if (identify(current) != target) return false
        return try {
            // Write only the recognized text; never read the user's existing clipboard.
            val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("Voltip", text))
            // Android's true return indicates acceptance, not confirmed insertion.
            current.performAction(AccessibilityNodeInfo.ACTION_PASTE)
        } catch (_: RuntimeException) {
            false
        }
    }

    companion object {
        @Volatile private var active: BubblePasteService? = null

        val enabled: Boolean get() = active != null

        fun captureTarget(): Target? = active?.capture()

        fun pasteText(text: String, target: Target?): Boolean =
            if (target == null) false else active?.paste(text, target) ?: false
    }
}
