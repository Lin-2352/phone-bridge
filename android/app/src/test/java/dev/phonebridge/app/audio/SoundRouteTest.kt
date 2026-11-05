package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Test

class SoundRouteTest {
    /** With a player of media sound active. */
    private fun route(level: Int = 8, canCover: Boolean = true) =
        SoundRoute(held = 1, maxIndex = 16, startLevel = level, canCover = canCover).also { it.onPlayers(setOf(1)) }

    private val nothing = RouteActions()

    /** A sound started and the meter ahead of the effect shows it: a video, say. */
    private fun covered(level: Int = 8): SoundRoute {
        val r = route(level)
        r.onPlayers(setOf(1))
        r.onLook(captured = true, mixAudible = false)
        r.onLook(captured = true, mixAudible = true)
        var at = 1
        while (at < indexForStep(level, 16)) {
            r.onReport(at, at + 1)
            at++
        }
        return r
    }

    /** A sound started and is captured, but the output with the effect plays nothing: a game. */
    private fun bypassing(level: Int = 8): SoundRoute {
        val r = route(level)
        r.onPlayers(setOf(1))
        r.onLook(captured = true, mixAudible = false)
        repeat(ROUTE_LOOKS) { r.onLook(captured = true, mixAudible = false) }
        return r
    }

    /** The tail of a sound is still being captured after its player stopped: there is nothing to look at. */
    @Test
    fun with_no_player_there_is_nothing_to_look_at() {
        val r = SoundRoute(held = 1, maxIndex = 16, startLevel = 8, canCover = true)
        assertEquals(nothing, r.onLook(captured = true, mixAudible = false))
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    @Test
    fun a_sound_stopping_during_the_look_mutes_at_once() {
        val r = route()
        r.onLook(captured = true, mixAudible = false)
        assertEquals(SoundRoute.Mode.PROBING, r.mode)
        assertEquals(true, r.onPlayers(emptySet()).muted)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    @Test
    fun it_starts_muted_with_the_cast_bar_and_stays_so_while_nothing_plays() {
        val r = route()
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
        assertEquals(nothing, r.onLook(captured = false, mixAudible = false))
        assertEquals(nothing, r.onPlayers(setOf(1)))
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    @Test
    fun sound_starting_unmutes_at_the_held_volume_for_a_look() {
        val r = route()
        val a = r.onLook(captured = true, mixAudible = false)
        assertEquals(false, a.muted)
        assertEquals("near silence while it looks", 0, a.moveIndex)
        assertEquals(true, a.freshMeter)
        assertEquals(SoundRoute.Mode.PROBING, r.mode)
    }

    @Test
    fun sound_the_effect_covers_gets_the_real_bar_already_at_the_pc_level() {
        val r = route(level = 9)
        r.onLook(captured = true, mixAudible = false)
        val a = r.onLook(captured = true, mixAudible = true)
        assertEquals("from the held volume to the level", 8, a.moveIndex)
        assertEquals("the phone's own bar now", false, a.castBar)
        assertEquals(null, a.muted)
        assertEquals(SoundRoute.Mode.COVERED, r.mode)
    }

    @Test
    fun relays_own_walk_up_is_not_taken_for_presses() {
        val r = route(level = 9)
        r.onLook(captured = true, mixAudible = false)
        r.onLook(captured = true, mixAudible = true)
        for (i in 1 until 9) {
            assertEquals(nothing, r.onReport(i, i + 1))
        }
        assertEquals(9, r.level)
    }

    @Test
    fun in_the_real_bar_each_press_is_just_the_pc_level() {
        val r = covered()
        val up = r.onReport(8, 9)
        assertEquals(RouteActions(level = 9), up)
        assertEquals(RouteActions(level = 8), r.onReport(9, 8))
        assertEquals(RouteActions(level = 7), r.onReport(8, 7))
        assertEquals(SoundRoute.Mode.COVERED, r.mode)
    }

    @Test
    fun fast_presses_in_the_real_bar_are_all_followed() {
        val r = covered()
        var at = 8
        repeat(5) {
            r.onReport(at, at + 1)
            at++
        }
        assertEquals(13, r.level)
    }

    @Test
    fun sound_the_effect_misses_goes_back_under_the_mute_and_keeps_the_cast_bar() {
        val r = route()
        r.onLook(captured = true, mixAudible = false)
        repeat(ROUTE_LOOKS - 1) {
            assertEquals("one look is not enough", nothing, r.onLook(captured = true, mixAudible = false))
        }
        val a = r.onLook(captured = true, mixAudible = false)
        assertEquals(true, a.muted)
        assertEquals(0, a.moveIndex)
        assertEquals(SoundRoute.Mode.BYPASS, r.mode)
        assertEquals("settled", nothing, r.onLook(captured = true, mixAudible = false))
    }

    @Test
    fun a_sound_gone_before_it_could_be_told_apart_mutes_again() {
        val r = route()
        r.onLook(captured = true, mixAudible = false)
        repeat(ROUTE_QUIET_LOOKS - 1) {
            assertEquals(nothing, r.onLook(captured = false, mixAudible = false))
        }
        assertEquals(true, r.onLook(captured = false, mixAudible = false).muted)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    @Test
    fun without_the_effect_and_its_meter_the_mute_never_comes_off() {
        val r = route(canCover = false)
        assertEquals(nothing, r.onLook(captured = true, mixAudible = true))
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    /** From a video to a game: the video stops first, and from then on the phone is safe. */
    @Test
    fun the_covered_sound_ending_mutes_and_walks_down_before_anything_new_can_start() {
        val r = covered()
        val a = r.onPlayers(emptySet())
        assertEquals(true, a.muted)
        assertEquals(1 - 8, a.moveIndex)
        assertEquals("the keys must reach the PC's level", true, a.castBar)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
        for (i in 8 downTo 2) {
            assertEquals("its own walk", nothing, r.onReport(i, i - 1))
        }
        assertEquals(8, r.level)
    }

    @Test
    fun any_new_player_while_covered_mutes_at_once_as_it_may_be_a_game() {
        val r = covered()
        val a = r.onPlayers(setOf(1, 2))
        assertEquals(true, a.muted)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    /** Measured at 13:28: a video was covered, a game started, and the meter still showed the video's sound. */
    @Test
    fun two_players_are_never_looked_at_as_one_sound_may_hide_the_other() {
        val r = route()
        r.onPlayers(setOf(1, 2))
        assertEquals(nothing, r.onLook(captured = true, mixAudible = false))
        assertEquals(nothing, r.onLook(captured = true, mixAudible = true))
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    @Test
    fun a_second_player_joining_a_covered_sound_mutes_and_stays_muted() {
        val r = covered()
        assertEquals(true, r.onPlayers(setOf(1, 2)).muted)
        assertEquals(nothing, r.onLook(captured = true, mixAudible = true))
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    @Test
    fun the_second_player_leaving_lets_it_look_again() {
        val r = covered()
        r.onPlayers(setOf(1, 2))
        r.onPlayers(setOf(2))
        assertEquals(false, r.onLook(captured = true, mixAudible = false).muted)
        assertEquals(SoundRoute.Mode.PROBING, r.mode)
    }

    @Test
    fun a_second_player_joining_during_the_look_mutes_at_once() {
        val r = route()
        r.onLook(captured = true, mixAudible = false)
        assertEquals(true, r.onPlayers(setOf(1, 2)).muted)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    @Test
    fun a_games_extra_sounds_dont_make_it_look_again() {
        val r = bypassing()
        assertEquals(nothing, r.onPlayers(setOf(1, 2)))
        assertEquals(nothing, r.onPlayers(setOf(1)))
        assertEquals(SoundRoute.Mode.BYPASS, r.mode)
    }

    /** From a game to a video: the two flickers came from deciding all this while the bar was up. */
    @Test
    fun the_game_ending_makes_it_look_again_at_the_next_sound() {
        val r = bypassing()
        val gone = r.onPlayers(emptySet())
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
        assertEquals("nothing moves", 0, gone.moveIndex)
        r.onPlayers(setOf(2))
        assertEquals(false, r.onLook(captured = true, mixAudible = false).muted)
        val next = r.onLook(captured = true, mixAudible = true)
        assertEquals(false, next.castBar)
        assertEquals(SoundRoute.Mode.COVERED, r.mode)
    }

    @Test
    fun something_starting_to_bypass_while_covered_goes_back_under_the_mute() {
        val r = covered()
        repeat(ROUTE_LOOKS - 1) {
            assertEquals(nothing, r.onLook(captured = true, mixAudible = false))
        }
        assertEquals(true, r.onLook(captured = true, mixAudible = false).muted)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    @Test
    fun a_quiet_moment_while_covered_is_not_a_bypass() {
        val r = covered()
        repeat(ROUTE_LOOKS * 3) {
            r.onLook(captured = true, mixAudible = false)
            r.onLook(captured = true, mixAudible = true)
        }
        assertEquals(SoundRoute.Mode.COVERED, r.mode)
    }

    /** Measured: held down, reports overtake the undos, and the extra undo read as a press down. */
    @Test
    fun fast_presses_in_a_game_that_keeps_the_keys_count_exactly_once_each() {
        val r = bypassing()
        for ((prev, index) in listOf(1 to 2, 2 to 3, 3 to 4)) {
            val a = r.onReport(prev, index)
            assertEquals(-1, a.moveIndex)
            assertEquals(true, a.muted)
        }
        assertEquals(11, r.level)
        for ((prev, index) in listOf(4 to 3, 3 to 2, 2 to 1)) {
            assertEquals("its own undo", nothing, r.onReport(prev, index))
        }
        assertEquals(11, r.level)
    }

    @Test
    fun a_press_down_in_a_game_that_keeps_the_keys_counts_once_and_comes_back() {
        val r = bypassing()
        val a = r.onReport(1, 0)
        assertEquals(7, a.level)
        assertEquals(1, a.moveIndex)
        assertEquals(true, a.muted)
        assertEquals(nothing, r.onReport(0, 1))
        assertEquals(7, r.level)
    }

    /** A video that starts very quietly can look like a bypass; a press there says otherwise. */
    @Test
    fun a_press_on_the_real_volume_after_deciding_bypass_makes_it_look_again() {
        val r = bypassing()
        r.onReport(1, 2)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
        r.onReport(2, 1)
        assertEquals(false, r.onLook(captured = true, mixAudible = false).muted)
    }

    @Test
    fun the_cast_bar_moves_the_level_and_the_real_volume_follows_if_it_shows() {
        val muted = route()
        assertEquals(nothing, muted.setLevel(12))
        assertEquals(12, muted.level)
        val shown = covered()
        val a = shown.setLevel(10)
        assertEquals(2, a.moveIndex)
        assertEquals("its own walk", nothing, shown.onReport(8, 9))
        assertEquals(nothing, shown.onReport(9, 10))
        assertEquals(10, shown.level)
    }

    /** Earphones plugged in or out. Android keeps a volume per output, so the old reading means nothing now. */
    @Test
    fun an_output_change_mutes_at_once_and_forgets_the_volume() {
        val r = covered()
        val a = r.onRouteChanged()
        assertEquals(true, a.muted)
        assertEquals("nothing is known to walk from", 0, a.moveIndex)
        assertEquals(true, a.castBar)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    @Test
    fun after_an_output_change_the_volume_isnt_walked_blind_and_the_level_doesnt_jump() {
        val r = covered()
        r.onRouteChanged()
        r.onLook(captured = true, mixAudible = false)
        val again = r.onLook(captured = true, mixAudible = true)
        assertEquals(0, again.moveIndex)
        assertEquals(SoundRoute.Mode.COVERED, r.mode)
        // The earphones were at 14. The first press tells that, and the level moves by one step only.
        val a = r.onReport(13, 14)
        assertEquals(9, a.level)
        assertEquals("and the bar goes to the level", 9 - 14, a.moveIndex)
        assertEquals("its own walk", RouteActions(), r.onReport(14, 13))
    }

    /**
     * Reproduced 01:50: a game in front and a song playing in YouTube. Android handed every key to YouTube's own
     * session, which sat above Relay's in its priority list, so the bar could not show the PC level.
     * A session goes back on top when it is set to play again, so each time the cast bar is wanted that is done.
     */
    @Test
    fun guarding_takes_the_keys_back_from_other_media_sessions() {
        assertEquals(true, covered().onPlayers(emptySet()).reclaimKeys)
        assertEquals(true, covered().onPlayers(setOf(1, 2)).reclaimKeys)
        assertEquals(true, covered().onRouteChanged().reclaimKeys)
    }

    @Test
    fun a_new_player_while_guarded_takes_the_keys_back_again() {
        val r = route()
        assertEquals(true, r.onPlayers(setOf(1, 2)).reclaimKeys)
        assertEquals("nothing new", false, r.onPlayers(setOf(1, 2)).reclaimKeys)
        assertEquals("a player leaving changes no order", false, r.onPlayers(setOf(1)).reclaimKeys)
        assertEquals(true, r.onPlayers(setOf(1, 3)).reclaimKeys)
    }

    @Test
    fun a_covered_sound_needs_no_reclaim_when_a_player_joins_the_same_way_it_mutes() {
        assertEquals(false, covered().onLook(captured = true, mixAudible = true).reclaimKeys)
    }

    /** Measured 16:00: YouTube in front sent the keys to the real stream whatever Relay did; both bars then showed. */
    @Test
    fun a_first_press_that_leaks_reclaims_the_keys_and_a_second_in_a_row_gives_up_the_cast_bar() {
        val r = bypassing()
        val first = r.onReport(1, 2)
        assertEquals(true, first.reclaimKeys)
        assertEquals("one more try before the cast bar goes", null, first.castBar)
        val second = r.onReport(1, 2)
        assertEquals(false, second.castBar)
    }

    @Test
    fun a_press_that_reached_the_cast_bar_starts_the_count_again() {
        val r = bypassing()
        r.onReport(1, 2)
        r.setLevel(9)
        assertEquals(null, r.onReport(1, 2).castBar)
    }

    /** A press in the middle of Relay's own walk up used to drop the PC to the middle of the walk. */
    @Test
    fun a_press_down_during_the_walk_up_moves_the_level_one_step_from_where_it_was_going() {
        val r = route(level = 13)
        r.onLook(captured = true, mixAudible = false)
        r.onLook(captured = true, mixAudible = true)
        for (i in 1..5) assertEquals(nothing, r.onReport(i, i + 1))
        val a = r.onReport(6, 5)
        assertEquals(12, a.level)
        assertEquals("the walk already ends where the level says", 0, a.moveIndex)
        assertEquals(12, r.level)
    }

    /** With nothing playing, and no panel on screen. */
    private fun idle(level: Int = 8, canCover: Boolean = true) =
        SoundRoute(held = 1, maxIndex = 16, startLevel = level, canCover = canCover)

    /** A press from an app that keeps the keys, with nothing playing: the real bar is on screen, showing the level. */
    private fun following(level: Int = 8): SoundRoute {
        val r = idle(level)
        val a = r.onReport(1, 2)
        var at = 2
        repeat(kotlin.math.abs(a.moveIndex)) {
            val next = if (a.moveIndex > 0) at + 1 else at - 1
            r.onReport(at, next)
            at = next
        }
        return r
    }

    /** Reproduced 22:57: YouTube paused, the bar showed a mute icon and stayed empty while the PC went 1 to 4 to 1. */
    @Test
    fun a_press_with_nothing_playing_shows_the_true_level_on_the_real_bar() {
        val r = idle()
        val a = r.onReport(1, 2)
        assertEquals(9, a.level)
        assertEquals("the mute comes off", false, a.muted)
        assertEquals("the bar goes to the level", 9 - 2, a.moveIndex)
        assertEquals(false, a.castBar)
        assertEquals(SoundRoute.Mode.FOLLOWING, r.mode)
    }

    @Test
    fun a_press_down_with_nothing_playing_does_the_same() {
        val r = idle()
        val a = r.onReport(1, 0)
        assertEquals(7, a.level)
        assertEquals(false, a.muted)
        assertEquals(7, a.moveIndex)
    }

    @Test
    fun relays_walk_to_the_level_is_not_taken_for_presses() {
        val r = idle()
        r.onReport(1, 2)
        for (i in 2 until 9) assertEquals("its own walk", nothing, r.onReport(i, i + 1))
        assertEquals(9, r.level)
    }

    @Test
    fun with_the_real_bar_showing_each_later_press_is_just_the_level() {
        val r = following()
        assertEquals(RouteActions(level = 10), r.onReport(9, 10))
        assertEquals(RouteActions(level = 9), r.onReport(10, 9))
        assertEquals(SoundRoute.Mode.FOLLOWING, r.mode)
    }

    @Test
    fun the_mute_comes_back_only_once_the_panel_has_gone() {
        val r = following()
        repeat(ROUTE_PANEL_LOOKS - 1) {
            assertEquals("the panel is still up", nothing, r.onLook(captured = false, mixAudible = false))
        }
        val a = r.onLook(captured = false, mixAudible = false)
        assertEquals(true, a.muted)
        assertEquals(1 - 9, a.moveIndex)
        assertEquals("the keys go to the cast bar again", true, a.castBar)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
        for (i in 9 downTo 2) assertEquals("its own walk", nothing, r.onReport(i, i - 1))
        assertEquals(9, r.level)
    }

    @Test
    fun each_press_keeps_the_panel_window_open() {
        val r = following()
        repeat(ROUTE_PANEL_LOOKS - 2) { r.onLook(captured = false, mixAudible = false) }
        r.onReport(9, 10)
        repeat(ROUTE_PANEL_LOOKS - 1) {
            assertEquals(nothing, r.onLook(captured = false, mixAudible = false))
        }
        assertEquals(SoundRoute.Mode.FOLLOWING, r.mode)
    }

    /** Reproduced 23:01: swiping Shorts with the panel up made it vanish and flip between the speaker and cast rows. */
    @Test
    fun a_player_swap_with_the_panel_up_moves_nothing() {
        val r = covered()
        r.onReport(8, 9)
        val gone = r.onPlayers(emptySet())
        assertEquals("no mute", null, gone.muted)
        assertEquals("no walk", 0, gone.moveIndex)
        assertEquals("no change of bar", null, gone.castBar)
        assertEquals(SoundRoute.Mode.FOLLOWING, r.mode)
        val next = r.onPlayers(setOf(2))
        assertEquals(null, next.muted)
        assertEquals(0, next.moveIndex)
        assertEquals("the last sound must not count for this one", true, next.freshMeter)
        val look = r.onLook(captured = true, mixAudible = true)
        assertEquals(null, look.muted)
        assertEquals(0, look.moveIndex)
        assertEquals(SoundRoute.Mode.COVERED, r.mode)
        assertEquals(9, r.level)
    }

    @Test
    fun a_player_swap_after_the_panel_has_gone_still_mutes_at_once() {
        val r = covered()
        r.onReport(8, 9)
        repeat(ROUTE_PANEL_LOOKS) { r.onLook(captured = true, mixAudible = true) }
        assertEquals(true, r.onPlayers(emptySet()).muted)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    @Test
    fun a_new_sound_the_effect_misses_while_following_goes_under_the_mute() {
        val r = following()
        r.onPlayers(setOf(1))
        repeat(ROUTE_LOOKS - 1) {
            assertEquals(nothing, r.onLook(captured = true, mixAudible = false))
        }
        assertEquals(true, r.onLook(captured = true, mixAudible = false).muted)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    @Test
    fun the_tail_of_a_sound_with_no_player_doesnt_count_while_following() {
        val r = following()
        repeat(ROUTE_LOOKS * 2) {
            assertEquals(nothing, r.onLook(captured = true, mixAudible = false))
        }
        assertEquals(SoundRoute.Mode.FOLLOWING, r.mode)
    }

    @Test
    fun a_second_player_while_following_mutes_at_once() {
        val r = following()
        r.onPlayers(setOf(1))
        assertEquals(true, r.onPlayers(setOf(1, 2)).muted)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    /** Reproduced 23:02: music and Clash Royale. Clash skips the effect, so unmuting would be heard: the bar can't be true here. */
    @Test
    fun a_press_with_two_players_stays_under_the_mute() {
        val r = route()
        r.onPlayers(setOf(1, 2))
        val a = r.onReport(1, 2)
        assertEquals(true, a.muted)
        assertEquals(-1, a.moveIndex)
        assertEquals(9, a.level)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    @Test
    fun a_press_with_uncovered_sound_playing_stays_under_the_mute() {
        val r = route()
        r.onPlayers(setOf(1, 2))
        r.onLook(captured = true, mixAudible = false)
        r.onPlayers(setOf(1))
        // Sound was captured a moment ago and nothing has shown the effect covers it.
        assertEquals(true, r.onReport(1, 2).muted)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    @Test
    fun without_the_effect_a_press_never_takes_the_mute_off() {
        val r = idle(canCover = false)
        assertEquals(true, r.onReport(1, 2).muted)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    /** Reproduced 23:01: a key landing during the look set off mute, unmute and raise in turn. */
    @Test
    fun a_press_during_the_look_moves_the_level_and_leaves_the_mute_alone() {
        val r = route()
        r.onLook(captured = true, mixAudible = false)
        val a = r.onReport(1, 2)
        assertEquals(9, a.level)
        assertEquals(null, a.muted)
        assertEquals("back to near silence", -1, a.moveIndex)
        assertEquals(SoundRoute.Mode.PROBING, r.mode)
        assertEquals("its own undo", nothing, r.onReport(2, 1))
        val done = r.onLook(captured = true, mixAudible = true)
        assertEquals("the bar goes to the new level", 9 - 1, done.moveIndex)
        assertEquals(SoundRoute.Mode.COVERED, r.mode)
    }

    @Test
    fun the_cast_bar_moves_the_real_volume_while_following() {
        val r = following()
        assertEquals(3, r.setLevel(12).moveIndex)
        assertEquals(12, r.level)
    }

    @Test
    fun an_output_change_while_following_mutes_at_once() {
        val r = following()
        assertEquals(true, r.onRouteChanged().muted)
        assertEquals(SoundRoute.Mode.GUARDED, r.mode)
    }

    /** The watchdog that puts a lost mute back would otherwise fight the real bar. */
    @Test
    fun the_mute_is_only_held_where_the_rule_wants_it() {
        assertEquals(true, idle().muteHeld)
        assertEquals(true, bypassing().muteHeld)
        assertEquals(false, covered().muteHeld)
        assertEquals(false, following().muteHeld)
        val looking = route()
        looking.onLook(captured = true, mixAudible = false)
        assertEquals(false, looking.muteHeld)
    }

    @Test
    fun the_level_stays_within_its_range() {
        assertEquals(PC_LEVEL_STEPS, bypassing(level = PC_LEVEL_STEPS).onReport(1, 2).level)
        assertEquals(0, bypassing(level = 0).onReport(1, 0).level)
    }

    @Test
    fun the_meter_counts_any_signal_at_all_as_sound_on_the_mix() {
        org.junit.Assert.assertFalse(mixIsAudible(-9600))
        org.junit.Assert.assertTrue("a quiet video at the held volume", mixIsAudible(-9000))
        org.junit.Assert.assertTrue(mixIsAudible(-2800))
    }

    @Test
    fun half_the_frames_having_sound_counts_as_sound_captured() {
        org.junit.Assert.assertTrue(capturedSound(audible = 13, frames = 25))
        org.junit.Assert.assertFalse(capturedSound(audible = 5, frames = 25))
        org.junit.Assert.assertFalse(capturedSound(audible = 0, frames = 0))
    }

    @Test
    fun players_are_told_apart_by_the_id_in_their_description() {
        assertEquals(
            setOf(12, 345),
            playerIds(listOf("AudioPlaybackConfiguration piid:12 deviceId:2 type:android.media.AudioTrack", "AudioPlaybackConfiguration piid:345 type:unknown")),
        )
    }

    /** Muting changes a player's hash on newer Android: its ID stays, so a mute isn't a new player. */
    @Test
    fun the_same_player_muted_is_the_same_player() {
        val before = playerIds(listOf("AudioPlaybackConfiguration piid:12 muted:false"))
        val after = playerIds(listOf("AudioPlaybackConfiguration piid:12 muted:true"))
        assertEquals(before, after)
    }

    @Test
    fun without_an_id_players_are_at_least_counted() {
        assertEquals(1, playerIds(listOf("something else")).size)
        assertEquals(2, playerIds(listOf("something else", "another")).size)
        assertEquals(playerIds(listOf("a")), playerIds(listOf("b")))
    }

    @Test
    fun a_report_that_moved_nothing_is_nothing() {
        assertEquals(nothing, covered().onReport(8, 8))
    }
}
