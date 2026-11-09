package dev.phonebridge.app

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import org.json.JSONArray
import org.json.JSONException
import java.net.HttpURLConnection
import java.net.URL

/**
 * The plain update check. It reads GitHub's release list when the person asks and says whether a newer one is
 * listed. Nothing is checked for authenticity (Relay holds no update key), nothing is downloaded or installed,
 * and a newer release is only a link to its page. Same rules as the PC's `pb_update::releases`.
 */
const val RELEASE_LIST_URL = "https://api.github.com/repos/Lin-2352/phone-bridge/releases?per_page=10"
const val RELEASE_URL_PREFIX = "https://github.com/Lin-2352/phone-bridge/releases/"
const val RELEASE_MAX_RESPONSE = 512 * 1024

sealed class CheckResult {
    object UpToDate : CheckResult()
    data class Found(val tag: String, val url: String) : CheckResult()
    data class Failed(val why: String) : CheckResult()
}

/** `major.minor.patch` only; anything else (a pre-release suffix, a name like "nightly") is not offered. */
fun parseVersion(s: String): Triple<Int, Int, Int>? {
    val p = s.split('.')
    if (p.size != 3) return null
    val n = p.map { it.toIntOrNull()?.takeIf { v -> v >= 0 } ?: return null }
    return Triple(n[0], n[1], n[2])
}

private fun newer(a: Triple<Int, Int, Int>, b: Triple<Int, Int, Int>): Boolean =
    compareValuesBy(a, b, { it.first }, { it.second }, { it.third }) > 0

/** The highest release newer than [installed], else up to date. A bad answer is a failure, never "up to date". */
fun newestRelease(body: ByteArray, installed: String): CheckResult {
    if (body.size > RELEASE_MAX_RESPONSE) return CheckResult.Failed("the answer was too large")
    val have = parseVersion(installed) ?: return CheckResult.Failed("this app's version is unreadable")
    val list = try {
        JSONArray(String(body, Charsets.UTF_8))
    } catch (_: JSONException) {
        return CheckResult.Failed("GitHub's answer was not a release list")
    }
    var best: Triple<Int, Int, Int>? = null
    var found: CheckResult.Found? = null
    for (i in 0 until list.length()) {
        val item = list.optJSONObject(i) ?: continue
        if (item.optBoolean("draft", false)) continue
        val tag = item.optString("tag_name", "")
        val url = item.optString("html_url", "")
        val v = parseVersion(tag.removePrefix("v")) ?: continue
        if (!newer(v, have) || !url.startsWith(RELEASE_URL_PREFIX)) continue
        if (best == null || newer(v, best)) {
            best = v
            found = CheckResult.Found(tag, url)
        }
    }
    return found ?: CheckResult.UpToDate
}

/** What one network looks like to the update check. */
data class UpdateNet(val id: Long, val vpn: Boolean, val wifi: Boolean, val ethernet: Boolean, val cellular: Boolean)

/**
 * The network the update request goes out on. Relay binds the whole process to the Wi-Fi network while a VPN is
 * up (so the local network works), and a request made with that binding would skip the person's VPN and show
 * their real address to GitHub. So the request names its network: the VPN when there is one, else Wi-Fi, then
 * Ethernet, then mobile data.
 */
fun updateNetworkToUse(nets: List<UpdateNet>): Long? =
    (nets.firstOrNull { it.vpn } ?: nets.firstOrNull { it.wifi } ?: nets.firstOrNull { it.ethernet }
        ?: nets.firstOrNull { it.cellular })?.id

/** The text under the row for a result. */
fun updateNote(result: CheckResult?, installed: String): String = when (result) {
    null -> "Relay $installed. Asks GitHub only when you tap"
    is CheckResult.UpToDate -> "No newer release is listed on GitHub (this is $installed)"
    is CheckResult.Found -> "${result.tag} is listed on GitHub. Not checked for authenticity: open the page and compare"
    is CheckResult.Failed -> "Couldn't check: ${result.why}"
}

/** One HTTPS GET of the release list. Blocking: call it off the main thread. */
fun checkForUpdate(ctx: Context): CheckResult {
    val installed = runCatching {
        ctx.packageManager.getPackageInfo(ctx.packageName, 0).versionName ?: ""
    }.getOrDefault("")
    val cm = ctx.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    val nets = cm.allNetworks.mapNotNull { n ->
        val c = cm.getNetworkCapabilities(n) ?: return@mapNotNull null
        if (!c.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)) return@mapNotNull null
        UpdateNet(
            n.networkHandle,
            c.hasTransport(NetworkCapabilities.TRANSPORT_VPN),
            c.hasTransport(NetworkCapabilities.TRANSPORT_WIFI),
            c.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET),
            c.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR),
        ) to n
    }
    val id = updateNetworkToUse(nets.map { it.first }) ?: return CheckResult.Failed("no internet connection")
    val network = nets.first { it.first.id == id }.second
    return try {
        val c = network.openConnection(URL(RELEASE_LIST_URL)) as HttpURLConnection
        c.connectTimeout = 10_000
        c.readTimeout = 10_000
        c.setRequestProperty("Accept", "application/vnd.github+json")
        try {
            when (val code = c.responseCode) {
                200 -> {
                    val out = java.io.ByteArrayOutputStream()
                    val buf = ByteArray(16 * 1024)
                    c.inputStream.use { s ->
                        while (out.size() <= RELEASE_MAX_RESPONSE) {
                            val n = s.read(buf)
                            if (n < 0) break
                            out.write(buf, 0, n)
                        }
                    }
                    newestRelease(out.toByteArray(), installed)
                }
                403, 429 -> CheckResult.Failed("GitHub is limiting requests, try later")
                else -> CheckResult.Failed("GitHub did not return the release list ($code)")
            }
        } finally {
            c.disconnect()
        }
    } catch (_: java.io.IOException) {
        CheckResult.Failed("no connection")
    }
}
