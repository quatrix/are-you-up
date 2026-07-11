package dev.areyouup.core

import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper

// Local buffer between synthesis and sync - the same schema and rules as
// the mac client's store. Rows live here until the server acks them;
// pruning only ever touches synced rows, so an unreachable server never
// costs data.
class Store(context: Context, name: String? = "client.db") :
    SQLiteOpenHelper(context, name, null, 1), SampleQueue {

    init {
        // No effect on in-memory databases (tests); WAL on the device.
        setWriteAheadLoggingEnabled(true)
    }

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            "CREATE TABLE samples(" +
                "ts TEXT PRIMARY KEY, " +
                "idle_s INTEGER NOT NULL, " +
                "synced INTEGER NOT NULL DEFAULT 0)"
        )
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        // ponytail: no migrations - single-table schema at version 1, the
        // same stance as the other two parts (see SESSION.md).
    }

    fun insert(ts: String, idleS: Int) {
        writableDatabase.execSQL(
            "INSERT OR IGNORE INTO samples(ts, idle_s, synced) VALUES(?, ?, 0)",
            arrayOf(ts, idleS)
        )
    }

    override fun nextBatch(limit: Int): List<Sample> {
        val out = mutableListOf<Sample>()
        readableDatabase.rawQuery(
            // $limit is a Kotlin Int: interpolation is injection-safe here
            "SELECT ts, idle_s FROM samples WHERE synced = 0 ORDER BY ts LIMIT $limit",
            null
        ).use { c ->
            while (c.moveToNext()) out.add(Sample(c.getString(0), c.getInt(1)))
        }
        return out
    }

    override fun markSynced(tss: List<String>) {
        val db = writableDatabase
        db.beginTransaction()
        try {
            for (ts in tss) {
                db.execSQL("UPDATE samples SET synced = 1 WHERE ts = ?", arrayOf(ts))
            }
            db.setTransactionSuccessful()
        } finally {
            db.endTransaction()
        }
    }

    // TEXT comparison on ts is only approximately chronological across
    // offset changes - fine for housekeeping (documented stance shared
    // with the mac client), unsound for anything correctness-critical.
    fun pruneSynced(olderThanTs: String) {
        writableDatabase.execSQL(
            "DELETE FROM samples WHERE synced = 1 AND ts < ?",
            arrayOf(olderThanTs)
        )
    }

    fun unsyncedCount(): Int =
        readableDatabase.rawQuery("SELECT COUNT(*) FROM samples WHERE synced = 0", null)
            .use { c -> c.moveToFirst(); c.getInt(0) }
}
