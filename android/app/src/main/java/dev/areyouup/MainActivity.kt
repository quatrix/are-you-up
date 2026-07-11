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
import android.widget.TextView

class MainActivity : Activity() {

    companion object {
        const val TAG = "are-you-up"
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.main)

        findViewById<Button>(R.id.grant).setOnClickListener {
            startActivity(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS))
        }
        findViewById<Button>(R.id.dump).setOnClickListener { dumpRecentEvents() }
    }

    override fun onResume() {
        super.onResume()
        findViewById<TextView>(R.id.status).text =
            "usage access: ${if (hasUsageAccess()) "granted" else "NOT GRANTED"}"
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
            Log.i(TAG, "event $name at ${e.timeStamp}")
            n++
        }
        Log.i(TAG, "dump: $n screen/keyguard events in last 2h")
    }
}
