package dev.areyouup

import android.app.Activity
import android.app.AppOpsManager
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Intent
import android.os.Bundle
import android.os.Process
import android.provider.Settings
import android.util.Log
import android.widget.Button
import android.widget.EditText
import android.widget.Switch
import android.widget.TextView
import dev.areyouup.core.Store
import kotlin.concurrent.thread

// The only UI: status + config. Opening it is also what (re)arms the
// job - including after a force-stop, which cancels persisted jobs.
class MainActivity : Activity() {

    private lateinit var prefs: Prefs

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.main)
        prefs = Prefs(this)

        findViewById<EditText>(R.id.server_url).setText(prefs.serverUrl)
        findViewById<Button>(R.id.save).setOnClickListener {
            prefs.serverUrl =
                findViewById<EditText>(R.id.server_url).text.toString().trim().trimEnd('/')
            refresh()
        }
        findViewById<Switch>(R.id.paused).setOnCheckedChangeListener { _, checked ->
            prefs.paused = checked
        }
        findViewById<Button>(R.id.grant).setOnClickListener {
            startActivity(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS))
        }
        findViewById<Button>(R.id.dump).setOnClickListener { dumpRecentEvents() }
        findViewById<Button>(R.id.sync_now).setOnClickListener { button ->
            // One job cycle, right now, user-initiated (e.g. right after
            // setting the server url instead of waiting for the 15-min
            // tick). Same code path as the scheduled job; overlap with a
            // concurrent scheduled run is idempotent (see SampleJob).
            button.isEnabled = false
            thread(name = "are-you-up-manual") {
                try {
                    SampleJob.runOnce(applicationContext)
                } catch (e: Exception) {
                    Log.e(SampleJob.TAG, "manual sync failed: ${e.message}", e)
                }
                runOnUiThread {
                    button.isEnabled = true
                    refresh()
                }
            }
        }

        SampleJob.schedule(this)
    }

    override fun onResume() {
        super.onResume()
        refresh()
    }

    private fun refresh() {
        findViewById<Switch>(R.id.paused).isChecked = prefs.paused
        val store = Store(this)
        val unsynced = try {
            store.unsyncedCount()
        } finally {
            store.close()
        }
        findViewById<TextView>(R.id.status).text = """
            usage access: ${if (hasUsageAccess()) "granted" else "NOT GRANTED"}
            last run: ${prefs.lastRunSummary}
            last sync: ${prefs.lastSyncSummary}
            last successful sync: ${prefs.lastSyncTs}
            unsynced samples: $unsynced
        """.trimIndent()
    }

    private fun hasUsageAccess(): Boolean {
        val ops = getSystemService(AppOpsManager::class.java)
        val mode = ops.unsafeCheckOpNoThrow(
            AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), packageName
        )
        return mode == AppOpsManager.MODE_ALLOWED
    }

    // The spec's on-device probe, kept forever as a debugging aid: dump
    // the last 2h of screen/keyguard usage events to logcat.
    private fun dumpRecentEvents() {
        val usm = getSystemService(UsageStatsManager::class.java)
        val now = System.currentTimeMillis()
        val events = usm.queryEvents(now - 2 * 60 * 60 * 1000, now)
        val e = UsageEvents.Event()
        var n = 0
        while (events.hasNextEvent()) {
            events.getNextEvent(e)
            val name = when (e.eventType) {
                UsageEvents.Event.SCREEN_INTERACTIVE -> "SCREEN_INTERACTIVE"
                UsageEvents.Event.SCREEN_NON_INTERACTIVE -> "SCREEN_NON_INTERACTIVE"
                UsageEvents.Event.KEYGUARD_HIDDEN -> "KEYGUARD_HIDDEN"
                UsageEvents.Event.KEYGUARD_SHOWN -> "KEYGUARD_SHOWN"
                UsageEvents.Event.DEVICE_SHUTDOWN -> "DEVICE_SHUTDOWN"
                UsageEvents.Event.DEVICE_STARTUP -> "DEVICE_STARTUP"
                else -> continue
            }
            Log.i(SampleJob.TAG, "event $name at ${e.timeStamp}")
            n++
        }
        Log.i(SampleJob.TAG, "dump: $n screen/keyguard events in last 2h")
    }
}
