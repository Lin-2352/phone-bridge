package dev.phonebridge.app.audio

/**
 * Fixed-size ring of 16-bit samples between the network thread (push) and the AudioTrack thread
 * (read). Plays silence until [targetMs] is buffered, drops the oldest audio above [maxMs]
 * (down to the target), and re-primes after an underrun. No allocation after construction.
 * All methods are synchronized; the critical sections only copy.
 */
class JitterBuffer(
    private val sampleRate: Int,
    private val channels: Int,
    targetMs: Int = 120,
    maxMs: Int = 300,
) {
    private val shortsPerMs = sampleRate * channels / 1000
    private val targetShorts = targetMs * shortsPerMs
    private val maxShorts = maxMs * shortsPerMs
    private val ring = ShortArray(maxShorts + shortsPerMs * 20)
    private var head = 0 // read index
    private var size = 0
    private var primed = false

    @Volatile var underruns = 0L
        private set

    @Volatile var droppedMs = 0L
        private set

    @Synchronized
    fun bufferedMs(): Int = size / shortsPerMs

    @Synchronized
    fun clear() {
        head = 0
        size = 0
        primed = false
    }

    /** Appends [len] shorts of [src] (whole sample frames). Drops the oldest audio above the max. */
    @Synchronized
    fun push(src: ShortArray, len: Int) {
        var from = 0
        var n = len - len % channels
        if (n > maxShorts) { // a huge push: only its newest part can matter
            droppedMs += (n - maxShorts) / shortsPerMs
            from = n - maxShorts
            n = maxShorts
        }
        val cap = ring.size
        var w = (head + size) % cap
        var left = n
        var i = from
        while (left > 0) {
            val chunk = minOf(left, cap - w)
            System.arraycopy(src, i, ring, w, chunk)
            w = (w + chunk) % cap
            i += chunk
            left -= chunk
        }
        size += n
        if (size > maxShorts) {
            val drop = size - targetShorts
            val aligned = drop - drop % channels
            head = (head + aligned) % cap
            size -= aligned
            droppedMs += aligned / shortsPerMs
        }
        if (!primed && size >= targetShorts) primed = true
    }

    /**
     * Fills [dst] with [len] shorts. Returns how many were real audio; the rest of [len] is zeroed
     * (silence). Before the prebuffer is reached returns 0 without counting an underrun.
     */
    @Synchronized
    fun read(dst: ShortArray, len: Int): Int {
        if (!primed) {
            java.util.Arrays.fill(dst, 0, len, 0)
            return 0
        }
        val n = minOf(len, size)
        val cap = ring.size
        var left = n
        var r = head
        var o = 0
        while (left > 0) {
            val chunk = minOf(left, cap - r)
            System.arraycopy(ring, r, dst, o, chunk)
            r = (r + chunk) % cap
            o += chunk
            left -= chunk
        }
        head = r
        size -= n
        if (n < len) {
            java.util.Arrays.fill(dst, n, len, 0)
            underruns++
            primed = false
        }
        return n
    }
}
