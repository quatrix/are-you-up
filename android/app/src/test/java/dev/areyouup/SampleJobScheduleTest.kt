package dev.areyouup

import android.app.job.JobInfo
import android.app.job.JobScheduler
import android.content.ComponentName
import android.net.NetworkCapabilities
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

// Scheduling is the one piece of SampleJob glue with real behavior in it:
// two persisted jobs with different constraints, plus stale-JobInfo
// invalidation across app upgrades (persisted jobs outlive `adb install -r`,
// so a changed JobInfo must be detected and rescheduled - see ADR-0009).
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class SampleJobScheduleTest {

    private val app = RuntimeEnvironment.getApplication()
    private val scheduler: JobScheduler
        get() = app.getSystemService(JobScheduler::class.java)

    @Test
    fun schedulesUnconstrainedSamplerAndVpnConstrainedSync() {
        SampleJob.schedule(app)

        // Sampler: must run offline on its own clock so samples buffer.
        val sampler = scheduler.getPendingJob(1)
        assertNotNull(sampler)
        assertTrue(sampler!!.isPeriodic)
        assertTrue(sampler.isPersisted)
        assertNull(sampler.requiredNetwork)

        // Sync: gated on a VPN network existing, so it fires the moment
        // tailscale comes up instead of failing into the void.
        val sync = scheduler.getPendingJob(2)
        assertNotNull(sync)
        assertTrue(sync!!.isPeriodic)
        assertTrue(sync.isPersisted)
        val net = sync.requiredNetwork
        assertNotNull(net)
        assertTrue(net!!.hasTransport(NetworkCapabilities.TRANSPORT_VPN))
        // default requests exclude VPNs; the constraint is useless unless
        // NOT_VPN was removed
        assertFalse(net.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN))
    }

    @Test
    fun scheduleIsIdempotent() {
        SampleJob.schedule(app)
        SampleJob.schedule(app)
        assertEquals(2, scheduler.allPendingJobs.size)
    }

    @Test
    fun staleUnconstrainedSyncJobIsReplaced() {
        // Simulates upgrading from a build whose job 2 had no network
        // constraint: same id and period, but stale JobInfo.
        scheduler.schedule(
            JobInfo.Builder(2, ComponentName(app, SampleJob::class.java))
                .setPeriodic(15 * 60 * 1000L)
                .setPersisted(true)
                .build()
        )
        SampleJob.schedule(app)
        assertNotNull(scheduler.getPendingJob(2)!!.requiredNetwork)
    }
}
