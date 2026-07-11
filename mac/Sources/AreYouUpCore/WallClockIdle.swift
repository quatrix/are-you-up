import Foundation

/// Converts the awake-time idle stopwatch into wall-clock seconds since
/// the last input event.
///
/// `CGEventSource.secondsSinceLastEventType` pauses while the machine
/// sleeps, so during closed-lid dark wakes it reports tens of seconds
/// when the true answer is hours, and the server classifies the sample
/// as active (LAB_NOTES 2026-07-11, ADR-0008).
///
/// The conversion needs no sleep/wake notifications (dark wakes do not
/// reliably post them): `ProcessInfo.systemUptime` pauses during sleep
/// in exactly the same way, so with no input the stopwatch grows by
/// precisely the uptime delta between ticks. A stopwatch reading below
/// that growth means an input event reset it, and `now - raw` pins the
/// event to wall clock (accurate because the event lies within the
/// current awake span).
public struct WallClockIdle {
    /// Jitter allowance between reading the stopwatch and the uptime
    /// clock in one tick; anything below one sample period works.
    private static let slackS: TimeInterval = 2

    private var lastInputDate: Date?
    private var prevRaw: TimeInterval = 0
    private var prevUptime: TimeInterval = 0

    public init() {}

    /// Feed one sample tick; returns wall-clock seconds since last input.
    /// Cadence does not matter (the comparison is against uptime delta,
    /// not the sample period), but callers should feed every reading so
    /// no stopwatch reset is missed.
    public mutating func tick(raw: TimeInterval, uptime: TimeInterval, now: Date) -> Int {
        let grownWithoutInput = prevRaw + (uptime - prevUptime)
        let anchor: Date
        if let last = lastInputDate, raw >= grownWithoutInput - Self.slackS {
            anchor = last
        } else {
            // First tick, or the stopwatch reset: real input happened
            // `raw` awake-seconds ago, which is also `raw` wall-seconds
            // ago within the current awake span.
            anchor = now.addingTimeInterval(-raw)
            lastInputDate = anchor
        }
        prevRaw = raw
        prevUptime = uptime
        return max(0, Int(now.timeIntervalSince(anchor).rounded()))
    }
}
