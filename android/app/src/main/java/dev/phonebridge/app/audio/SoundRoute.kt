package dev.phonebridge.app.audio

/** How often [SoundRoute.onLook] should run while a share does. */
const val ROUTE_LOOK_MS = 120L

/** Looks at sound the effect's output doesn't show before deciding it isn't there: that output plays in large pieces. */
const val ROUTE_LOOKS = 3

/** Looks with nothing captured before giving up on a check. */
const val ROUTE_QUIET_LOOKS = 10

/** Android's volume panel stays up under 4 s after the last press (measured on a realme RMX3561, 2026-11-05). */
const val ROUTE_PANEL_LOOKS = 34

/** A Visualizer's peak, in millibels, at this floor is an output playing nothing at all. */
private const val MIX_SILENT_MB = -9590

fun mixIsAudible(peakMb: Int): Boolean = peakMb > MIX_SILENT_MB

/** Whether a stretch of captured frames held sound. */
fun capturedSound(audible: Int, frames: Int): Boolean = frames > 0 && audible * 2 >= frames

private val PLAYER_ID = Regex("piid:(\\d+)")

/**
 * Tells players apart from Android's descriptions of them. An app can't ask for a player's ID, but
 * the description has always carried it, and the hash changes when the stream is muted. If the
 * description ever changes, players are just counted, which only loses a change that keeps the count.
 */
fun playerIds(described: List<String>): Set<Int> =
    described.mapIndexed { i, d -> PLAYER_ID.find(d)?.groupValues?.get(1)?.toIntOrNull() ?: -1 - i }.toSet()

/** What [SoundRoute] wants done. Moves are relative steps of the phone's volume. */
data class RouteActions(
    val muted: Boolean? = null,
    val moveIndex: Int = 0,
    val level: Int? = null,
    val castBar: Boolean? = null,
    val freshMeter: Boolean = false,
    /** Put Relay's volume session back on top of Android's list, so the keys reach it. */
    val reclaimKeys: Boolean = false,
)

/**
 * The one rule for keeping the phone silent while its volume keys set the PC's level, for any app:
 * look at each new sound once, before any bar is on screen.
 *
 * The stream starts muted. When sound is captured it is unmuted at [held], near silence, for a look at
 * a meter ahead of the silencing effect. If the meter shows the sound, the effect covers it: the
 * mute isn't needed, so the phone's own volume bar is used, already at the PC's level, and the cast bar
 * is off, as Android flips between the two if both are on. If the meter shows nothing though sound
 * is captured, the sound bypasses the effect (a game on its low-latency output): the mute goes back on
 * and the keys go to the cast bar, since a raise of the real volume would unmute it. Android won't
 * give an app the keys from one that keeps them while playing, so if a game does that each press still
 * moves the level, stays near silent at [held], and makes it look again.
 *
 * Only one player is ever looked at: with two, one sound can hide the other (a video's sound on the
 * meter hides a game's), so the phone stays muted with the cast bar.
 *
 * When the covered sound ends or any new player starts, it mutes and walks down at once: what starts
 * next may be a game, and it must find the phone already silent. Android draws none of that unless the
 * volume panel is on screen.
 *
 * So while the panel is up after a press there is a second rule, as a muted stream can only show a mute
 * icon and an empty bar. With nothing playing, or sound the effect covers, the stream stays unmuted at
 * the PC's level until [ROUTE_PANEL_LOOKS] after the last press ([Mode.FOLLOWING]), and a video swapping
 * players moves nothing. A game whose sound starts in that window is heard until the looks catch it, up
 * to [ROUTE_LOOKS] of them. Sound not known to be covered is never unmuted for the panel: there the bar
 * can't be true. Pure; the caller does what it says, on one thread.
 */
class SoundRoute(
    private val held: Int,
    private val maxIndex: Int,
    startLevel: Int,
    private val canCover: Boolean,
) {
    enum class Mode { GUARDED, PROBING, COVERED, BYPASS, FOLLOWING }

    var mode = Mode.GUARDED
        private set
    var level = clampStep(startLevel)
        private set

    /** Whether the stream is meant to be muted now. */
    val muteHeld: Boolean get() = mode == Mode.GUARDED || mode == Mode.BYPASS

    private var index = held

    /** False after a change of output: Android keeps a volume per output, and it can't be read under the mute. */
    private var indexKnown = true
    private var players = emptySet<Int>()

    /** The players the current decision was made about. */
    private var decided = emptySet<Int>()

    /** Moves Relay asked for that haven't been reported yet: those reports aren't presses. */
    private var owed = 0
    private var looksLeft = 0
    private var quietLeft = 0
    private var misses = 0

    /** Looks left until the volume panel has gone, after a press. */
    private var panelLooks = 0

    /** Whether sound was captured at the last look. */
    private var soundNow = false

    /** Presses in a row that went to the real volume though the cast bar was wanted. */
    private var leaks = 0

    /** The players of media sounds now. */
    fun onPlayers(now: Set<Int>): RouteActions {
        val added = now.any { it !in players }
        players = now
        val decidedGone = decided.none { it in now }
        return when (mode) {
            Mode.COVERED -> when {
                !added && !decidedGone -> RouteActions()
                panelLooks > 0 && now.size <= 1 -> follow(added)
                else -> guard()
            }
            Mode.BYPASS -> if (decidedGone) guard() else RouteActions()
            Mode.PROBING -> if (now.isEmpty() || now.size > 1) guard() else RouteActions()
            // Another player may have put its media session above Relay's in Android's list.
            Mode.GUARDED -> RouteActions(reclaimKeys = added)
            Mode.FOLLOWING -> when {
                now.size > 1 -> guard()
                added -> follow(true)
                else -> RouteActions()
            }
        }
    }

    /** Every [ROUTE_LOOK_MS]: whether sound was captured, and whether the effect's output plays any. */
    fun onLook(captured: Boolean, mixAudible: Boolean): RouteActions {
        soundNow = captured
        if (panelLooks > 0) panelLooks--
        return when (mode) {
            Mode.GUARDED -> if (canCover && captured && players.size == 1) probe() else RouteActions()
            Mode.PROBING -> when {
                captured && mixAudible -> cover()
                captured -> if (--looksLeft <= 0) bypass() else RouteActions()
                else -> if (--quietLeft <= 0) guard() else RouteActions()
            }
            Mode.COVERED -> {
                misses = if (captured && !mixAudible) misses + 1 else 0
                if (misses >= ROUTE_LOOKS) guard() else RouteActions()
            }
            Mode.BYPASS -> RouteActions()
            Mode.FOLLOWING -> when {
                panelLooks == 0 -> guard()
                // No player: this is the tail of the last sound.
                players.size != 1 -> RouteActions()
                captured && mixAudible -> cover()
                else -> {
                    misses = if (captured) misses + 1 else 0
                    if (misses >= ROUTE_LOOKS) guard() else RouteActions()
                }
            }
        }
    }

    /** Earphones were plugged in or out, and the mute stays wanted. */
    fun onRouteChanged(): RouteActions {
        indexKnown = false
        owed = 0
        panelLooks = 0
        return guard()
    }

    /** Android reports that the phone's real volume went from [prev] to [now]. */
    fun onReport(prev: Int, now: Int): RouteActions {
        index = now
        val wasKnown = indexKnown
        indexKnown = true
        val moved = now - prev
        if (moved == 0) return RouteActions()
        val direction = Integer.signum(moved)
        if (owed != 0 && direction == Integer.signum(owed)) {
            owed = if (kotlin.math.abs(moved) >= kotlin.math.abs(owed)) 0 else owed - moved
            return RouteActions()
        }
        // A real press: the panel is on screen from now.
        panelLooks = ROUTE_PANEL_LOOKS
        if (mode == Mode.COVERED || mode == Mode.FOLLOWING) {
            if (owed != 0) {
                // A press in the middle of Relay's own walk: `now` is mid-walk, so the level is one
                // step from where the walk was going, and the walk is corrected to end there.
                level = stepAfterKey(level, direction)
                val walk = indexForStep(level, maxIndex) - (now + owed)
                owed += walk
                return RouteActions(level = level, moveIndex = walk)
            }
            if (!wasKnown) {
                // The first press on a new output: one step, and only now can the bar go to the level.
                level = stepAfterKey(level, direction)
                return RouteActions(level = level, moveIndex = walkTo(indexForStep(level, maxIndex)))
            }
            level = stepForIndex(now, maxIndex)
            return RouteActions(level = level)
        }
        // An app that keeps the keys, with sound not known to be covered.
        level = stepAfterKey(level, direction)
        if (mode == Mode.PROBING) {
            // Near silence either way, and the look is about to settle the bar.
            owed -= direction
            return RouteActions(moveIndex = -direction, level = level, castBar = false)
        }
        if (mode == Mode.GUARDED && canCover && players.size <= 1 && !soundNow) {
            // Nothing to hear: the real bar can show the true level while the panel is up.
            val walk = indexForStep(level, maxIndex) - (now + owed)
            owed += walk
            return follow(players.isNotEmpty()).copy(muted = false, moveIndex = walk, level = level, castBar = false)
        }
        mode = Mode.GUARDED
        owed -= direction
        // The press went to the real stream. Android hands keys to the top media session, so Relay's goes back
        // on top once; a second press in a row that still misses it means the app in front keeps the keys
        // itself, and the cast bar would only add a second row.
        leaks++
        return RouteActions(
            muted = true, moveIndex = -direction, level = level,
            castBar = if (leaks >= 2) false else null,
            reclaimKeys = true,
        )
    }

    /** The cast bar set the level. If the real volume is what shows, it follows. */
    fun setLevel(step: Int): RouteActions {
        leaks = 0
        level = clampStep(step)
        val shown = mode == Mode.COVERED || mode == Mode.FOLLOWING
        return if (shown) RouteActions(moveIndex = walkTo(indexForStep(level, maxIndex))) else RouteActions()
    }

    private fun walkTo(target: Int): Int {
        if (!indexKnown) return 0
        val walk = target - index
        owed += walk
        return walk
    }

    /** The real bar stays on screen, at the level. [newSound]: the meter must not show the last sound for it. */
    private fun follow(newSound: Boolean): RouteActions {
        mode = Mode.FOLLOWING
        decided = emptySet()
        misses = 0
        return RouteActions(freshMeter = newSound)
    }

    private fun guard(): RouteActions {
        mode = Mode.GUARDED
        decided = emptySet()
        misses = 0
        leaks = 0
        return RouteActions(muted = true, moveIndex = walkTo(held), castBar = true, reclaimKeys = true)
    }

    private fun probe(): RouteActions {
        mode = Mode.PROBING
        looksLeft = ROUTE_LOOKS
        quietLeft = ROUTE_QUIET_LOOKS
        return RouteActions(muted = false, freshMeter = true)
    }

    private fun cover(): RouteActions {
        mode = Mode.COVERED
        decided = players
        misses = 0
        return RouteActions(moveIndex = walkTo(indexForStep(level, maxIndex)), castBar = false)
    }

    private fun bypass(): RouteActions {
        mode = Mode.BYPASS
        decided = players
        return RouteActions(muted = true)
    }
}
