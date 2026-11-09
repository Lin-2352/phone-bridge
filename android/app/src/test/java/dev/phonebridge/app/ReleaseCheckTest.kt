package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ReleaseCheckTest {
    private fun rel(tag: String, pre: Boolean = false, draft: Boolean = false, url: String = "${RELEASE_URL_PREFIX}tag/$tag") =
        """{"tag_name":"$tag","prerelease":$pre,"draft":$draft,"html_url":"$url"}"""

    private fun list(vararg items: String) = "[${items.joinToString(",")}]".toByteArray()

    @Test
    fun offers_a_newer_release() {
        val r = newestRelease(list(rel("v0.4.0"), rel("v0.2.0", true)), "0.3.0")
        assertEquals(CheckResult.Found("v0.4.0", "${RELEASE_URL_PREFIX}tag/v0.4.0"), r)
    }

    @Test
    fun never_offers_the_installed_version_or_older() {
        assertEquals(CheckResult.UpToDate, newestRelease(list(rel("v0.3.0"), rel("v0.2.0", true)), "0.3.0"))
    }

    @Test
    fun the_highest_wins_whatever_the_order() {
        val r = newestRelease(list(rel("v0.3.5"), rel("v0.10.0"), rel("v0.4.0")), "0.3.0") as CheckResult.Found
        assertEquals("v0.10.0", r.tag)
    }

    @Test
    fun drafts_and_odd_tags_and_other_sites_are_ignored() {
        val body = list(rel("v0.9.0", draft = true), rel("v0.8.0-rc1", true), rel("nightly", true), rel("v0.7.0", url = "https://evil.example/x"))
        assertEquals(CheckResult.UpToDate, newestRelease(body, "0.3.0"))
    }

    @Test
    fun garbage_is_a_failure_not_up_to_date() {
        assertTrue(newestRelease("<html>rate limited</html>".toByteArray(), "0.3.0") is CheckResult.Failed)
        assertTrue(newestRelease("""{"message":"API rate limit exceeded"}""".toByteArray(), "0.3.0") is CheckResult.Failed)
        assertTrue(newestRelease(ByteArray(0), "0.3.0") is CheckResult.Failed)
    }

    @Test
    fun an_oversized_answer_is_refused() {
        assertTrue(newestRelease(ByteArray(RELEASE_MAX_RESPONSE + 1) { ' '.code.toByte() }, "0.3.0") is CheckResult.Failed)
    }

    @Test
    fun an_unreadable_installed_version_is_a_failure() {
        assertTrue(newestRelease("[]".toByteArray(), "") is CheckResult.Failed)
    }

    @Test
    fun an_empty_list_is_up_to_date() {
        assertEquals(CheckResult.UpToDate, newestRelease("[]".toByteArray(), "0.3.0"))
    }

    @Test
    fun the_request_uses_the_vpn_when_one_is_up_never_the_bypassed_wifi() {
        val wifi = UpdateNet(1, vpn = false, wifi = true, ethernet = false, cellular = false)
        val vpn = UpdateNet(2, vpn = true, wifi = false, ethernet = false, cellular = false)
        assertEquals(2L, updateNetworkToUse(listOf(wifi, vpn)))
        assertEquals(1L, updateNetworkToUse(listOf(wifi)))
        assertNull(updateNetworkToUse(emptyList()))
    }

    @Test
    fun the_note_never_says_verified_or_safe() {
        val texts = listOf(null, CheckResult.UpToDate, CheckResult.Found("v0.4.0", "u"), CheckResult.Failed("x")).map { updateNote(it, "0.3.0").lowercase() }
        assertTrue(texts.none { it.contains("verified") || it.contains("safe") })
        assertTrue(updateNote(CheckResult.Failed("no connection"), "0.3.0").startsWith("Couldn't check"))
    }
}
