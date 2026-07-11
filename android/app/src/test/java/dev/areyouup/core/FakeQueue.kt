package dev.areyouup.core

class FakeQueue(samples: List<Sample>) : SampleQueue {
    val pending = samples.toMutableList()
    val synced = mutableListOf<String>()

    override fun nextBatch(limit: Int): List<Sample> = pending.take(limit)

    override fun markSynced(tss: List<String>) {
        synced += tss
        pending.removeAll { it.ts in tss }
    }
}
