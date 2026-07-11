package dev.areyouup.core

data class Sample(val ts: String, val idleS: Int)

// The syncer's view of the store. Exists so the drain loop is testable
// on the plain JVM with an in-memory fake; Store itself needs Robolectric
// (android sqlite classes).
interface SampleQueue {
    fun nextBatch(limit: Int): List<Sample>
    fun markSynced(tss: List<String>)
}
