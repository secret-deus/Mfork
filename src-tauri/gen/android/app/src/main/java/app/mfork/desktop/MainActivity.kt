package app.mfork.desktop

import android.os.Bundle
import androidx.core.view.WindowCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    // Keep the status and navigation bars: the web layout does not handle
    // display-cutout / system-bar insets, so the window must not be drawn
    // edge-to-edge (the generated template called enableEdgeToEdge()).
    WindowCompat.setDecorFitsSystemWindows(window, true)
  }
}
