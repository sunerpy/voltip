package dev.voltip.rn

import android.content.Context
import org.json.JSONObject

/**
 * The floating bubble's options (docs/dictation.md §20.12), kept in the app's SharedPreferences:
 * whether the user switched it on, how to talk, whether the result shows and with 不复制, where on
 * the screen it sits and whether it is tucked into the edge, and the texts it shows in the app's
 * language (the app hands them over from its dictionaries with every change, `BubbleConfig.strings`
 * in src/backend/bubble.tsx).
 */
class BubblePrefs(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences(NAME, Context.MODE_PRIVATE)

    val enabled: Boolean
        get() = prefs.getBoolean(KEY_ENABLED, false)

    /** `hold` (hold to talk, slide up to cancel) or `tap` (tap to start, tap again to finish). */
    val mode: String
        get() = prefs.getString(KEY_MODE, MODE_HOLD).takeIf { it == MODE_TAP } ?: MODE_HOLD

    val pasteEnabled: Boolean get() = prefs.getBoolean(KEY_PASTE_ENABLED, false)
    val autoStowSeconds: Int get() = prefs.getInt(KEY_AUTO_STOW_SECONDS, 0).takeIf { it in listOf(0, 5, 10, 30) } ?: 0
    val showResult: Boolean
        get() = prefs.getBoolean(KEY_SHOW_RESULT, true)

    /** The result's card has 不复制 too (off unless the user asks for it, 2026-10-28). */
    val showDiscard: Boolean
        get() = prefs.getBoolean(KEY_SHOW_DISCARD, false)

    /** Tucked into its edge: only the handle shows. Cleared when the bubble is switched off, so
     *  switching it on shows the bubble itself. */
    var stowed: Boolean
        get() = prefs.getBoolean(KEY_STOWED, false)
        set(value) {
            prefs.edit().putBoolean(KEY_STOWED, value).apply()
        }

    /** The bubble's edge (left or right) and its height on the screen (0…1), where it was let go. */
    var dock: Pair<Boolean, Float>
        get() = Pair(prefs.getBoolean(KEY_DOCK_RIGHT, true), prefs.getFloat(KEY_DOCK_Y, 0.62f))
        set(value) {
            prefs.edit().putBoolean(KEY_DOCK_RIGHT, value.first).putFloat(KEY_DOCK_Y, value.second).apply()
        }

    /** The text `key` in the app's language, `fallback` before the app handed any over. */
    fun string(key: String, fallback: String): String =
        prefs.getString(STRING_PREFIX + key, null)?.takeIf { it.isNotEmpty() } ?: fallback

    /** Save a `BubbleConfig`: `enabled`, `mode`, `showResult`, `showDiscard`, `strings`. */
    fun save(config: JSONObject) {
        val edit = prefs.edit()
        val enabled = config.optBoolean("enabled", false)
        edit.putBoolean(KEY_ENABLED, enabled)
        if (!enabled) edit.putBoolean(KEY_STOWED, false)
        edit.putString(KEY_MODE, if (config.optString("mode") == MODE_TAP) MODE_TAP else MODE_HOLD)
        edit.putBoolean(KEY_SHOW_RESULT, config.optBoolean("showResult", true))
        edit.putBoolean(KEY_PASTE_ENABLED, config.optBoolean("pasteEnabled", false))
        edit.putInt(KEY_AUTO_STOW_SECONDS, config.optInt("autoStowSeconds", 0).takeIf { it in listOf(0, 5, 10, 30) } ?: 0)
        edit.putBoolean(KEY_SHOW_DISCARD, config.optBoolean("showDiscard", false))
        config.optJSONObject("strings")?.let { strings ->
            for (key in strings.keys()) edit.putString(STRING_PREFIX + key, strings.optString(key))
        }
        edit.apply()
    }

    /** Switched off from the notification's 关闭: it stays off until the user switches it on. */
    fun disable() {
        prefs.edit().putBoolean(KEY_ENABLED, false).putBoolean(KEY_STOWED, false).apply()
    }

    companion object {
        const val MODE_HOLD = "hold"
        const val MODE_TAP = "tap"
        private const val NAME = "voltip-bubble"
        private const val KEY_ENABLED = "enabled"
        private const val KEY_MODE = "mode"
        private const val KEY_SHOW_RESULT = "show_result"
        private const val KEY_PASTE_ENABLED = "paste_enabled"
        private const val KEY_AUTO_STOW_SECONDS = "auto_stow_seconds"
        private const val KEY_SHOW_DISCARD = "show_discard"
        private const val KEY_STOWED = "stowed"
        private const val KEY_DOCK_RIGHT = "dock_right"
        private const val KEY_DOCK_Y = "dock_y"
        private const val STRING_PREFIX = "s."
    }
}
