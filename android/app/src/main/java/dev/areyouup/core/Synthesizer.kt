package dev.areyouup.core

// ==========================================================================
// Sample synthesis from the system usage-event log
// ==========================================================================
//
// ADR-0007: instead of observing the screen live, the app periodically
// replays the screen/keyguard events Android records anyway and
// reconstructs "the owner was using the phone" windows after the fact.
// This object is that reconstruction: pure, clock-free (the caller passes
// nowMs), and the only real algorithm in the app.
object Synthesizer {

    // Matches the mac client's cadence; the server merges consecutive
    // same-state samples while gaps stay <= 90s (3x this period).
    const val SAMPLE_PERIOD_MS = 30_000L

    // ADR-0006: the phone counts as active while the screen is on AND the
    // keyguard is dismissed. The two bits are tracked separately because
    // they change independently: re-lighting the screen inside the lock
    // delay fires screen events but no keyguard events.
    data class Cursor(val tsMs: Long, val screenOn: Boolean, val unlocked: Boolean) {
        val interactive: Boolean get() = screenOn && unlocked
    }

    data class Event(val tsMs: Long, val kind: Kind) {
        enum class Kind { SCREEN_ON, SCREEN_OFF, UNLOCKED, LOCKED, SHUTDOWN }
    }

    data class Result(val sampleTimesMs: List<Long>, val next: Cursor)

    // Replays `events` (sorted ascending by tsMs; events at or before the
    // cursor, or after nowMs, are ignored) and returns the sample instants
    // for every interactive window inside (cursor.tsMs, nowMs], plus the
    // cursor for the next run. A window still open at nowMs emits samples
    // up to nowMs and stays "interactive" in the returned cursor, so the
    // next run continues it seamlessly; the duplicate boundary sample this
    // produces is absorbed by INSERT OR IGNORE locally and the upsert
    // server-side.
    fun synthesize(cursor: Cursor, events: List<Event>, nowMs: Long): Result {
        var screenOn = cursor.screenOn
        var unlocked = cursor.unlocked
        var windowStart = if (cursor.interactive) cursor.tsMs else -1L
        val samples = mutableListOf<Long>()

        for (e in events) {
            if (e.tsMs <= cursor.tsMs || e.tsMs > nowMs) continue
            val wasInteractive = screenOn && unlocked
            when (e.kind) {
                Event.Kind.SCREEN_ON -> screenOn = true
                Event.Kind.SCREEN_OFF -> screenOn = false
                Event.Kind.UNLOCKED -> unlocked = true
                Event.Kind.LOCKED -> unlocked = false
                Event.Kind.SHUTDOWN -> { screenOn = false; unlocked = false }
            }
            val isInteractive = screenOn && unlocked
            if (!wasInteractive && isInteractive) {
                windowStart = e.tsMs
            } else if (wasInteractive && !isInteractive) {
                emitGrid(samples, windowStart, e.tsMs)
                windowStart = -1L
            }
        }
        if (windowStart >= 0) emitGrid(samples, windowStart, nowMs)
        return Result(samples, Cursor(nowMs, screenOn, unlocked))
    }

    // Samples at start, start+30s, start+60s, ... plus one at end, so the
    // server-observed interval spans the full window (no extrapolation).
    private fun emitGrid(out: MutableList<Long>, startMs: Long, endMs: Long) {
        var t = startMs
        while (t < endMs) {
            out.add(t)
            t += SAMPLE_PERIOD_MS
        }
        out.add(endMs)
    }
}
