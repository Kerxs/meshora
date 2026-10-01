package io.github.kerxs.meshora.vpn

import android.app.Activity
import android.content.Intent
import android.net.VpnService
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
    lateinit var address: String
    var prefix: Int = 10
    var mtu: Int = 1280
    var routes: List<String> = emptyList()
}

/**
 * Rust 那边（tauri-plugin-meshora-vpn）调的三个命令：prepare、establish、stop。
 */
@TauriPlugin
class VpnPlugin(private val activity: Activity) : Plugin(activity) {

    /** 要 VPN 权限：给过了直接答"有"，没给过弹系统对话框 */
    @Command
    fun prepare(invoke: Invoke) {
        activity.runOnUiThread {
            val intent = VpnService.prepare(activity)
            if (intent == null) {
                invoke.resolve(JSObject().apply { put("granted", true) })
            } else {
                startActivityForResult(invoke, intent, "prepared")
            }
        }
    }

    @ActivityCallback
    private fun prepared(invoke: Invoke, result: ActivityResult) {
        invoke.resolve(JSObject().apply { put("granted", result.resultCode == Activity.RESULT_OK) })
    }

    /** 起服务、建网卡、交回描述符。在后台线程里做：要等服务的 onCreate，它在主线程上跑 */
    @Command
    fun establish(invoke: Invoke) {
        val args = invoke.parseArgs(EstablishArgs::class.java)
        Thread {
            try {
                if (MeshoraVpnService.instance == null) {
                    activity.startService(Intent(activity, MeshoraVpnService::class.java))
                }
                val service = MeshoraVpnService.awaitInstance(5000)
                    ?: throw IllegalStateException("VPN 服务没能启动")
                val fd = service.establish(args.address, args.prefix, args.mtu, args.routes)
                invoke.resolve(JSObject().apply { put("fd", fd) })
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.toString())
            }
        }.start()
    }

    @Command
    fun stop(invoke: Invoke) {
        MeshoraVpnService.instance?.stopSelf()
        invoke.resolve()
    }
}
