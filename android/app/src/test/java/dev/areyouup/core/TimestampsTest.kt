package dev.areyouup.core

import org.junit.Assert.assertEquals
import org.junit.Test
import java.time.ZoneId

class TimestampsTest {

    // 1783764000000 ms = 2026-07-11T10:00:00Z (fixed instant used throughout)

    @Test
    fun formatsSummerPositiveOffset() {
        assertEquals(
            "2026-07-11T13:00:00+03:00",
            Timestamps.format(1_783_764_000_000L, ZoneId.of("Asia/Jerusalem"))
        )
    }

    @Test
    fun formatsWinterOffsetOfSameZone() {
        // 1768471200000 ms = 2026-01-15T10:00:00Z; Israel is +02:00 in January.
        // The offset must reflect the zone rules AT the instant (ADR-0004).
        assertEquals(
            "2026-01-15T12:00:00+02:00",
            Timestamps.format(1_768_471_200_000L, ZoneId.of("Asia/Jerusalem"))
        )
    }

    @Test
    fun formatsNegativeOffset() {
        assertEquals(
            "2026-07-11T06:00:00-04:00",
            Timestamps.format(1_783_764_000_000L, ZoneId.of("America/New_York"))
        )
    }

    @Test
    fun formatsUtcAsZ() {
        // RFC 3339 allows Z for +00:00; the backend's chrono parser accepts it.
        assertEquals(
            "2026-07-11T10:00:00Z",
            Timestamps.format(1_783_764_000_000L, ZoneId.of("UTC"))
        )
    }

    @Test
    fun truncatesSubSecondPrecision() {
        assertEquals(
            "2026-07-11T13:00:00+03:00",
            Timestamps.format(1_783_764_000_999L, ZoneId.of("Asia/Jerusalem"))
        )
    }
}
