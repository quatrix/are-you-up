package dev.areyouup.core

import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

// ADR-0004: every timestamp in the system is an RFC 3339 string carrying
// the device's local UTC offset, computed per instant so DST changes and
// travel produce the offset in effect at that moment. XXX renders
// +03:00-style offsets (and Z for UTC, which RFC 3339 also allows).
object Timestamps {
    private val formatter = DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm:ssXXX")

    fun format(epochMs: Long, zone: ZoneId = ZoneId.systemDefault()): String =
        Instant.ofEpochMilli(epochMs).atZone(zone).format(formatter)
}
