package dev.areyouup.core

import dev.areyouup.core.Synthesizer.Event.Kind.LOCKED
import dev.areyouup.core.Synthesizer.Event.Kind.SCREEN_OFF
import dev.areyouup.core.Synthesizer.Event.Kind.SCREEN_ON
import dev.areyouup.core.Synthesizer.Event.Kind.SHUTDOWN
import dev.areyouup.core.Synthesizer.Event.Kind.UNLOCKED
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SynthesizerTest {

    private val t0 = 1_783_764_000_000L // 2026-07-11T10:00:00Z

    private fun s(sec: Int) = t0 + sec * 1000L

    private fun cursor(interactive: Boolean = false) =
        Synthesizer.Cursor(t0, screenOn = interactive, unlocked = interactive)

    private fun ev(sec: Int, kind: Synthesizer.Event.Kind) =
        Synthesizer.Event(s(sec), kind)

    @Test
    fun unlockSessionEmitsGridPlusEndSample() {
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(ev(10, SCREEN_ON), ev(12, UNLOCKED), ev(107, LOCKED)),
            nowMs = s(600)
        )
        // window [12s, 107s]: grid at 12, 42, 72, 102 plus the end at 107
        assertEquals(listOf(s(12), s(42), s(72), s(102), s(107)), r.sampleTimesMs)
        assertFalse(r.next.interactive)
        assertEquals(s(600), r.next.tsMs)
    }

    @Test
    fun screenOnWhileLockedEmitsNothing() {
        // checking lock-screen notifications is not "using the phone"
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(ev(10, SCREEN_ON), ev(40, SCREEN_OFF)),
            nowMs = s(600)
        )
        assertTrue(r.sampleTimesMs.isEmpty())
        assertFalse(r.next.interactive)
    }

    @Test
    fun relightInsideLockDelayResumesWithoutKeyguardEvent() {
        // screen off then on again before the keyguard re-engages: no
        // KEYGUARD events fire, yet the second window must still open
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(
                ev(10, SCREEN_ON), ev(12, UNLOCKED),
                ev(20, SCREEN_OFF), ev(25, SCREEN_ON), ev(30, LOCKED)
            ),
            nowMs = s(600)
        )
        // windows [12,20] and [25,30]
        assertEquals(listOf(s(12), s(20), s(25), s(30)), r.sampleTimesMs)
    }

    @Test
    fun openWindowEmitsUpToNowAndCursorStaysInteractive() {
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(ev(10, SCREEN_ON), ev(12, UNLOCKED)),
            nowMs = s(82)
        )
        assertEquals(listOf(s(12), s(42), s(72), s(82)), r.sampleTimesMs)
        assertTrue(r.next.interactive)
    }

    @Test
    fun continuationFromInteractiveCursor() {
        // previous run left a window open; this run has no new events
        val r = Synthesizer.synthesize(cursor(interactive = true), emptyList(), s(50))
        assertEquals(listOf(s(0), s(30), s(50)), r.sampleTimesMs)
        assertTrue(r.next.interactive)
    }

    @Test
    fun shutdownClosesWindowAndResetsState() {
        val r = Synthesizer.synthesize(
            cursor(interactive = true),
            listOf(ev(40, SHUTDOWN)),
            nowMs = s(600)
        )
        assertEquals(listOf(s(0), s(30), s(40)), r.sampleTimesMs)
        assertFalse(r.next.interactive)
    }

    @Test
    fun subPeriodWindowEmitsStartAndEnd() {
        // UNLOCKED before SCREEN_ON also occurs (device wake paths vary);
        // the window opens when BOTH bits are finally true
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(ev(10, UNLOCKED), ev(11, SCREEN_ON), ev(15, SCREEN_OFF)),
            nowMs = s(600)
        )
        assertEquals(listOf(s(11), s(15)), r.sampleTimesMs)
    }

    @Test
    fun zeroLengthWindowEmitsOneSample() {
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(ev(10, SCREEN_ON), ev(10, UNLOCKED), ev(10, LOCKED)),
            nowMs = s(600)
        )
        assertEquals(listOf(s(10)), r.sampleTimesMs)
    }

    @Test
    fun multipleWindowsInOneRun() {
        // The LOCKED at 46 mirrors the probe-observed device behavior: the
        // keyguard re-engages ~1s after screen-off (LAB_NOTES 2026-07-11).
        // Without it, unlocked would survive the gap and the second window
        // would correctly open at the bare SCREEN_ON (the relight rule).
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(
                ev(10, SCREEN_ON), ev(10, UNLOCKED), ev(45, SCREEN_OFF),
                ev(46, LOCKED),
                ev(200, SCREEN_ON), ev(201, UNLOCKED), ev(230, LOCKED)
            ),
            nowMs = s(600)
        )
        assertEquals(listOf(s(10), s(40), s(45), s(201), s(230)), r.sampleTimesMs)
    }

    @Test
    fun eventsAtOrBeforeCursorAreIgnored() {
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(
                Synthesizer.Event(t0 - 5000, SCREEN_ON),
                Synthesizer.Event(t0, UNLOCKED)
            ),
            nowMs = s(600)
        )
        assertTrue(r.sampleTimesMs.isEmpty())
        assertFalse(r.next.interactive)
    }

    @Test
    fun eventsAfterNowAreIgnored() {
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(ev(10, SCREEN_ON), ev(12, UNLOCKED), ev(700, LOCKED)),
            nowMs = s(600)
        )
        // the LOCKED event lies beyond now; window closes at now instead
        assertEquals(listOf(s(12), s(42), s(72), s(102), s(132), s(162),
            s(192), s(222), s(252), s(282), s(312), s(342), s(372), s(402),
            s(432), s(462), s(492), s(522), s(552), s(582), s(600)),
            r.sampleTimesMs)
        assertTrue(r.next.interactive)
    }

    @Test
    fun noEventsNotInteractiveJustAdvancesCursor() {
        val r = Synthesizer.synthesize(cursor(), emptyList(), s(600))
        assertTrue(r.sampleTimesMs.isEmpty())
        assertEquals(Synthesizer.Cursor(s(600), screenOn = false, unlocked = false), r.next)
    }

    @Test
    fun redundantEventsDoNotSplitWindows() {
        // duplicate SCREEN_ON / UNLOCKED must not restart the grid
        val r = Synthesizer.synthesize(
            cursor(),
            listOf(
                ev(10, SCREEN_ON), ev(10, UNLOCKED),
                ev(20, SCREEN_ON), ev(25, UNLOCKED),
                ev(70, LOCKED)
            ),
            nowMs = s(600)
        )
        assertEquals(listOf(s(10), s(40), s(70)), r.sampleTimesMs)
    }
}
