package app.mfork.desktop

import android.app.Activity
import android.content.Intent
import android.net.VpnService
import android.os.Build
import android.webkit.WebView
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class EstablishArgs {
  var mtu: Int = 1500
  var ipv6: Boolean = false
  var session: String = "Mfork"
}

@InvokeArg
class AttachArgs {
  var port: Int = 0
  var secret: String = ""
}

/**
 * Tauri mobile plugin "mfork-vpn": the bridge between the Rust core
 * manager and [MforkVpnService]. Called from Rust via run_mobile_plugin;
 * `prepare` is also exposed to the frontend so the consent dialog can be
 * shown before the core is started.
 */
@TauriPlugin
class VpnPlugin(private val activity: Activity) : Plugin(activity) {
  private var webView: WebView? = null

  override fun load(webView: WebView) {
    this.webView = webView
    MforkVpnService.onStopped = { reason ->
      // no plugin event channel needed: poke the page directly
      activity.runOnUiThread {
        this.webView?.evaluateJavascript(
          "window.dispatchEvent(new CustomEvent('mfork-vpn-stopped', {detail: '$reason'}))", null
        )
      }
    }
  }

  /** Ask for the system VPN consent if it was not granted yet. Resolves {granted}. */
  @Command
  fun prepare(invoke: Invoke) {
    val intent = VpnService.prepare(activity)
    if (intent == null) {
      val out = JSObject()
      out.put("granted", true)
      invoke.resolve(out)
      return
    }
    startActivityForResult(invoke, intent, "onPrepared")
  }

  @ActivityCallback
  fun onPrepared(invoke: Invoke, result: ActivityResult) {
    val out = JSObject()
    out.put("granted", result.resultCode == Activity.RESULT_OK)
    invoke.resolve(out)
  }

  /** Start the VPN service and resolve {fd} once the TUN interface exists. */
  @Command
  fun establish(invoke: Invoke) {
    val args = invoke.parseArgs(EstablishArgs::class.java)
    if (VpnService.prepare(activity) != null) {
      invoke.reject("VPN 未授权")
      return
    }
    MforkVpnService.onEstablished = { fd, err ->
      if (fd != null) {
        val out = JSObject()
        out.put("fd", fd)
        invoke.resolve(out)
      } else {
        invoke.reject(err ?: "establish failed")
      }
    }
    val intent = Intent(activity, MforkVpnService::class.java)
      .setAction(MforkVpnService.ACTION_START)
      .putExtra(MforkVpnService.EXTRA_MTU, args.mtu)
      .putExtra(MforkVpnService.EXTRA_IPV6, args.ipv6)
      .putExtra(MforkVpnService.EXTRA_SESSION, args.session)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      activity.startForegroundService(intent)
    } else {
      activity.startService(intent)
    }
  }

  /** The core's API is up: let the service mirror its traffic into the notification. */
  @Command
  fun attachCore(invoke: Invoke) {
    val args = invoke.parseArgs(AttachArgs::class.java)
    MforkVpnService.attachCore(args.port, args.secret)
    invoke.resolve()
  }

  @Command
  fun stop(invoke: Invoke) {
    activity.startService(
      Intent(activity, MforkVpnService::class.java).setAction(MforkVpnService.ACTION_STOP)
    )
    invoke.resolve()
  }

  @Command
  fun status(invoke: Invoke) {
    val out = JSObject()
    out.put("running", MforkVpnService.running)
    out.put("granted", VpnService.prepare(activity) == null)
    invoke.resolve(out)
  }
}
