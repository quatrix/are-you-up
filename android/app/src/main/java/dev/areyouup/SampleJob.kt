package dev.areyouup

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.ComponentName
import android.content.Context
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.util.Log
import dev.areyouup.core.Store
import dev.areyouup.core.Syncer
import dev.areyouup.core.Synthesizer
import dev.areyouup.core.Timestamps
import kotlin.concurrent.thread

// ==========================================================================
// The entire runtime of the app (ADR-0007, ADR-0009)
// ==========================================================================
//
// Two persisted 15-minute periodic jobs, both served by this JobService,
// and nothing else ever runs in the background - do not add receivers,
// services, alarms, or wakelocks.
//
// - The SAMPLER (job 1) is unconstrained: replay system screen/keyguard
//   events from the stored cursor, synthesize samples, buffer, exit. It
//   must run offline so samples keep accumulating.
// - The SYNC job (job 2) is gated on a VPN network existing: the server
//   only resolves through tailscale, so an unconstrained sync mostly
//   fired while the tunnel was down and failed. Constraint satisfaction
//   also works as a trigger - unlocking the phone brings tailscale up,
//   which starts the pending sync within seconds, so even a quick
//   check-something unlock uploads the backlog (ADR-0009).
class SampleJob : JobService() {

    companion object {
        const val TAG = "are-you-up"
        private const val JOB_ID_SAMPLE = 1
        private const val JOB_ID_SYNC = 2
        private const val PERIOD_MS = 15 * 60 * 1000L
        private const val PRUNE_AFTER_MS = 7 * 24 * 60 * 60 * 1000L

        // Store opens one sqlite connection per instance, so concurrent
        // sample/sync/manual runs would trip over each other's write
        // locks. All db-touching entry points serialize here.
        private val dbLock = Any()

        fun schedule(context: Context) {
            val scheduler = context.getSystemService(JobScheduler::class.java)
            // Re-scheduling an existing periodic job resets its phase, so
            // return early when the pending job already matches. But ONLY
            // then: persisted jobs survive app updates (the documented
            // upgrade flow is git pull && make install), so changed job
            // parameters must invalidate the stale pending job or it would
            // pin the old JobInfo forever.
            val pendingSample = scheduler.getPendingJob(JOB_ID_SAMPLE)
            if (pendingSample == null || pendingSample.intervalMillis != PERIOD_MS) {
                scheduler.schedule(
                    JobInfo.Builder(JOB_ID_SAMPLE, ComponentName(context, SampleJob::class.java))
                        .setPeriodic(PERIOD_MS)
                        .setPersisted(true) // survives reboot; needs RECEIVE_BOOT_COMPLETED
                        .build()
                )
                Log.i(TAG, "sampler scheduled: every ${PERIOD_MS / 60_000} min, persisted")
            }

            val pendingSync = scheduler.getPendingJob(JOB_ID_SYNC)
            if (pendingSync == null || pendingSync.intervalMillis != PERIOD_MS ||
                pendingSync.requiredNetwork == null
            ) {
                // Default NetworkRequests exclude VPNs; drop NOT_VPN and
                // require the VPN transport itself. No NetworkSpecifier -
                // persisted jobs forbid those.
                val vpn = NetworkRequest.Builder()
                    .removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
                    .addTransportType(NetworkCapabilities.TRANSPORT_VPN)
                    .build()
                scheduler.schedule(
                    JobInfo.Builder(JOB_ID_SYNC, ComponentName(context, SampleJob::class.java))
                        .setPeriodic(PERIOD_MS)
                        .setPersisted(true)
                        .setRequiredNetwork(vpn)
                        .build()
                )
                Log.i(TAG, "sync scheduled: every ${PERIOD_MS / 60_000} min, VPN-gated, persisted")
            }
        }

        // One synthesis pass: replay events since the cursor into buffered
        // samples. Never touches the network.
        fun sampleOnce(context: Context): Unit = synchronized(dbLock) {
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
                val summary = "${Timestamps.format(now)}: ${events.size} events, $samplesNote"
                prefs.lastRunSummary = summary
                Log.i(TAG, summary)
            } finally {
                store.close()
            }
        }

        // One sync pass: upload buffered samples, prune old synced rows.
        // Runs from the VPN-gated job, so by the time it fires the tunnel
        // exists (barring races, which just read as a failed attempt).
        fun syncOnce(context: Context): Unit = synchronized(dbLock) {
            val prefs = Prefs(context)
            val now = System.currentTimeMillis()
            // Blank URL = first launch, not yet configured: skip the doomed
            // request but keep buffering; the summary names the actual fix
            // instead of a cryptic connect error.
            if (prefs.serverUrl.isBlank()) {
                prefs.lastSyncSummary =
                    "${Timestamps.format(now)}: skipped, server url not configured (set it in the app)"
                Log.i(TAG, prefs.lastSyncSummary)
                return
            }

            val store = Store(context)
            try {
                val summary = when (val outcome = Syncer(prefs.serverUrl, prefs.source).sync(store)) {
                    is Syncer.Outcome.Ok -> {
                        // Only when rows actually reached the server: an
                        // empty-queue Ok(0) says nothing about reachability,
                        // and "last successful sync" reads as a health signal.
                        if (outcome.synced > 0) prefs.lastSyncTs = Timestamps.format(now)
                        store.pruneSynced(Timestamps.format(now - PRUNE_AFTER_MS))
                        "${Timestamps.format(now)}: synced ${outcome.synced}"
                    }
                    is Syncer.Outcome.Failed ->
                        "${Timestamps.format(now)}: FAILED after ${outcome.synced}: ${outcome.reason}"
                }
                prefs.lastSyncSummary = summary
                Log.i(TAG, "sync $summary")
            } finally {
                store.close()
            }
        }

        // The "Sync now" button: one full cycle, user-initiated, ignoring
        // the VPN gate (the user is looking at the screen, so the tunnel is
        // as up as it will ever be). Overlap with the scheduled jobs is
        // serialized by dbLock and idempotent besides.
        fun runOnce(context: Context) {
            sampleOnce(context)
            syncOnce(context)
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
        // Self-healing across upgrades: persisted jobs keep running the old
        // JobInfo set after `adb install -r` until the app is opened. Any
        // job run re-asserting the schedule means new/changed jobs take
        // effect without a manual launch.
        schedule(applicationContext)
        thread(name = "are-you-up-job") {
            try {
                when (params.jobId) {
                    JOB_ID_SYNC -> syncOnce(applicationContext)
                    else -> sampleOnce(applicationContext)
                }
            } catch (e: Exception) {
                // Log-and-finish keeps the periodic schedule alive. No
                // usage is lost: the cursor only advances after synthesis
                // and insertion succeeded, so the next run replays.
                Log.e(TAG, "job ${params.jobId} failed: ${e.message}", e)
            }
            jobFinished(params, false)
        }
        return true // work continues on the worker thread
    }

    // true = retry later. The worker thread is deliberately not
    // interrupted; if its run overlaps the retry, dbLock serializes the db
    // work and the overlap is safe by idempotence (level-based event
    // replay, INSERT OR IGNORE, server upsert - see LAB_NOTES 2026-07-11).
    override fun onStopJob(params: JobParameters): Boolean = true
}
