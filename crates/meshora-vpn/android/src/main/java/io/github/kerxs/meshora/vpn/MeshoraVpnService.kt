package io.github.kerxs.meshora.vpn

import android.net.VpnService
import android.os.Build
import android.system.OsConstants
import android.util.Log

/**
 * Meshora 的 VPN 服务。只做一件事：按 Rust 那边给的地址建网卡，把描述符交出去。
 * 报文的读写、加密都在 Rust 里（同一个进程）。
 *
 * 网卡只接管 overlay 网段（addAddress 自带这条路由）和额外给的广播、组播路由，上网的流量不经过它；
 * Meshora 自己被排除在外，免得自己的 UDP 绕回自己。
 */
class MeshoraVpnService : VpnService() {
    companion object {
        private const val TAG = "Meshora"

        @Volatile
        var instance: MeshoraVpnService? = null
            private set

        /** 等服务的 onCreate 跑完（startService 是异步的）。不能在主线程上等：onCreate 就在主线程上跑 */
        fun awaitInstance(timeoutMs: Long): MeshoraVpnService? {
            val deadline = System.currentTimeMillis() + timeoutMs
            while (System.currentTimeMillis() < deadline) {
                instance?.let { return it }
                Thread.sleep(50)
            }
            return instance
        }
    }

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    override fun onDestroy() {
        instance = null
        super.onDestroy()
    }

    /** 用户在系统设置里断开了 VPN，或者别的 VPN 顶替了它。网卡已经没了，Rust 那边读写会出错、自己停下 */
    override fun onRevoke() {
        Log.w(TAG, "VPN 权限被收回")
        stopSelf()
    }

    /** 建网卡，返回描述符（所有权交给调用方） */
    fun establish(address: String, prefix: Int, mtu: Int, routes: List<String>): Int {
        val builder = Builder()
            .setSession("Meshora")
            .setMtu(mtu)
            .addAddress(address, prefix)
        for (route in routes) {
            val parts = route.split("/")
            try {
                builder.addRoute(parts[0], parts[1].toInt())
            } catch (e: Exception) {
                // 有的系统不收广播、组播路由：跳过，单播照样能用
                Log.w(TAG, "跳过路由 $route：$e")
            }
        }
        // 网卡只有 IPv4：不说一声的话，系统会把别的 App 的 IPv6 流量整个拦掉（防泄漏的默认做法）。
        // 放行 IPv6，它照常走原来的网络 —— 上网不受影响，Meshora 打洞也用得上公网 IPv6
        builder.allowFamily(OsConstants.AF_INET6)
        try {
            builder.addDisallowedApplication(packageName)
        } catch (e: Exception) {
            Log.w(TAG, "没能把自己排除在 VPN 外：$e")
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            // 网卡不代表计费网络：别让系统因为它把别的 App 当成在用流量
            builder.setMetered(false)
        }
        val tun = builder.establish()
            ?: throw IllegalStateException("没有 VPN 权限，或者权限刚被收回")
        Log.i(TAG, "网卡已建好：$address/$prefix，MTU $mtu")
        return tun.detachFd()
    }
}
