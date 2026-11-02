package dev.phonebridge.app

import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.ui.Modifier
import dev.phonebridge.app.ui.RelayTheme
import dev.phonebridge.app.ui.ThemeChoice
import dev.phonebridge.app.ui.remote.TouchpadPad

/**
 * Thin host for [TouchpadPad], so existing shortcuts and tiles still open the touchpad full-screen.
 * The pad holds the input session while this activity is started.
 */
class TouchpadActivity : ComponentActivity() {
    @OptIn(ExperimentalMaterial3Api::class)
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        ThemeChoice.mode.intValue = Prefs(this).theme
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        setContent {
            RelayTheme {
                Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surface) {
                    Scaffold(
                        topBar = {
                            TopAppBar(
                                title = { Text("Touchpad") },
                                navigationIcon = { IconButton(onClick = { finish() }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") } },
                            )
                        },
                    ) { pad ->
                        TouchpadPad(Modifier.padding(pad).consumeWindowInsets(pad).fillMaxSize().navigationBarsPadding())
                    }
                }
            }
        }
    }
}
