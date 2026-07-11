package dev.areyouup.core

import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

// Runs the real Syncer (real HttpURLConnection) against a real local
// HTTP server socket (MockWebServer) - no mocking of the transport.
// (com.sun.net.httpserver is not usable here: AGP compiles unit tests
// against android.jar, which omits JDK-internal modules.)
class SyncerTest {

    private var server: MockWebServer? = null
    private val requests = mutableListOf<String>()

    private fun startServer(handler: (callIndex: Int, body: String) -> Pair<Int, String>): String {
        val s = MockWebServer()
        s.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                // Parity with a path-routed server: anything but the
                // contract path is a 404, so a Syncer that POSTs to the
                // wrong path fails the ack check loudly.
                if (request.path != "/v1/samples") return MockResponse().setResponseCode(404)
                val body = request.body.readUtf8()
                val index: Int
                synchronized(requests) {
                    index = requests.size
                    requests.add(body)
                }
                val (code, response) = handler(index, body)
                return MockResponse().setResponseCode(code).setBody(response)
            }
        }
        s.start()
        server = s
        return "http://127.0.0.1:${s.port}"
    }

    @After
    fun tearDown() {
        server?.shutdown()
    }

    private fun accepting() = { _: Int, body: String ->
        val n = JSONObject(body).getJSONArray("samples").length()
        200 to """{"accepted": $n}"""
    }

    // unique, valid RFC 3339 timestamps: 30s apart starting 10:00:00
    private fun samples(n: Int) = (0 until n).map { i ->
        val total = i * 30
        Sample("2026-07-11T%02d:%02d:%02d+03:00".format(10 + total / 3600, (total / 60) % 60, total % 60), 0)
    }

    @Test
    fun drainsInBatchesOf1000AndMarksAllSynced() {
        val url = startServer(accepting())
        val queue = FakeQueue(samples(1500))
        val outcome = Syncer(url, "pixel").sync(queue)
        assertEquals(Syncer.Outcome.Ok(1500), outcome)
        assertTrue(queue.pending.isEmpty())
        assertEquals(1500, queue.synced.size)
        assertEquals(2, requests.size) // 1000 + 500
    }

    @Test
    fun sendsTheContractShape() {
        val url = startServer(accepting())
        Syncer(url, "pixel").sync(FakeQueue(listOf(Sample("2026-07-11T10:00:00+03:00", 4))))
        val req = JSONObject(requests.single())
        assertEquals("pixel", req.getString("source"))
        val sample = req.getJSONArray("samples").getJSONObject(0)
        assertEquals("2026-07-11T10:00:00+03:00", sample.getString("ts"))
        assertEquals(4, sample.getInt("idle_s"))
    }

    @Test
    fun emptyQueueSucceedsWithoutAnyRequest() {
        val outcome = Syncer("http://127.0.0.1:1", "pixel").sync(FakeQueue(emptyList()))
        assertEquals(Syncer.Outcome.Ok(0), outcome)
    }

    @Test
    fun non200LeavesRowsUnsynced() {
        val url = startServer { _, _ -> 500 to "boom" }
        val queue = FakeQueue(samples(3))
        assertTrue(Syncer(url, "pixel").sync(queue) is Syncer.Outcome.Failed)
        assertEquals(3, queue.pending.size)
        assertTrue(queue.synced.isEmpty())
    }

    @Test
    fun ackMismatchLeavesRowsUnsynced() {
        // a server (or middlebox) claiming fewer rows than sent
        val url = startServer { _, _ -> 200 to """{"accepted": 2}""" }
        val queue = FakeQueue(samples(3))
        assertTrue(Syncer(url, "pixel").sync(queue) is Syncer.Outcome.Failed)
        assertEquals(3, queue.pending.size)
    }

    @Test
    fun nonJson200LeavesRowsUnsynced() {
        // captive portals answer 200 to anything; a bare 200 is not an ack
        val url = startServer { _, _ -> 200 to "<html>welcome to hotel wifi</html>" }
        val queue = FakeQueue(samples(1))
        assertTrue(Syncer(url, "pixel").sync(queue) is Syncer.Outcome.Failed)
        assertEquals(1, queue.pending.size)
    }

    @Test
    fun connectionRefusedFailsGracefully() {
        val queue = FakeQueue(samples(1))
        val outcome = Syncer("http://127.0.0.1:1", "pixel").sync(queue)
        assertTrue(outcome is Syncer.Outcome.Failed)
        assertEquals(1, queue.pending.size)
    }

    @Test
    fun midDrainFailureKeepsEarlierProgress() {
        val url = startServer { index, body ->
            if (index == 0) {
                val n = JSONObject(body).getJSONArray("samples").length()
                200 to """{"accepted": $n}"""
            } else {
                500 to "boom"
            }
        }
        val queue = FakeQueue(samples(1500))
        val outcome = Syncer(url, "pixel").sync(queue)
        assertEquals(1000, (outcome as Syncer.Outcome.Failed).synced)
        assertEquals(500, queue.pending.size) // first batch marked, second kept
    }
}
