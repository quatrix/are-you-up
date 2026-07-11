import XCTest
@testable import AreYouUpCore

// The raw CGEventSource idle stopwatch runs on AWAKE time: it pauses
// during sleep, so a closed lid dark-waking hourly reports idle_s that
// creeps +30s per wake and never crosses the server's 900s threshold
// (LAB_NOTES 2026-07-11). WallClockIdle converts it to wall-clock
// seconds since last input. Ticks inject (raw, uptime, now) so tests
// can replay real observed data; uptime, like the stopwatch, advances
// only while awake.
final class WallClockIdleTests: XCTestCase {

    // Replay of the actual 2026-07-11 afternoon from the client db that
    // exposed the bug: lid closed at ~16:00 with idle_s=5, dark wakes at
    // 16:17 / 17:59 / 19:56 each advancing the stopwatch by exactly one
    // 30s tick of awake time.
    func testDarkWakeReportsWallClockIdleNotFrozenStopwatch() {
        var idle = WallClockIdle()
        let lidClose = Date(timeIntervalSince1970: 1_800_000_000)

        // 16:00:05: real use, input 5s ago.
        XCTAssertEqual(idle.tick(raw: 5, uptime: 1_000, now: lidClose), 5)

        // 16:17:28 dark wake (1043s of wall clock, only 30s of it awake):
        // stopwatch says 35, but nobody has touched the machine since
        // 16:00:00 -> 1048s of wall-clock idle.
        XCTAssertEqual(
            idle.tick(raw: 35, uptime: 1_030, now: lidClose.addingTimeInterval(1_043)),
            1_048)

        // 17:59:13 dark wake: stopwatch crept to just 65.
        XCTAssertEqual(
            idle.tick(raw: 65, uptime: 1_060, now: lidClose.addingTimeInterval(7_148)),
            7_153)

        // 19:56:23: user comes home, stopwatch resets to 0 -> active again.
        XCTAssertEqual(
            idle.tick(raw: 0, uptime: 1_090, now: lidClose.addingTimeInterval(14_178)),
            0)
    }

    // While awake, uptime and wall clock advance together, so the raw
    // stopwatch is already wall-accurate and must pass through unchanged.
    func testAwakeIdleMatchesRawStopwatch() {
        var idle = WallClockIdle()
        let t0 = Date(timeIntervalSince1970: 1_800_000_000)
        XCTAssertEqual(idle.tick(raw: 5, uptime: 100, now: t0), 5)
        // No input for two ticks: raw grows exactly with uptime.
        XCTAssertEqual(idle.tick(raw: 35, uptime: 130, now: t0.addingTimeInterval(30)), 35)
        XCTAssertEqual(idle.tick(raw: 965, uptime: 1_060, now: t0.addingTimeInterval(960)), 965)
    }

    func testInputWhileAwakeResetsIdle() {
        var idle = WallClockIdle()
        let t0 = Date(timeIntervalSince1970: 1_800_000_000)
        XCTAssertEqual(idle.tick(raw: 25, uptime: 100, now: t0), 25)
        // Stopwatch fell below pure awake-time growth (25+30): input.
        XCTAssertEqual(idle.tick(raw: 3, uptime: 130, now: t0.addingTimeInterval(30)), 3)
    }

    // Lid opened after a long sleep and the user types immediately: the
    // stopwatch reset is detected against pre-sleep state.
    func testInputRightAfterWakeIsActive() {
        var idle = WallClockIdle()
        let t0 = Date(timeIntervalSince1970: 1_800_000_000)
        XCTAssertEqual(idle.tick(raw: 215, uptime: 100, now: t0), 215)
        // 2h of wall clock, 60s of it awake; user typed 2s ago.
        XCTAssertEqual(idle.tick(raw: 2, uptime: 160, now: t0.addingTimeInterval(7_200)), 2)
    }

    // Sub-second jitter between reading the stopwatch and the uptime
    // clock must not read as input.
    func testSmallJitterIsNotInput() {
        var idle = WallClockIdle()
        let t0 = Date(timeIntervalSince1970: 1_800_000_000)
        XCTAssertEqual(idle.tick(raw: 5, uptime: 100, now: t0), 5)
        XCTAssertEqual(idle.tick(raw: 34, uptime: 130, now: t0.addingTimeInterval(30)), 35)
    }
}
