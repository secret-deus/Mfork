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
import java.io.BufferedReader
import java.io.InputStreamReader
import java.net.HttpURLConnection
import java.net.URL
import org.json.JSONObject

/**
 * Owns the TUN interface. The core itself is a child process of the app
 * spawned by the Rust side: this service only establishes the VPN, hands
 * the raw fd back through [onEstablished], and keeps a foreground
 * notification alive while the VPN is up (docs/ANDROID.md, decision 2).
 *
 * Once the core is up, Rust calls [attachCore]; a reader thread follows
 * the core's `/traffic` stream and mirrors the speed into the
 * notification, independent of the WebView (which may be frozen in the
 * background). The notification's "断开" action and the quick-settings
 * tile stop the VPN and tell the page through [onStopped].
 */
class MforkVpnService : VpnService() {
  companion object {
    const val ACTION_START = "app.mfork.desktop.vpn.START"
    const val ACTION_STOP = "app.mfork.desktop.vpn.STOP"
    const val EXTRA_MTU = "mtu"
    const val EXTRA_IPV6 = "ipv6"
    const val EXTRA_SESSION = "session"
    /** the stop came from the user (notification / tile), not from Rust */
    const val EXTRA_FROM_USER = "fromUser"
    private const val TAG = "MforkVpn"
    private const val CHANNEL = "mfork-vpn"
    private const val NOTIFICATION_ID = 1
    private const val OPEN_APP_NOTIFICATION_ID = 2

    /** Set by the plugin before starting the service; called once with (fd, null) or (null, error). */
    @Volatile var onEstablished: ((Int?, String?) -> Unit)? = null
    /** VPN went away without Rust asking: "revoked" (system) or "user" (notification / tile). */
    @Volatile var onStopped: ((String) -> Unit)? = null
    /**
     * The system started the service on its own (always-on VPN, or a restart
     * after the app process died) while the page is alive: ask it to connect
     * through the normal path instead of putting up a bare tunnel.
     */
    @Volatile var onStartRequested: (() -> Unit)? = null
    @Volatile var running = false
      private set
    @Volatile private var instance: MforkVpnService? = null

    /** Called (via the plugin) once the core's API is reachable. */
    fun attachCore(port: Int, secret: String) {
      instance?.startTrafficReader(port, secret)
    }
  }

  private var tun: ParcelFileDescriptor? = null
  private var session = "Mfork"
  @Volatile private var reader: Thread? = null

  override fun onCreate() {
    super.onCreate()
    instance = this
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    if (intent?.action == ACTION_STOP) {
      val fromUser = intent.getBooleanExtra(EXTRA_FROM_USER, false)
      teardown()
      if (fromUser) onStopped?.invoke("user")
      return START_NOT_STICKY
    }
    if (intent?.action != ACTION_START || onEstablished == null) {
      // Not Rust asking for a TUN: the system started us (always-on VPN,
      // or a sticky restart after the app process was killed). Establishing
      // here would put up an interface with no core behind it — every
      // packet vanishes while system settings say "connected". Hand off.
      if (running && tun != null) return START_NOT_STICKY // already up, nothing to do
      Log.i(TAG, "system-initiated start (action=${intent?.action}); not establishing")
      startForegroundCompat(buildNotification("等待应用连接…"))
      val ask = onStartRequested
      if (ask != null) ask() else notifyOpenApp()
      teardown()
      return START_NOT_STICKY
    }
    val mtu = intent.getIntExtra(EXTRA_MTU, 1500)
    val ipv6 = intent.getBooleanExtra(EXTRA_IPV6, false)
    session = intent.getStringExtra(EXTRA_SESSION) ?: "Mfork"

    startForegroundCompat(buildNotification("VPN 已连接"))
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
      MforkTileService.requestUpdate(this)
      Log.i(TAG, "tun established fd=${pfd.fd} mtu=$mtu ipv6=$ipv6")
      val cb = onEstablished
      onEstablished = null
      cb?.invoke(pfd.fd, null)
      // never sticky: if the process dies the core dies with it, and a
      // resurrected service would only own a dead tunnel
      return START_NOT_STICKY
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
    onStopped?.invoke("revoked")
  }

  override fun onDestroy() {
    teardown()
    instance = null
    super.onDestroy()
  }

  private fun teardown() {
    running = false
    reader?.interrupt()
    reader = null
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
    MforkTileService.requestUpdate(this)
  }

  // ------------------------------------------------------------ traffic → notification

  private fun startTrafficReader(port: Int, secret: String) {
    reader?.interrupt()
    val t = Thread {
      var conn: HttpURLConnection? = null
      try {
        conn = (URL("http://127.0.0.1:$port/traffic").openConnection() as HttpURLConnection).apply {
          setRequestProperty("Authorization", "Bearer $secret")
          connectTimeout = 3000
          readTimeout = 0
        }
        BufferedReader(InputStreamReader(conn.inputStream)).use { br ->
          while (!Thread.currentThread().isInterrupted && running) {
            val line = br.readLine() ?: break
            if (line.isBlank()) continue
            val j = JSONObject(line)
            val text = "↓ ${rate(j.optLong("down"))}   ↑ ${rate(j.optLong("up"))}"
            getSystemService(NotificationManager::class.java)
              .notify(NOTIFICATION_ID, buildNotification(text))
          }
        }
      } catch (e: Exception) {
        if (running) Log.w(TAG, "traffic reader ended: $e")
      } finally {
        conn?.disconnect()
      }
    }
    t.isDaemon = true
    t.name = "mfork-traffic"
    reader = t
    t.start()
  }

  private fun rate(bps: Long): String = when {
    bps < 1024 -> "$bps B/s"
    bps < 1024 * 1024 -> String.format("%.0f KB/s", bps / 1024.0)
    bps < 1024L * 1024 * 1024 -> String.format("%.1f MB/s", bps / 1048576.0)
    else -> String.format("%.2f GB/s", bps / 1073741824.0)
  }

  // ------------------------------------------------------------ notification

  private fun buildNotification(text: String): Notification {
    val open = PendingIntent.getActivity(
      this, 0,
      Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
    )
    val stop = PendingIntent.getService(
      this, 1,
      Intent(this, MforkVpnService::class.java).setAction(ACTION_STOP).putExtra(EXTRA_FROM_USER, true),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
    )
    val b = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) Notification.Builder(this, CHANNEL)
    else @Suppress("DEPRECATION") Notification.Builder(this)
    b.setSmallIcon(R.mipmap.ic_launcher)
      .setContentTitle("Mfork · $session")
      .setContentText(text)
      .setContentIntent(open)
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setCategory(Notification.CATEGORY_SERVICE)
    @Suppress("DEPRECATION")
    b.addAction(Notification.Action.Builder(null, "断开", stop).build())
    return b.build()
  }

  /** The app is not running: the system's VPN request needs a human to open it. */
  private fun notifyOpenApp() {
    val open = PendingIntent.getActivity(
      this, 2,
      Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
    )
    val b = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) Notification.Builder(this, CHANNEL)
    else @Suppress("DEPRECATION") Notification.Builder(this)
    b.setSmallIcon(R.mipmap.ic_launcher)
      .setContentTitle("Mfork 未连接")
      .setContentText("系统请求开启 VPN，点击打开应用完成连接")
      .setContentIntent(open)
      .setAutoCancel(true)
    getSystemService(NotificationManager::class.java).notify(OPEN_APP_NOTIFICATION_ID, b.build())
  }

  private fun startForegroundCompat(notification: Notification) {
    val nm = getSystemService(NotificationManager::class.java)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      nm.createNotificationChannel(
        NotificationChannel(CHANNEL, "VPN", NotificationManager.IMPORTANCE_LOW).apply {
          setShowBadge(false)
        }
      )
    }
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
      startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
    } else {
      startForeground(NOTIFICATION_ID, notification)
    }
  }
}
