package app.mfork.desktop

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.VpnService
import android.os.Build
import android.os.ParcelFileDescriptor
import android.util.Log

/**
 * Owns the TUN interface. The core itself is a child process of the app
 * spawned by the Rust side: this service only establishes the VPN, hands
 * the raw fd back through [onEstablished], and keeps a foreground
 * notification alive while the VPN is up (docs/ANDROID.md, decision 2).
 *
 * The app excludes itself from the VPN (addDisallowedApplication), so
 * every socket the core opens bypasses the tunnel without protect().
 */
class MforkVpnService : VpnService() {
  companion object {
    const val ACTION_START = "app.mfork.desktop.vpn.START"
    const val ACTION_STOP = "app.mfork.desktop.vpn.STOP"
    const val EXTRA_MTU = "mtu"
    const val EXTRA_IPV6 = "ipv6"
    const val EXTRA_SESSION = "session"
    private const val TAG = "MforkVpn"
    private const val CHANNEL = "mfork-vpn"
    private const val NOTIFICATION_ID = 1

    /** Set by the plugin before starting the service; called once with (fd, null) or (null, error). */
    @Volatile var onEstablished: ((Int?, String?) -> Unit)? = null
    /** The system (or another VPN app) took the VPN away. */
    @Volatile var onRevoked: (() -> Unit)? = null
    @Volatile var running = false
      private set
  }

  private var tun: ParcelFileDescriptor? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    if (intent?.action == ACTION_STOP) {
      teardown()
      return START_NOT_STICKY
    }
    val mtu = intent?.getIntExtra(EXTRA_MTU, 1500) ?: 1500
    val ipv6 = intent?.getBooleanExtra(EXTRA_IPV6, false) ?: false
    val session = intent?.getStringExtra(EXTRA_SESSION) ?: "Mfork"

    startForegroundCompat(session)
    try {
      val builder = Builder()
        .setSession(session)
        .setMtu(mtu)
        .addAddress("172.19.0.1", 30)
        .addRoute("0.0.0.0", 0)
        // any address inside the tunnel: the core's dns-hijack picks the query up
        .addDnsServer("198.18.0.2")
        .setBlocking(false)
      if (ipv6) {
        builder.addAddress("fdfe:dcba:9876::1", 126)
        builder.addRoute("::", 0)
      }
      try {
        builder.addDisallowedApplication(packageName)
      } catch (e: Exception) {
        Log.w(TAG, "addDisallowedApplication: $e")
      }
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) builder.setMetered(false)

      val pfd = builder.establish()
        ?: throw IllegalStateException("establish() returned null — VPN permission missing or revoked")
      tun?.close()
      tun = pfd
      running = true
      Log.i(TAG, "tun established fd=${pfd.fd} mtu=$mtu ipv6=$ipv6")
      val cb = onEstablished
      onEstablished = null
      cb?.invoke(pfd.fd, null)
      return START_STICKY
    } catch (e: Exception) {
      Log.e(TAG, "establish failed", e)
      val cb = onEstablished
      onEstablished = null
      cb?.invoke(null, e.message ?: e.toString())
      teardown()
      return START_NOT_STICKY
    }
  }

  override fun onRevoke() {
    Log.w(TAG, "VPN revoked by the system")
    teardown()
    onRevoked?.invoke()
  }

  override fun onDestroy() {
    teardown()
    super.onDestroy()
  }

  private fun teardown() {
    running = false
    try {
      tun?.close()
    } catch (_: Exception) {
    }
    tun = null
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
      stopForeground(STOP_FOREGROUND_REMOVE)
    } else {
      @Suppress("DEPRECATION")
      stopForeground(true)
    }
    stopSelf()
  }

  private fun startForegroundCompat(session: String) {
    val nm = getSystemService(NotificationManager::class.java)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      nm.createNotificationChannel(
        NotificationChannel(CHANNEL, "VPN", NotificationManager.IMPORTANCE_LOW).apply {
          setShowBadge(false)
        }
      )
    }
    val open = PendingIntent.getActivity(
      this, 0,
      Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
    )
    val notification: Notification = (
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) Notification.Builder(this, CHANNEL)
      else @Suppress("DEPRECATION") Notification.Builder(this)
      )
      .setSmallIcon(R.mipmap.ic_launcher)
      .setContentTitle("Mfork")
      .setContentText("$session · VPN 已连接")
      .setContentIntent(open)
      .setOngoing(true)
      .build()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
      startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
    } else {
      startForeground(NOTIFICATION_ID, notification)
    }
  }
}
