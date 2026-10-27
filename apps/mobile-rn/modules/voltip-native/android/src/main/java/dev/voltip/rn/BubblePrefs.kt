package dev.voltip.rn

import android.content.Context
import org.json.JSONObject

/**
 * The floating bubble's options (docs/dictation.md §20.12), kept in the app's SharedPreferences:
 * whether the user switched it on, how to talk, whether the result shows, where on the screen it
 * sits, and the texts it shows in the app's language (the app hands them over from its
 * dictionaries with every change, `BubbleConfig.strings` in src/backend/bubble.tsx).
 */
class BubblePrefs(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences(NAME, Context.MODE_PRIVATE)

    val enabled: Boolean
        get() = prefs.getBoolean(KEY_ENABLED, false)

    /** `hold` (hold to talk, slide up to cancel) or `tap` (tap to start, tap again to finish). */
    val mode: String
        get() = prefs.getString(KEY_MODE, MODE_HOLD).takeIf { it == MODE_TAP } ?: MODE_HOLD

    val showResult: Boolean
        get() = prefs.getBoolean(KEY_SHOW_RESULT, true)

    /** The bubble's edge (left or right) and its height on the screen (0…1), where it was let go. */
    var dock: Pair<Boolean, Float>
        get() = Pair(prefs.getBoolean(KEY_DOCK_RIGHT, true), prefs.getFloat(KEY_DOCK_Y, 0.62f))
        set(value) {
            prefs.edit().putBoolean(KEY_DOCK_RIGHT, value.first).putFloat(KEY_DOCK_Y, value.second).apply()
        }

    /** The text `key` in the app's language, `fallback` before the app handed any over. */
    fun string(key: String, fallback: String): String =
        prefs.getString(STRING_PREFIX + key, null)?.takeIf { it.isNotEmpty() } ?: fallback

    /** Save a `BubbleConfig`: `enabled`, `mode`, `showResult`, `strings`. */
    fun save(config: JSONObject) {
        val edit = prefs.edit()
        edit.putBoolean(KEY_ENABLED, config.optBoolean("enabled", false))
        edit.putString(KEY_MODE, if (config.optString("mode") == MODE_TAP) MODE_TAP else MODE_HOLD)
        edit.putBoolean(KEY_SHOW_RESULT, config.optBoolean("showResult", true))
        config.optJSONObject("strings")?.let { strings ->
            for (key in strings.keys()) edit.putString(STRING_PREFIX + key, strings.optString(key))
        }
        edit.apply()
    }

    /** Switched off from the notification's 关闭: it stays off until the user switches it on. */
    fun disable() {
        prefs.edit().putBoolean(KEY_ENABLED, false).apply()
    }

    companion object {
        const val MODE_HOLD = "hold"
        const val MODE_TAP = "tap"
        private const val NAME = "voltip-bubble"
        private const val KEY_ENABLED = "enabled"
        private const val KEY_MODE = "mode"
        private const val KEY_SHOW_RESULT = "show_result"
        private const val KEY_DOCK_RIGHT = "dock_right"
        private const val KEY_DOCK_Y = "dock_y"
        private const val STRING_PREFIX = "s."
    }
}
