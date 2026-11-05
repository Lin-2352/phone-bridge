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
}
