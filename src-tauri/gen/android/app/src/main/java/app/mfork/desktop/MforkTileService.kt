package app.mfork.desktop

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService

/**
 * Quick-settings tile. Active while the VPN is up: tapping stops it (the
 * page is told through the service's onStopped callback). Inactive:
 * tapping opens the app, which auto-connects on launch.
 */
class MforkTileService : TileService() {
  companion object {
    fun requestUpdate(context: Context) {
      try {
        requestListeningState(context, ComponentName(context, MforkTileService::class.java))
      } catch (_: Exception) {
      }
    }
  }

  override fun onStartListening() {
    super.onStartListening()
    refresh()
  }

  override fun onClick() {
    super.onClick()
    if (MforkVpnService.running) {
      startService(
        Intent(this, MforkVpnService::class.java)
          .setAction(MforkVpnService.ACTION_STOP)
          .putExtra(MforkVpnService.EXTRA_FROM_USER, true)
      )
    } else {
      val open = Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
        startActivityAndCollapse(
          android.app.PendingIntent.getActivity(
            this, 0, open,
            android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT
          )
        )
      } else {
        @Suppress("DEPRECATION")
        startActivityAndCollapse(open)
      }
    }
    refresh()
  }

  private fun refresh() {
    val tile = qsTile ?: return
    tile.state = if (MforkVpnService.running) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
    tile.label = "Mfork"
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      tile.subtitle = if (MforkVpnService.running) "VPN 已连接" else "未连接"
    }
    tile.updateTile()
  }
}
