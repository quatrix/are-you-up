package dev.areyouup.core

import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

// POSTs unsynced samples in batches until the queue drains. Rows are
// marked synced ONLY after the server's ack ({"accepted": N}) equals the
// batch size: a bare 200 is not an ack (captive portals answer 200 to
// anything), and a falsely-marked row becomes permanent data loss once
// pruning runs. Same rule as the mac client.
class Syncer(private val serverUrl: String, private val source: String) {

    companion object {
        const val BATCH_LIMIT = 1000
        private const val TIMEOUT_MS = 30_000
    }

    sealed class Outcome {
        data class Ok(val synced: Int) : Outcome()
        data class Failed(val synced: Int, val reason: String) : Outcome()
    }

    fun sync(queue: SampleQueue): Outcome {
        var total = 0
        while (true) {
            val batch = queue.nextBatch(BATCH_LIMIT)
            if (batch.isEmpty()) return Outcome.Ok(total)
            val error = postVerified(batch)
            if (error != null) return Outcome.Failed(total, error)
            queue.markSynced(batch.map { it.ts })
            total += batch.size
        }
    }

    // Returns null on verified success, else the failure reason. Never
    // throws: the job's dumb-retry loop only needs to know it failed.
    private fun postVerified(batch: List<Sample>): String? {
        val body = JSONObject()
            .put("source", source)
            .put("samples", JSONArray(batch.map { sample ->
                JSONObject().put("ts", sample.ts).put("idle_s", sample.idleS)
            }))
            .toString()
        return try {
            // trimEnd: a trailing slash in the configured URL would make
            // the path //v1/samples - a permanent, puzzling 404.
            val conn = URL("${serverUrl.trimEnd('/')}/v1/samples").openConnection() as HttpURLConnection
            try {
                conn.requestMethod = "POST"
                conn.connectTimeout = TIMEOUT_MS
                conn.readTimeout = TIMEOUT_MS
                conn.doOutput = true
                conn.setRequestProperty("Content-Type", "application/json")
                conn.outputStream.use { it.write(body.toByteArray()) }
                if (conn.responseCode != 200) {
                    return "server returned status ${conn.responseCode}"
                }
                val response = conn.inputStream.use { it.readBytes().decodeToString() }
                val accepted = try {
                    JSONObject(response).getInt("accepted")
                } catch (e: Exception) {
                    return "unparseable ack: ${response.take(120)}"
                }
                if (accepted != batch.size) {
                    return "ack mismatch: accepted=$accepted, sent=${batch.size}"
                }
                null
            } finally {
                conn.disconnect()
            }
        } catch (e: Exception) {
            "request failed: ${e.message}"
        }
    }
}
