package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ShareDraftTest {
    @Test fun linkIsFoundInsideShortText() =
        assertEquals(SharedTextKind.Link("https://a.b/c"), classifySharedText("Look at this https://a.b/c"))

    @Test fun linkSchemeIsMatchedCaseInsensitivelyAndLowercased() =
        assertEquals(SharedTextKind.Link("https://a.b/C"), classifySharedText("  HTTPS://a.b/C \n"))

    @Test fun longTextWithALinkStaysText() {
        val t = "word ".repeat(40) + "https://a.b"
        assertEquals(SharedTextKind.Text(t), classifySharedText(t))
    }

    @Test fun plainTextIsText() = assertEquals(SharedTextKind.Text("hello"), classifySharedText("hello"))

    @Test fun textSharedWithFilesIsKept() {
        assertEquals(SharedTextKind.Text("caption"), textToSend("caption"))
        assertNull(textToSend(null))
    }

    @Test fun ownTreeMatchesPercentEncodingAndUserPrefix() {
        val tree = "content://com.android.externalstorage.documents/tree/primary%3ADownload%2FRelay"
        assertTrue(isUnderTree("content://com.android.externalstorage.documents/tree/primary%3aDownload%2fRelay/document/primary%3ADownload%2FRelay%2Fa.jpg", tree))
        assertTrue(isUnderTree("content://0@com.android.externalstorage.documents/document/primary%3ADownload%2FRelay%2Fa.jpg", tree))
        assertTrue(isUnderTree("content://com.android.externalstorage.documents/document/primary%3ADownload%2FRelay", tree))
        assertFalse(isUnderTree("content://com.android.externalstorage.documents/document/primary%3ADownload%2FRelayOther%2Fa.jpg", tree))
        assertFalse(isUnderTree("content://other.authority/document/primary%3ADownload%2FRelay%2Fa.jpg", tree))
        assertFalse(isUnderTree("content://media/external/images/media/12", tree))
    }

    @Test fun previewListsFilesAndText() {
        assertEquals("hello", sharePreview("hello", emptyList()))
        assertEquals("2 files: a.jpg, b.pdf", sharePreview(null, listOf("a.jpg", "b.pdf")))
        assertEquals("5 files: a, b, c and 2 more\nnote", sharePreview("note", listOf("a", "b", "c", "d", "e")))
        assertEquals("x".repeat(200) + "…", sharePreview("x".repeat(250), emptyList()))
    }
}
