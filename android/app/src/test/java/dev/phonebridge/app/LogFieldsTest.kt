package dev.phonebridge.app

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class LogFieldsTest {
    @get:Rule val tmp = TemporaryFolder()

    @Test fun json_builds_a_flat_object_and_skips_nulls() {
        assertEquals("""{"a":1,"b":true,"c":"x","e":2.5}""", LogFields.json("a" to 1, "b" to true, "c" to "x", "d" to null, "e" to 2.5))
        assertEquals("{}", LogFields.json())
    }

    @Test fun json_escapes_quotes_backslashes_and_control_characters() {
        val bs = "\\"
        assertEquals("{\"k\":\"a${bs}\"b${bs}${bs}c${bs}nd${bs}u0001\"}", LogFields.json("k" to "a\"b${bs}c\nd\u0001"))
        assertEquals("{\"we${bs}\"ird\":\"v\"}", LogFields.json("we\"ird" to "v"))
    }

    @Test fun json_never_emits_non_finite_numbers() {
        assertEquals("""{"x":null}""", LogFields.json("x" to Double.NaN))
    }

    @Test fun bssid_is_hashed_to_8_hex_and_case_insensitive() {
        val h = LogFields.hashBssid("AA:BB:CC:DD:EE:FF")!!
        assertEquals(8, h.length)
        assertEquals(h, LogFields.hashBssid("aa:bb:cc:dd:ee:ff"))
        // First 8 hex of SHA-256("aa:bb:cc:dd:ee:ff"), checked with sha256sum.
        assertEquals("c1582e87", h)
    }

    @Test fun redacted_or_missing_bssid_is_omitted() {
        assertNull(LogFields.hashBssid(null))
        assertNull(LogFields.hashBssid(""))
        assertNull(LogFields.hashBssid("02:00:00:00:00:00"))
    }

    @Test fun bands_by_frequency() {
        assertEquals("2.4 GHz", LogFields.band(2437))
        assertEquals("5 GHz", LogFields.band(5180))
        assertEquals("6 GHz", LogFields.band(6055))
        assertNull(LogFields.band(0))
    }

    @Test fun only_an_extension_is_kept_from_a_file_name() {
        assertEquals(".pdf", LogFields.extOnly("Tax return 2025.PDF"))
        assertEquals("", LogFields.extOnly("README"))
        assertEquals("", LogFields.extOnly("a.verylongext"))
        assertEquals("", LogFields.extOnly("weird.ex t"))
    }

    @Test fun export_takes_only_day_folders_in_date_order() {
        val root = tmp.newFolder("logs")
        for (d in listOf("2026-11-03", "2026-11-01", "notes", "2026-1-1")) File(root, d).mkdirs()
        File(root, "2026-11-03/events.jsonl").writeText("x")
        File(root, "2026-11-01/events.jsonl.gz").writeText("x")
        File(root, "2026-11-01/crash.txt").writeText("x")
        File(root, "notes/events.jsonl").writeText("x")
        File(root, "stray.txt").writeText("x")
        File(root, "2026-11-03/sub").mkdirs()
        assertEquals(
            listOf("2026-11-01/crash.txt", "2026-11-01/events.jsonl.gz", "2026-11-03/events.jsonl"),
            LogFields.exportFiles(root).map { it.first },
        )
    }

    @Test fun mb_has_one_decimal() {
        assertEquals("1.5 MB", LogFields.mb(1_572_864))
        assertEquals("0.0 MB", LogFields.mb(0))
    }
}
