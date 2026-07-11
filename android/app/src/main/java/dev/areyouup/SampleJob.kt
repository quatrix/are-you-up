package dev.areyouup

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.ComponentName
import android.content.Context
import android.util.Log
import dev.areyouup.core.Store
import dev.areyouup.core.Syncer
import dev.areyouup.core.Synthesizer
import dev.areyouup.core.Timestamps
import kotlin.concurrent.thread

// ==========================================================================
// The entire runtime of the app (ADR-0007)
// ==========================================================================
//
// A persisted 15-minute periodic job: replay system screen/keyguard
// events from the stored cursor, synthesize samples, buffer, sync,
// prune, exit. No other component of this app ever runs in the
// background - do not add receivers, services, alarms, or wakelocks.
class SampleJob : JobService() {

    companion object {
        const val TAG = "are-you-up"
        private const val JOB_ID = 1
        private const val PERIOD_MS = 15 * 60 * 1000L
        private const val PRUNE_AFTER_MS = 7 * 24 * 60 * 60 * 1000L

        // No constraints beyond the period: the job must run even with no
        // network (samples buffer; sync just fails and retries next run).
        fun schedule(context: Context) {
            val scheduler = context.getSystemService(JobScheduler::class.java)
            // Re-scheduling an existing periodic job resets its phase, so
            // return early when the pending job already matches. But ONLY
            // then: persisted jobs survive app updates (the documented
            // upgrade flow is git pull && make install), so a changed
            // PERIOD_MS must invalidate the stale pending job or it would
            // pin the old parameters forever.
            val pending = scheduler.getPendingJob(JOB_ID)
            if (pending != null && pending.intervalMillis == PERIOD_MS) return
            scheduler.schedule(
                JobInfo.Builder(JOB_ID, ComponentName(context, SampleJob::class.java))
                    .setPeriodic(PERIOD_MS)
                    .setPersisted(true) // survives reboot; needs RECEIVE_BOOT_COMPLETED
                    .build()
            )
            Log.i(TAG, "job scheduled: every ${PERIOD_MS / 60_000} min, persisted")
        }

        // In the companion so MainActivity's "Sync now" button can run one
        // cycle on a user-initiated thread. That is still not background
        // machinery (ADR-0007): it only ever runs while the owner is
        // looking at the screen, and overlap with a scheduled run is the
        // same idempotent-overlap case that onStopJob documents.
        fun runOnce(context: Context) {
            val prefs = Prefs(context)
            // First run ever: start at the current instant - history before
            // install is not reported (spec: no backfill).
            val cursor = prefs.cursor
                ?: Synthesizer.Cursor(System.currentTimeMillis(), screenOn = false, unlocked = false)
            // Clamped against backward clock steps (NTP/carrier resync between
            // runs): now < cursor.tsMs would synthesize a spurious past sample
            // and regress the cursor (LAB_NOTES 2026-07-11). Clamping turns the
            // run into a no-op span instead; the next run heals naturally.
            val now = maxOf(System.currentTimeMillis(), cursor.tsMs)

            val events = queryEvents(context, cursor.tsMs, now)
            val result = Synthesizer.synthesize(cursor, events, now)

            val store = Store(context)
            try {
                if (prefs.paused) {
                    // Paused spans become permanent gaps: drop the samples but
                    // still advance the cursor (mac pause semantics).
                    Log.i(TAG, "paused: dropped ${result.sampleTimesMs.size} samples")
                } else {
                    for (t in result.sampleTimesMs) store.insert(Timestamps.format(t), 0)
                }
                prefs.cursor = result.next

                val samplesNote =
                    if (prefs.paused) "${result.sampleTimesMs.size} samples dropped (paused)"
                    else "${result.sampleTimesMs.size} samples"
                // Blank URL = first launch, not yet configured: skip the
                // doomed request but keep buffering; the summary names the
                // actual fix instead of a cryptic connect error.
                val outcome =
                    if (prefs.serverUrl.isBlank())
                        Syncer.Outcome.Failed(0, "server url not configured (set it in the app)")
                    else Syncer(prefs.serverUrl, prefs.source).sync(store)
                val summary = when (outcome) {
                    is Syncer.Outcome.Ok -> {
                        // Only when rows actually reached the server: an
                        // empty-queue Ok(0) says nothing about reachability,
                        // and "last successful sync" reads as a health signal.
                        if (outcome.synced > 0) prefs.lastSyncTs = Timestamps.format(now)
                        store.pruneSynced(Timestamps.format(now - PRUNE_AFTER_MS))
                        "${Timestamps.format(now)}: ${events.size} events, " +
                            "$samplesNote, synced ${outcome.synced}"
                    }
                    is Syncer.Outcome.Failed ->
                        "${Timestamps.format(now)}: ${events.size} events, " +
                            "$samplesNote, " +
                            "sync FAILED after ${outcome.synced}: ${outcome.reason}"
                }
                prefs.lastRunSummary = summary
                Log.i(TAG, summary)
            } finally {
                store.close()
            }
        }

        // Maps the system's usage events to the Synthesizer's platform-free
        // event type. Without the Usage Access grant, queryEvents just
        // returns nothing: the job logs "0 events" and the activity shows
        // the missing grant.
        private fun queryEvents(
            context: Context,
            fromMs: Long,
            toMs: Long,
        ): List<Synthesizer.Event> {
            val usm = context.getSystemService(UsageStatsManager::class.java)
            val out = mutableListOf<Synthesizer.Event>()
            val events = usm.queryEvents(fromMs, toMs)
            val e = UsageEvents.Event()
            while (events.hasNextEvent()) {
                events.getNextEvent(e)
                val kind = when (e.eventType) {
                    UsageEvents.Event.SCREEN_INTERACTIVE -> Synthesizer.Event.Kind.SCREEN_ON
                    UsageEvents.Event.SCREEN_NON_INTERACTIVE -> Synthesizer.Event.Kind.SCREEN_OFF
                    UsageEvents.Event.KEYGUARD_HIDDEN -> Synthesizer.Event.Kind.UNLOCKED
                    UsageEvents.Event.KEYGUARD_SHOWN -> Synthesizer.Event.Kind.LOCKED
                    UsageEvents.Event.DEVICE_SHUTDOWN -> Synthesizer.Event.Kind.SHUTDOWN
                    else -> null
                }
                if (kind != null) out.add(Synthesizer.Event(e.timeStamp, kind))
            }
            return out.sortedBy { it.tsMs } // defensive; the API returns sorted
        }
    }

    // Jobs start on the main thread; sqlite + network work happens on a
    // worker thread that reports completion via jobFinished.
    override fun onStartJob(params: JobParameters): Boolean {
        thread(name = "are-you-up-job") {
            try {
                runOnce(applicationContext)
            } catch (e: Exception) {
                // Log-and-finish keeps the periodic schedule alive. No
                // usage is lost: the cursor only advances after synthesis
                // and insertion succeeded, so the next run replays.
                Log.e(TAG, "job failed: ${e.message}", e)
            }
            jobFinished(params, false)
        }
        return true // work continues on the worker thread
    }

    // true = retry later. The worker thread is deliberately not
    // interrupted; if its run overlaps the retry, the overlap is safe by
    // idempotence (level-based event replay, INSERT OR IGNORE, server
    // upsert - see LAB_NOTES 2026-07-11).
    override fun onStopJob(params: JobParameters): Boolean = true
}
