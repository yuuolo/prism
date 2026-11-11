package dev.prism.android

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import org.json.JSONArray
import org.json.JSONObject
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Credentials and subscription URLs never enter plaintext preferences or backups. */
class ProfileStore(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("prism-mobile", Context.MODE_PRIVATE)
    companion object { private val lock = Any(); private const val ALIAS = "prism-mobile-profile-v1" }
    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
        }.generateKey()
    }
    fun load(): JSONObject = synchronized(lock) {
        val value = prefs.getString("profile", null) ?: return@synchronized JSONObject().put("nodes", JSONArray()).put("direct", JSONArray()).put("selected", 0)
        val bytes = Base64.decode(value, Base64.NO_WRAP)
        require(bytes.size >= 29)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes.copyOfRange(0, 12)))
        JSONObject(String(cipher.doFinal(bytes.copyOfRange(12, bytes.size)), Charsets.UTF_8))
    }
    fun save(value: JSONObject) = synchronized(lock) {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val bytes = cipher.iv + cipher.doFinal(value.toString().toByteArray(Charsets.UTF_8))
        check(prefs.edit().putString("profile", Base64.encodeToString(bytes, Base64.NO_WRAP)).commit())
    }
    fun session(): String {
        val profile = load()
        val nodes = profile.getJSONArray("nodes")
        require(nodes.length() > 0)
        val selected = profile.optInt("selected", 0).coerceIn(0, nodes.length()-1)
        return JSONObject().put("node", nodes.getJSONObject(selected)).put("direct", profile.optJSONArray("direct") ?: JSONArray()).toString()
    }
    fun import(raw: String, append: Boolean, sourceUrl: String? = null): String = synchronized(lock) {
        require(raw.length <= 4 * 1024 * 1024) { "导入内容超过 4 MiB" }
        check(PrismVpnService.stage !in listOf("已连接","正在连接","正在断开")) { "请先断开再修改配置" }
        val imported = JSONObject(CoreBridge.importProfile(raw))
        require(!imported.has("error")) { imported.optString("error") }
        val incoming = imported.getJSONArray("nodes")
        if (append) require(incoming.length() == 1) { "添加单个节点时只能输入一个节点" }
        val state = load()
        val oldNodes = state.getJSONArray("nodes")
        val oldName = oldNodes.optJSONObject(state.optInt("selected"))?.optString("name")
        val next = if (append) JSONArray(oldNodes.toString()).apply { put(incoming.getJSONObject(0).put("_local",true)) }
            else incoming.apply { for (i in 0 until oldNodes.length()) if (oldNodes.getJSONObject(i).optBoolean("_local")) put(oldNodes.getJSONObject(i)) }
        require(next.length() <= 2000) { "最多保存 2000 个节点" }
        var selection = 0
        for (i in 0 until next.length()) if (next.getJSONObject(i).optString("name") == oldName) { selection = i; break }
        state.put("nodes", next).put("selected", selection)
        if (!append) state.put("sourceUrl", sourceUrl ?: "")
        save(state)
        "已保存 ${incoming.length()} 个节点；跳过 ${imported.optInt("skipped")} 项。只导入节点，不执行订阅内桌面规则。"
    }
}
