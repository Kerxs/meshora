package io.github.kerxs.meshora.vpn

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.net.VpnService
import android.os.Build
import android.util.Log
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.PermissionState
import app.tauri.plugin.Plugin

@InvokeArg
class OpenArgs {
    lateinit var url: String
}

@InvokeArg
class EstablishArgs {
    lateinit var address: String
    var prefix: Int = 10
    var mtu: Int = 1280
    var routes: List<String> = emptyList()
}

/** 安卓 17（API 37）：局域网访问要运行时权限 */
private const val API_LOCAL_NETWORK = 37
private const val LOCAL_NETWORK = "localNetwork"

/**
 * Rust 那边（tauri-plugin-meshora-vpn）调的命令：prepare、establish、open、stop。
 */
@TauriPlugin(
    permissions = [
        Permission(strings = ["android.permission.ACCESS_LOCAL_NETWORK"], alias = LOCAL_NETWORK)
    ]
)
class VpnPlugin(private val activity: Activity) : Plugin(activity) {

    /**
     * 要 VPN 权限：给过了直接答"有"，没给过弹系统对话框。
     * 有了 VPN 权限，再要局域网权限（安卓 17 起才有这个权限）：没有它，往同一个局域网里的朋友、
     * 往路由器发的 UDP 全被系统拦下（EPERM）。拒了也照样能用，只是走不了局域网
     */
    @Command
    fun prepare(invoke: Invoke) {
        activity.runOnUiThread {
            val intent = VpnService.prepare(activity)
            if (intent == null) {
                askLocalNetwork(invoke)
            } else {
                startActivityForResult(invoke, intent, "prepared")
            }
        }
    }

    @ActivityCallback
    private fun prepared(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode == Activity.RESULT_OK) {
            askLocalNetwork(invoke)
        } else {
            invoke.resolve(JSObject().apply { put("granted", false) })
        }
    }

    private fun askLocalNetwork(invoke: Invoke) {
        if (Build.VERSION.SDK_INT < API_LOCAL_NETWORK || localNetworkGranted()) {
            answer(invoke, true)
        } else {
            requestPermissionForAlias(LOCAL_NETWORK, invoke, "localNetworkAnswered")
        }
    }

    @PermissionCallback
    private fun localNetworkAnswered(invoke: Invoke) {
        val granted = localNetworkGranted()
        if (!granted) {
            Log.w("Meshora", "没有局域网权限：同一个局域网里的朋友、路由器都发不过去")
        }
        answer(invoke, granted)
    }

    private fun localNetworkGranted() = getPermissionState(LOCAL_NETWORK) == PermissionState.GRANTED

    /** VPN 权限已经有了；`localNetwork` 说局域网能不能发 */
    private fun answer(invoke: Invoke, localNetwork: Boolean) {
        invoke.resolve(JSObject().apply {
            put("granted", true)
            put("localNetwork", localNetwork)
        })
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

    /** 交给浏览器打开（下载新版本的 APK）。下载完由系统安装器装，它会核对签名和装着的是同一把钥匙 */
    @Command
    fun open(invoke: Invoke) {
        val args = invoke.parseArgs(OpenArgs::class.java)
        try {
            val intent = Intent(Intent.ACTION_VIEW, Uri.parse(args.url))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            activity.startActivity(intent)
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject("打不开下载地址：${e.message}")
        }
    }

    @Command
    fun stop(invoke: Invoke) {
        MeshoraVpnService.instance?.stopSelf()
        invoke.resolve()
    }
}
