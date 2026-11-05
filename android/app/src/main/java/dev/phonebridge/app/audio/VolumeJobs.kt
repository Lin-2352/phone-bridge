package dev.phonebridge.app.audio

import java.util.concurrent.Executors

/**
 * Volume work (silencer start and stop, stepping the volume, restoring it) takes a 100 ms settle
 * plus one binder call per volume step. It runs here, one job at a time and in the order asked,
 * never on the main thread and never under the share's lock. A failing job never stops the next.
 */
class VolumeJobs(private val executor: (Runnable) -> Unit, private val onError: (Throwable) -> Unit = {}) {
    fun submit(job: () -> Unit) = executor(Runnable { runCatching(job).onFailure(onError) })

    /** Queues [job] like [submit] and waits up to [timeoutMs] for it. False: still pending (it will run, in order). */
    fun runAndWait(timeoutMs: Long, job: () -> Unit): Boolean {
        val done = java.util.concurrent.CountDownLatch(1)
        submit {
            try {
                job()
            } finally {
                done.countDown()
            }
        }
        return done.await(timeoutMs, java.util.concurrent.TimeUnit.MILLISECONDS)
    }

    companion object {
        val shared: VolumeJobs by lazy {
            val pool = Executors.newSingleThreadExecutor { r -> Thread(r, "pb-audio-vol") }
            VolumeJobs({ pool.execute(it) }) { android.util.Log.w("PhoneBridgeAudio", "AUDIO volume job failed: $it") }
        }
    }
}
