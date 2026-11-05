package dev.phonebridge.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.media.AudioPlaybackConfiguration
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import dev.phonebridge.app.audio.PC_LEVEL_STEPS
import dev.phonebridge.app.audio.PcLevel
import dev.phonebridge.app.audio.ROUTE_LOOK_MS
import dev.phonebridge.app.audio.RouteActions
import dev.phonebridge.app.audio.SoundRoute
import dev.phonebridge.app.audio.SpeakerAction
import dev.phonebridge.app.audio.SpeakerGuard
import dev.phonebridge.app.audio.capturedSound
import dev.phonebridge.app.audio.isEarphones
import dev.phonebridge.app.audio.playerIds
import dev.phonebridge.app.audio.rememberedStep
import dev.phonebridge.app.audio.shareStartStep
import dev.phonebridge.app.audio.silenceWanted
import dev.phonebridge.app.audio.volumeGoesBack
import java.util.concurrent.atomic.AtomicInteger

/**
 * "Silence this phone while streaming". The media stream is muted from the moment a share starts,
 * which reaches every output, and each new sound finds it so: waiting to notice a game would let it
 * be heard first. The mute is always given back: on Stop, on a dropped link, and on the next start
 * after a crash (the flags in [Prefs]). The setting applies at once, also in the middle of a stream.
 * If muting proves to silence the capture too ([SpeakerGuard]), it unmutes, remembers that, and says
 * so: never a toggle that quietly sends nothing.
 *
 * The volume keys set the PC's level in every app. [SoundRoute] holds the one rule for that, which
 * knows nothing about particular apps: sound the effect of [SpeakerSilencer] covers gets the phone's
 * own volume bar, and sound it misses stays muted with [PcVolumeKeys]' cast bar. Everything moves the
 * volume in relative steps: this phone ignores setting a value while another app is in front.
 *
 * Earphones are silenced too unless the user turned that off, in which case they play and only the
 * speaker is kept silent. Android keeps a volume per output, so a saved volume waits for its own
 * output to come back rather than being walked onto another, where it could be painfully loud.
 */
object PhoneSpeaker {
    private const val TAG = "PhoneBridgeAudio"

    /** System calls cost milliseconds: the capture thread makes them this rarely. */
    private const val SYSTEM_CHECK_FRAMES = 50
    private const val HELD_INDEX = 1

    /** A missed report must not leave the phone playing: every this many looks, the mute is checked. */
    private const val MUTE_CHECK_LOOKS = 4
    private const val VOLUME_CHANGED = "android.media.VOLUME_CHANGED_ACTION"
    private const val MUTE_CHANGED = "android.media.STREAM_MUTE_CHANGED_ACTION"
    private const val EXTRA_STREAM = "android.media.EXTRA_VOLUME_STREAM_TYPE"
    private const val EXTRA_VALUE = "android.media.EXTRA_VOLUME_STREAM_VALUE"
    private const val EXTRA_PREV = "android.media.EXTRA_PREV_VOLUME_STREAM_VALUE"

    /** Read on the capture thread, which must never wait for the volume work on the main thread. */
    @Volatile private var guard: SpeakerGuard? = null
    private var route: SoundRoute? = null
    private var app: Context? = null
    private var sharing = false
    private var frameMs = 10
    private var frames = 0
    private var musicActive = false
    private var listening = false
    private var looks = 0
    private var watchingOutputs = false
    private var earphonesOn = false
    private val level = PcLevel(PC_LEVEL_STEPS)
    private val main = Handler(Looper.getMainLooper())
    private val audibleFrames = AtomicInteger()
    private val seenFrames = AtomicInteger()

    /** Android reports every change of the stream, so a press is dealt with in milliseconds. */
    private val changes = object : BroadcastReceiver() {
        override fun onReceive(ctx: Context, intent: Intent) {
            if (intent.action == VOLUME_CHANGED) {
                if (intent.getIntExtra(EXTRA_STREAM, -1) != AudioManager.STREAM_MUSIC) return
                val index = intent.getIntExtra(EXTRA_VALUE, HELD_INDEX)
                onIndex(ctx, intent.getIntExtra(EXTRA_PREV, index), index)
            }
            holdMute(ctx)
        }
    }

    /** Fires as a player starts or stops, which is the moment to be muted if it may be a game. */
    private val players = object : AudioManager.AudioPlaybackCallback() {
        override fun onPlaybackConfigChanged(configs: MutableList<AudioPlaybackConfiguration>) {
            synchronized(this@PhoneSpeaker) {
                val ctx = app ?: return
                route?.let { r ->
                    val ids = mediaPlayers(configs)
                    val before = r.mode
                    act(ctx, r.onPlayers(ids))
                    Log.i(TAG, "AUDIO players now $ids, sound route $before to ${r.mode}")
                }
            }
        }
    }

    private val outputs = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>) = outputsChanged()

        override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) = outputsChanged()
    }

    private val look = object : Runnable {
        override fun run() {
            synchronized(this@PhoneSpeaker) {
                val ctx = app ?: return
                val captured = capturedSound(audibleFrames.getAndSet(0), seenFrames.getAndSet(0))
                route?.let { r ->
                    val before = r.mode
                    act(ctx, r.onLook(captured, SpeakerSilencer.mixIsAudible()))
                    if (r.mode != before) {
                        Log.i(TAG, "AUDIO sound route $before to ${r.mode}, captured=$captured, mix=${SpeakerSilencer.lastPeakMb} mB, pc level step=${r.level}")
                    }
                }
                if (++looks % MUTE_CHECK_LOOKS == 0) holdMute(ctx)
                main.postDelayed(this, ROUTE_LOOK_MS)
            }
        }
    }

    private fun mediaPlayers(configs: List<AudioPlaybackConfiguration>): Set<Int> =
        playerIds(
            configs.filter {
                when (it.audioAttributes.usage) {
                    AudioAttributes.USAGE_MEDIA, AudioAttributes.USAGE_GAME, AudioAttributes.USAGE_UNKNOWN -> true
                    else -> false
                }
            }.map { it.toString() },
        )

    private fun audio(ctx: Context) = ctx.getSystemService(AudioManager::class.java)

    private fun adjust(ctx: Context, how: Int) {
        runCatching { audio(ctx).adjustStreamVolume(AudioManager.STREAM_MUSIC, how, 0) }
            .onFailure { Log.w(TAG, "AUDIO speaker change refused: $it") }
    }

    /** Mutes or unmutes the media stream. */
    fun setStreamMuted(ctx: Context, muted: Boolean) {
        adjust(ctx, if (muted) AudioManager.ADJUST_MUTE else AudioManager.ADJUST_UNMUTE)
    }

    /** [count] relative steps of the phone's volume, up when positive. */
    fun stepIndex(ctx: Context, count: Int) {
        val how = if (count > 0) AudioManager.ADJUST_RAISE else AudioManager.ADJUST_LOWER
        repeat(kotlin.math.abs(count)) { adjust(ctx, how) }
    }

    /** The captured sound at the level the PC should hear. Capture thread. */
    fun applyLevel(buf: ByteArray, len: Int, channels: Int) = level.apply(buf, len, channels)

    private fun earphonesConnected(ctx: Context): Boolean =
        audio(ctx).getDevices(AudioManager.GET_DEVICES_OUTPUTS).any { isEarphones(it.type) }

    private fun silenceWanted(ctx: Context): Boolean {
        val p = Prefs(ctx)
        return silenceWanted(p.audioSilencePhone, p.audioSilenceEarphones, earphonesOn)
    }

    private fun watchOutputs(ctx: Context) {
        app = ctx.applicationContext
        earphonesOn = earphonesConnected(ctx)
        if (watchingOutputs) return
        watchingOutputs = true
        audio(ctx).registerAudioDeviceCallback(outputs, main)
    }

    /** Earphones may have come or gone. Android calls this for everything, including at the start. */
    @Synchronized
    private fun outputsChanged() {
        val ctx = app ?: return
        val now = earphonesConnected(ctx)
        if (now == earphonesOn) return
        earphonesOn = now
        Log.i(TAG, "AUDIO earphones ${if (now) "connected" else "gone"}")
        val wanted = silenceWanted(ctx)
        when {
            !sharing -> {}
            wanted && listening -> route?.let { act(ctx, it.onRouteChanged()) }
            wanted -> apply(ctx)
            else -> stopMuting(ctx)
        }
        walkBackIfOwed(ctx)
    }

    /** A share of this phone's audio started. */
    @Synchronized
    fun begin(ctx: Context, frameMs: Int) {
        sharing = true
        this.frameMs = frameMs
        watchOutputs(ctx)
        apply(ctx)
    }

    /** The setting changed: takes effect now if a share is running. */
    @Synchronized
    fun settingChanged(ctx: Context) {
        if (!sharing) return
        if (silenceWanted(ctx)) apply(ctx) else stopMuting(ctx)
    }

    private fun apply(ctx: Context) {
        val p = Prefs(ctx)
        if (!silenceWanted(ctx)) {
            Log.i(TAG, "AUDIO phone stays audible: silencing is off${if (p.audioSilencePhone) " for earphones" else ""}")
            return
        }
        if (listening) return
        if (p.audioSilenceUnsupported) {
            Log.i(TAG, "AUDIO phone speaker stays on: muting silenced the capture before")
            return
        }
        val am = audio(ctx)
        val appCtx = ctx.applicationContext
        app = appCtx
        val ownMute = am.isStreamMute(AudioManager.STREAM_MUSIC)
        if (ownMute) {
            // The user's own mute: not ours to undo later, and its volume can't be read to restore.
            Log.i(TAG, "AUDIO phone speaker was already muted")
        } else {
            // Saved before anything changes: a crash from here on still gets it back.
            walkBackIfOwed(ctx)
            val index = am.getStreamVolume(AudioManager.STREAM_MUSIC)
            p.audioHeldIndex = index
            p.audioHeldOnEarphones = earphonesOn
            p.audioMutedByRelay = true
            guard = SpeakerGuard(frameMs, proven = p.audioMuteKeepsCapture)
            // Muted first: lowering does not unmute, so the walk down is not heard.
            adjust(ctx, AudioManager.ADJUST_MUTE)
            stepIndex(ctx, HELD_INDEX - index)
            adjust(ctx, AudioManager.ADJUST_MUTE)
            Log.i(TAG, "AUDIO phone speaker muted, held at $HELD_INDEX from $index")
        }
        SpeakerSilencer.start()
        level.step = startStep(p, am)
        // The opt-in key filter: the share counts as a session for Relay control, which switches off after it.
        val filter = Prefs(appCtx).audioKeysViaControl && ControlService.instance != null
        PcVolumeKeys.heldByFilter = filter
        if (filter) ControlService.instance?.noteSession()
        PcVolumeKeys.start(appCtx, level.step) { step ->
            level.step = step
            savePcLevel(appCtx, step)
            synchronized(this) { route?.let { act(appCtx, it.setLevel(step)) } }
        }
        if (!ownMute) {
            val r = SoundRoute(HELD_INDEX, am.getStreamMaxVolume(AudioManager.STREAM_MUSIC), level.step, SpeakerSilencer.canCover())
            route = r
            r.onPlayers(mediaPlayers(am.activePlaybackConfigurations))
            am.registerAudioPlaybackCallback(players, main)
        }
        listen(ctx)
        audibleFrames.set(0)
        seenFrames.set(0)
        main.postDelayed(look, ROUTE_LOOK_MS)
    }

    /** Where this share starts, once taking over what an earlier version kept in the volume bar. */
    private fun startStep(p: Prefs, am: AudioManager): Int {
        val old = p.audioShareIndex
        val step = shareStartStep(p.audioPcLevel, old, am.getStreamMaxVolume(AudioManager.STREAM_MUSIC))
        if (old >= 0) {
            p.audioPcLevel = step
            p.audioShareIndex = -1
        }
        return step
    }

    private fun savePcLevel(ctx: Context, step: Int) {
        val p = Prefs(ctx)
        p.audioPcLevel = rememberedStep(p.audioPcLevel, step)
    }

    private fun listen(ctx: Context) {
        val filter = IntentFilter(VOLUME_CHANGED).apply { addAction(MUTE_CHANGED) }
        val app = ctx.applicationContext
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            app.registerReceiver(changes, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            app.registerReceiver(changes, filter)
        }
        listening = true
    }

    /** The phone's volume moved. */
    @Synchronized
    private fun onIndex(ctx: Context, prev: Int, index: Int) {
        if (!listening || guard?.isChecking == true) return
        val r = route ?: return
        val before = r.mode
        act(ctx, r.onReport(prev, index))
        if (r.mode != before) Log.i(TAG, "AUDIO a volume key moved the real volume, index $prev to $index, sound route $before to ${r.mode}, pc level step=${r.level}")
    }

    /** Does what [SoundRoute] says. A walk down under the mute isn't heard, and a raise unmutes, so the mute goes on both ways round. */
    private fun act(ctx: Context, a: RouteActions) {
        if (a.freshMeter) SpeakerSilencer.freshMeter()
        if (a.muted != null) setStreamMuted(ctx, a.muted)
        stepIndex(ctx, a.moveIndex)
        if (a.muted == true && a.moveIndex > 0) setStreamMuted(ctx, true)
        a.level?.let { step ->
            level.step = step
            savePcLevel(ctx, step)
            PcVolumeKeys.setStep(step)
        }
        a.castBar?.let { PcVolumeKeys.setActive(it) }
        if (a.reclaimKeys) PcVolumeKeys.reclaim()
    }

    @Synchronized
    private fun holdMute(ctx: Context) {
        if (!listening || guard?.isChecking == true) return
        if (route?.muteHeld == false) return
        if (!audio(ctx).isStreamMute(AudioManager.STREAM_MUSIC)) adjust(ctx, AudioManager.ADJUST_MUTE)
    }

    /**
     * Every captured frame, from the capture thread. Returns true when the capture just proved to
     * die with the mute, so the owner can tell the user. Once that question is answered, as it soon
     * is, this only counts: the capture thread must not wait for a lock or a system call.
     */
    fun onFrame(ctx: Context, silent: Boolean): Boolean {
        seenFrames.incrementAndGet()
        if (!silent) audibleFrames.incrementAndGet()
        val g = guard ?: return false
        return if (g.proven) false else askGuard(ctx, g, silent)
    }

    @Synchronized
    private fun askGuard(ctx: Context, g: SpeakerGuard, silent: Boolean): Boolean {
        if (guard !== g) return false
        val wasProven = g.proven
        if (++frames >= SYSTEM_CHECK_FRAMES) {
            frames = 0
            musicActive = audio(ctx).isMusicActive
        }
        val action = g.onFrame(silent, musicActive)
        if (g.proven && !wasProven) {
            Prefs(ctx).audioMuteKeepsCapture = true
            Log.i(TAG, "AUDIO sound captured through the mute: never unmuting to check again")
        }
        when (action) {
            SpeakerAction.NONE -> return false
            SpeakerAction.UNMUTE_TO_CHECK -> {
                Log.i(TAG, "AUDIO silence while muted: unmuting to check the capture")
                adjust(ctx, AudioManager.ADJUST_UNMUTE)
                return false
            }
            SpeakerAction.MUTE_AGAIN -> {
                Log.i(TAG, "AUDIO the content is just quiet: muting again")
                adjust(ctx, AudioManager.ADJUST_MUTE)
                return false
            }
            SpeakerAction.GIVE_UP -> {
                Log.i(TAG, "AUDIO muting silences the capture on this phone: speaker stays on")
                Prefs(ctx).audioSilenceUnsupported = true
                stopMuting(ctx)
                return true
            }
        }
    }

    private fun stopMuting(ctx: Context) {
        main.removeCallbacks(look)
        runCatching { audio(ctx).unregisterAudioPlaybackCallback(players) }
        guard = null
        route = null
        PcVolumeKeys.stop()
        if (PcVolumeKeys.heldByFilter) {
            PcVolumeKeys.heldByFilter = false
            // Banking apps refuse to open while Relay control is on: it goes off once the share is over.
            ControlService.instance?.maybeSwitchOff()
        }
        if (listening) {
            listening = false
            runCatching { ctx.applicationContext.unregisterReceiver(changes) }
        }
        // The phone plays again, so nothing may lower what the PC hears.
        level.step = PC_LEVEL_STEPS
        if (Prefs(ctx).audioMutedByRelay) giveBack(ctx)
        // After the walk back: until then the effect covers the apps it reaches.
        SpeakerSilencer.stop(ctx)
    }

    /** At session end, and at every service start for the crash case. */
    @Synchronized
    fun restore(ctx: Context) {
        sharing = false
        watchOutputs(ctx)
        stopMuting(ctx)
    }

    private fun giveBack(ctx: Context) {
        val p = Prefs(ctx)
        adjust(ctx, AudioManager.ADJUST_UNMUTE)
        p.audioMutedByRelay = false
        walkBackIfOwed(ctx)
    }

    /** The phone's own volume goes back, as soon as the output it belongs to is the one in use. */
    private fun walkBackIfOwed(ctx: Context) {
        val p = Prefs(ctx)
        val index = p.audioHeldIndex
        if (index < 0 || p.audioMutedByRelay) return
        if (!volumeGoesBack(p.audioHeldOnEarphones, earphonesOn)) {
            Log.i(TAG, "AUDIO phone's own volume waits for its output to come back")
            return
        }
        val am = audio(ctx)
        stepIndex(ctx, index - am.getStreamVolume(AudioManager.STREAM_MUSIC))
        p.audioHeldIndex = -1
        Log.i(TAG, "AUDIO phone speaker restored to $index, reads ${am.getStreamVolume(AudioManager.STREAM_MUSIC)}")
    }
}
