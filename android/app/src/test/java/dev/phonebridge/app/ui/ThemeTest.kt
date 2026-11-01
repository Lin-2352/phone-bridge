package dev.phonebridge.app.ui

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.phonebridge.app.APP_NAME
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class ThemeTest {
    @Test fun primary_is_cobalt() {
        assertEquals(Color(0xFF3651C5), relayLightScheme().primary)
    }

    @Test fun dark_scheme_uses_dark_tokens() {
        assertEquals(Tokens.Dark.primary, relayDarkScheme().primary)
        assertEquals(Tokens.Dark.text, relayDarkScheme().onSurface)
    }

    @Test fun body_text_uses_token_size() {
        assertEquals(14.sp, RelayTypography.bodyMedium.fontSize)
        assertEquals(Tokens.Type.Title, RelayTypography.titleLarge.fontSize)
    }

    @Test fun shapes_use_token_radii() {
        assertEquals(Tokens.Radius[0], 4.dp)
        assertEquals(Tokens.Radius[3], 20.dp)
    }

    @Test fun no_user_visible_string_says_phone_bridge() {
        assertFalse("APP_NAME", APP_NAME.contains("Phone Bridge", ignoreCase = true))
        val strings = listOf("src/main/res/values/strings.xml", "app/src/main/res/values/strings.xml")
            .map(::File).first { it.exists() }
        // Every <string> value is user-visible. None may say "Phone Bridge" (the product is Relay).
        // Source comments and the legacy Downloads folder name are not in strings.xml, so no allow-list is needed.
        val values = Regex("<string[^>]*>(.*?)</string>", RegexOption.DOT_MATCHES_ALL)
            .findAll(strings.readText()).map { it.groupValues[1] }.toList()
        assertFalse(values.isEmpty())
        for (v in values) assertFalse("user string says Phone Bridge: $v", v.contains("Phone Bridge", ignoreCase = true))
    }
}
