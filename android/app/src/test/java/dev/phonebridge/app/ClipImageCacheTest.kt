package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class ClipImageCacheTest {
    private val day = 24 * 60 * 60 * 1000L
    private val now = 10 * day

    private fun clip(name: String, ageMs: Long) = CachedClip(name, now - ageMs)

    @Test
    fun the_newest_five_recent_pictures_stay() {
        val files = (1..5).map { clip("c$it", it * 1000L) }
        assertEquals(emptyList<String>(), clipsToDelete(files, now))
    }

    @Test
    fun older_pictures_beyond_five_go_oldest_first_kept_are_the_newest() {
        val files = (1..8).map { clip("c$it", it * 1000L) }
        assertEquals(setOf("c6", "c7", "c8"), clipsToDelete(files, now).toSet())
    }

    @Test
    fun a_picture_older_than_a_day_goes_even_when_it_is_the_only_one() {
        assertEquals(listOf("old"), clipsToDelete(listOf(clip("old", day + 1)), now))
    }

    @Test
    fun the_order_the_files_arrive_in_does_not_matter() {
        val files = listOf(clip("a", 3000), clip("b", 1000), clip("c", 2000))
        assertEquals(emptyList<String>(), clipsToDelete(files, now, keep = 3))
        assertEquals(listOf("a"), clipsToDelete(files, now, keep = 2))
    }

    @Test
    fun nothing_to_prune_gives_an_empty_list() {
        assertEquals(emptyList<String>(), clipsToDelete(emptyList(), now))
    }
}
