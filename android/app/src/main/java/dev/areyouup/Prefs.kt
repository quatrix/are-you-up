package dev.areyouup

import android.content.Context
import dev.areyouup.core.Synthesizer

// SharedPreferences accessors - the android-native equivalent of the mac
// client's config.json, plus the job's cursor and status breadcrumbs.
class Prefs(context: Context) {
    private val p = context.getSharedPreferences("are-you-up", Context.MODE_PRIVATE)

    // Deliberately no baked-in endpoint: the repo is public and the real
    // address is deployment config, entered once in the app on first
    // launch. Blank means "not configured yet" - the job skips syncing
    // (samples still buffer) and the status screen says so.
    var serverUrl: String
        get() = p.getString("server_url", "")!!
        set(v) { p.edit().putString("server_url", v).apply() }

    var source: String
        get() = p.getString("source", "pixel")!!
        set(v) { p.edit().putString("source", v).apply() }

    var paused: Boolean
        get() = p.getBoolean("paused", false)
        set(v) { p.edit().putBoolean("paused", v).apply() }

    // The synthesis cursor: the instant up to which samples exist, plus
    // the screen state at that instant. null means "never ran" - the
    // first run starts at now (no backfill, per the spec).
    var cursor: Synthesizer.Cursor?
        get() {
            val ts = p.getLong("cursor_ts", 0L)
            if (ts == 0L) return null
            return Synthesizer.Cursor(
                ts,
                screenOn = p.getBoolean("cursor_screen_on", false),
                unlocked = p.getBoolean("cursor_unlocked", false)
            )
        }
        set(v) {
            requireNotNull(v) { "cursor only ever advances, never resets" }
            p.edit()
                .putLong("cursor_ts", v.tsMs)
                .putBoolean("cursor_screen_on", v.screenOn)
                .putBoolean("cursor_unlocked", v.unlocked)
                .apply()
        }

    var lastRunSummary: String
        get() = p.getString("last_run", "never")!!
        set(v) { p.edit().putString("last_run", v).apply() }

    // The sync job reports separately from the sampler (ADR-0009): a
    // failing upload must stay visible even while sampling is healthy.
    var lastSyncSummary: String
        get() = p.getString("last_sync_summary", "never")!!
        set(v) { p.edit().putString("last_sync_summary", v).apply() }

    var lastSyncTs: String
        get() = p.getString("last_sync", "never")!!
        set(v) { p.edit().putString("last_sync", v).apply() }
}
