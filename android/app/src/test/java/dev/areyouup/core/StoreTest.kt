package dev.areyouup.core

import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class StoreTest {

    // name = null -> in-memory database, fresh per Store instance
    private fun store() = Store(RuntimeEnvironment.getApplication(), null)

    @Test
    fun insertIsIdempotentOnTs() {
        val s = store()
        s.insert("2026-07-11T10:00:00+03:00", 0)
        s.insert("2026-07-11T10:00:00+03:00", 0)
        assertEquals(1, s.unsyncedCount())
    }

    @Test
    fun reinsertingSyncedRowKeepsItSynced() {
        // Cursor-overlap replay re-inserts already-uploaded instants; OR
        // IGNORE must not reset synced=1 (OR REPLACE would re-upload them).
        val s = store()
        s.insert("2026-07-11T10:00:00+03:00", 0)
        s.markSynced(listOf("2026-07-11T10:00:00+03:00"))
        s.insert("2026-07-11T10:00:00+03:00", 0)
        assertEquals(0, s.unsyncedCount())
    }

    @Test
    fun nextBatchRespectsLimitAndTsOrder() {
        val s = store()
        s.insert("2026-07-11T10:00:30+03:00", 0)
        s.insert("2026-07-11T10:00:00+03:00", 0)
        s.insert("2026-07-11T10:01:00+03:00", 0)
        assertEquals(
            listOf("2026-07-11T10:00:00+03:00", "2026-07-11T10:00:30+03:00"),
            s.nextBatch(2).map { it.ts }
        )
    }

    @Test
    fun batchCarriesIdleSeconds() {
        val s = store()
        s.insert("2026-07-11T10:00:00+03:00", 7)
        assertEquals(listOf(Sample("2026-07-11T10:00:00+03:00", 7)), s.nextBatch(10))
    }

    @Test
    fun markSyncedRemovesFromUnsynced() {
        val s = store()
        s.insert("2026-07-11T10:00:00+03:00", 0)
        s.insert("2026-07-11T10:00:30+03:00", 0)
        s.markSynced(listOf("2026-07-11T10:00:00+03:00"))
        assertEquals(listOf("2026-07-11T10:00:30+03:00"), s.nextBatch(10).map { it.ts })
        assertEquals(1, s.unsyncedCount())
    }

    @Test
    fun pruneOnlyDeletesSyncedRowsOlderThanCutoff() {
        val s = store()
        s.insert("2026-07-01T10:00:00+03:00", 0) // old, synced -> pruned
        s.insert("2026-07-01T10:00:30+03:00", 0) // old, UNSYNCED -> must survive
        s.insert("2026-07-11T10:00:00+03:00", 0) // recent, synced -> survives
        s.markSynced(listOf("2026-07-01T10:00:00+03:00", "2026-07-11T10:00:00+03:00"))
        s.pruneSynced(olderThanTs = "2026-07-08T00:00:00+03:00")
        assertEquals(listOf("2026-07-01T10:00:30+03:00"), s.nextBatch(10).map { it.ts })
        val total = s.readableDatabase
            .rawQuery("SELECT COUNT(*) FROM samples", null)
            .use { c -> c.moveToFirst(); c.getInt(0) }
        assertEquals(2, total)
    }
}
