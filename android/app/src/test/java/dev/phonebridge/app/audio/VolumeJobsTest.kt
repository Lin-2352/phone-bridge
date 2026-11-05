package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class VolumeJobsTest {
    private val queued = ArrayList<Runnable>()
    private val jobs = VolumeJobs({ queued.add(it) })

    @Test
    fun submit_returns_before_the_job_runs() {
        val ran = ArrayList<String>()
        jobs.submit { ran.add("restore") }
        assertTrue("nothing runs on the caller's thread", ran.isEmpty())
        queued.forEach { it.run() }
        assertEquals(listOf("restore"), ran)
    }

    @Test
    fun jobs_run_in_submit_order() {
        // A Stop followed at once by a new share: the restore must finish before the next begin.
        val ran = ArrayList<String>()
        jobs.submit { ran.add("restore") }
        jobs.submit { ran.add("begin") }
        queued.forEach { it.run() }
        assertEquals(listOf("restore", "begin"), ran)
    }

    @Test
    fun a_failing_job_does_not_stop_the_next() {
        val ran = ArrayList<String>()
        jobs.submit { throw IllegalStateException("effect refused") }
        jobs.submit { ran.add("begin") }
        queued.forEach { it.run() }
        assertEquals(listOf("begin"), ran)
    }

    /**
     * Merged 2026-11-06: silencing must be done before the PC is told "yes" (the first sound then finds the
     * phone muted), and must never overtake a restore still queued from the share before.
     */
    @Test
    fun run_and_wait_runs_after_queued_jobs_and_returns_once_done() {
        val pool = java.util.concurrent.Executors.newSingleThreadExecutor()
        val real = VolumeJobs({ pool.execute(it) })
        val ran = java.util.Collections.synchronizedList(ArrayList<String>())
        real.submit { Thread.sleep(50); ran.add("restore") }
        val done = real.runAndWait(2_000) { ran.add("begin") }
        assertTrue(done)
        assertEquals(listOf("restore", "begin"), ran.toList())
        pool.shutdown()
    }

    @Test
    fun run_and_wait_gives_up_after_its_timeout_but_the_job_still_runs_in_order() {
        val pool = java.util.concurrent.Executors.newSingleThreadExecutor()
        val real = VolumeJobs({ pool.execute(it) })
        val ran = java.util.Collections.synchronizedList(ArrayList<String>())
        real.submit { Thread.sleep(300); ran.add("slow") }
        assertEquals(false, real.runAndWait(50) { ran.add("begin") })
        pool.shutdown()
        pool.awaitTermination(2, java.util.concurrent.TimeUnit.SECONDS)
        assertEquals(listOf("slow", "begin"), ran.toList())
    }
}
