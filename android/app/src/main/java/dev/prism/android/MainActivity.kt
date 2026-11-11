package dev.prism.android

import android.app.*
import android.content.*
import android.content.pm.PackageManager
import android.net.VpnService
import android.os.*
import android.view.View
import android.widget.*
import org.json.JSONArray
import org.json.JSONObject
import java.net.URL
import java.util.concurrent.Executors
import javax.net.ssl.HttpsURLConnection

class MainActivity : Activity() {
    private lateinit var root: LinearLayout
    private lateinit var store: ProfileStore
    private val io = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())
    private var tab = 0
    private var status: TextView? = null
    private var counters: TextView? = null
    private var connect: Button? = null
    private var nodes: Spinner? = null
    @Volatile private var busy = false
    private var nodeCount = 0
    private val tick = object : Runnable { override fun run() { updateStatus(); main.postDelayed(this, 1000) } }
    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()
    private fun label(value: String, size: Float = 15f): TextView = TextView(this).apply { text=value; textSize=size; setPadding(0,dp(8),0,dp(8)); root.addView(this) }
    private fun field(hintText: String, lines: Int = 1): EditText = EditText(this).apply { hint=hintText; minLines=lines; maxLines=if(lines==1) 1 else 6; isSingleLine=lines==1; root.addView(this) }
    private fun button(value: String, action: () -> Unit): Button = Button(this).apply { text=value; setOnClickListener { action() }; root.addView(this,LinearLayout.LayoutParams(-1,-2)) }
    private fun message(value: String) { Toast.makeText(this,value,Toast.LENGTH_LONG).show() }
    private fun idle() = PrismVpnService.stage !in listOf("已连接","正在连接","正在断开")
    private fun editable(): Boolean {
        if (busy) { message("请等待当前操作完成"); return false }
        if (!idle()) { message("修改配置前请先断开"); return false }
        return true
    }
    private fun runTask(action: () -> String) {
        if (busy) { message("请等待当前操作完成"); return }
        busy=true; updateStatus()
        io.execute {
            val result = try { action() } catch (_: UnsatisfiedLinkError) { "未找到匹配当前 CPU 的内核" } catch (e: UserMessage) { e.message ?: "操作失败" } catch (_: Exception) { "操作失败，请检查格式、网络或本地存储；原配置保留" }
            main.post { busy=false; if (!isDestroyed) { message(result); render() } }
        }
    }
    override fun onCreate(saved: Bundle?) { super.onCreate(saved); store=ProfileStore(this); render() }
    private fun render() {
        val scroller=ScrollView(this)
        root=LinearLayout(this).apply { orientation=LinearLayout.VERTICAL; setPadding(dp(20),dp(20),dp(20),dp(28)) }
        root.setOnApplyWindowInsetsListener { view,insets -> view.setPadding(dp(20)+insets.systemWindowInsetLeft,dp(20)+insets.systemWindowInsetTop,dp(20)+insets.systemWindowInsetRight,dp(28)+insets.systemWindowInsetBottom); insets }
        scroller.addView(root); setContentView(scroller); root.requestApplyInsets()
        status=null; counters=null; connect=null; nodes=null
        label("Prism Android",26f)
        label("精简测试版 ${BuildConfig.VERSION_NAME} · Android 8.0+ / 64 位",12f)
        val tabs=LinearLayout(this)
        listOf("连接","节点 / 订阅","设置").forEachIndexed { i,title ->
            tabs.addView(Button(this).apply {text=title; isSelected=tab==i; setOnClickListener {tab=i; render()}}, LinearLayout.LayoutParams(0,-2,1f))
        }; root.addView(tabs)
        when(tab) { 0 -> connectionPage(); 1 -> profilesPage(); else -> settingsPage() }
    }
    private fun connectionPage() {
        status=label(PrismVpnService.stage,23f)
        val state=try {store.load()} catch (_: Exception) {label("无法读取加密配置，请检查本机存储后重新导入"); JSONObject().put("nodes",JSONArray())}
        val choices=state.getJSONArray("nodes"); nodeCount=choices.length()
        val spinner=Spinner(this)
        spinner.adapter=ArrayAdapter(this,android.R.layout.simple_spinner_dropdown_item,if(nodeCount==0) listOf("请先添加节点") else (0 until nodeCount).map {choices.getJSONObject(it).optString("name","节点 ${it+1}")})
        if(nodeCount>0) spinner.setSelection(state.optInt("selected").coerceIn(0,nodeCount-1))
        root.addView(spinner); nodes=spinner
        spinner.onItemSelectedListener=object : AdapterView.OnItemSelectedListener {
            override fun onNothingSelected(parent: AdapterView<*>?) {}
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                if(idle() && nodeCount>0 && state.optInt("selected")!=position) {state.put("selected",position); runCatching {store.save(state)}}
            }
        }
        connect=button("连接") {
            if(!idle()) {startService(Intent(this,PrismVpnService::class.java).setAction(PrismVpnService.DISCONNECT))}
            else if(nodeCount>0) {
                val permission=VpnService.prepare(this)
                if(permission!=null) startActivityForResult(permission,100) else startVpn()
            }
        }
        counters=label("↑ 0 B    ↓ 0 B")
        button("测试当前节点") {
            if(PrismVpnService.stage!="已连接") message("请先连接")
            else runTask {
                val result=JSONObject(CoreBridge.probe())
                if(result.has("error")) result.getString("error") else "当前节点往返探测：${result.getLong("ms")} ms"
            }
        }
        label("完整接管 IPv4 / IPv6；首版使用 IPv4 目标，IPv6 目标在隧道内阻断。DNS 使用经所选节点访问的 DoH，节点域名使用独立引导解析。",13f)
        label("保持前台 VPN 服务运行。首版未承诺系统常开 VPN 或断网保护；Tailscale 与桌面高级功能不包含在此版本。",13f)
        updateStatus()
    }
    private fun startVpn() { startForegroundService(Intent(this,PrismVpnService::class.java).setAction(PrismVpnService.CONNECT)) }
    @Deprecated("Platform callback")
    override fun onActivityResult(request: Int, result: Int, data: Intent?) { super.onActivityResult(request,result,data); if(request==100 && result==RESULT_OK) startVpn() }
    private fun updateStatus() {
        if(tab!=0) return
        val stage=PrismVpnService.stage
        status?.text=stage + if(PrismVpnService.detail.isEmpty()) "" else "\n${PrismVpnService.detail}"
        connect?.text=when(stage) {"已连接","正在连接" -> "断开"; "正在断开" -> "正在断开"; else -> "连接"}
        connect?.isEnabled=stage!="正在断开" && (nodeCount>0 || !idle()) && (!busy || !idle())
        nodes?.isEnabled=idle() && nodeCount>0 && !busy
        if(stage=="已连接") runCatching {
            val stats=JSONObject(CoreBridge.status())
            counters?.text="↑ ${formatBytes(stats.optLong("upload"))}    ↓ ${formatBytes(stats.optLong("download"))}"
        }
    }
    private fun profilesPage() {
        val state=try {store.load()} catch (_: Exception) {label("无法读取加密配置，请检查本机存储后重试"); return}
        label("当前保存 ${state.getJSONArray("nodes").length()} 个节点")
        label("仅导入节点。桌面规则、providers、脚本、链式代理和高级传输不会在手机端执行。单独添加的节点会保留，不被订阅更新覆盖。",13f)
        val url=field("HTTPS 订阅地址"); url.setText(state.optString("sourceUrl"))
        button("导入 / 手动更新订阅") {
            if(editable()) {
                val address=url.text.toString().trim()
                runTask {
                    store.import(downloadSubscription(address),false,address)
                }
            }
        }
        label("粘贴订阅内容 / 分享链接")
        val raw=field("Clash YAML、sing-box JSON 或分享链接",4)
        button("导入粘贴内容") { if(editable()) {val value=raw.text.toString(); runTask {store.import(value,false)}} }
        button("添加单个节点") {
            if(!editable()) return@button
            val input=EditText(this).apply {hint="粘贴单个节点分享链接"; minLines=2}
            AlertDialog.Builder(this).setTitle("添加单个节点").setView(input).setNegativeButton("取消",null)
                .setPositiveButton("添加") {_,_ -> val value=input.text.toString(); runTask {store.import(value,true)}}.show()
        }
        val list=state.getJSONArray("nodes")
        for(i in 0 until list.length()) {
            val node=list.getJSONObject(i)
            label("${i+1}. ${node.optString("name")} · ${node.optString("kind")}",14f)
        }
    }
    private fun settingsPage() {
        label("基础分流",20f)
        label("默认全部经所选节点。以下域名后缀或 IPv4 CIDR 可明确设为直连；其他桌面规则不会自动迁移。",13f)
        val state=try {store.load()} catch (_: Exception) {label("无法读取加密配置，请检查本机存储后重试"); return}
        val rules=state.optJSONArray("direct") ?: JSONArray()
        for(i in 0 until rules.length()) {
            val rule=rules.getJSONObject(i)
            button("${rule.getString("value")}  · 删除") { if(editable()) {rules.remove(i); state.put("direct",rules); store.save(state); render()} }
        }
        val kind=Spinner(this).apply {adapter=ArrayAdapter(this@MainActivity,android.R.layout.simple_spinner_dropdown_item,listOf("域名后缀","IPv4 CIDR"))}; root.addView(kind)
        val value=field("例如 example.com 或 192.168.1.0/24")
        button("添加直连规则") {
            if(!editable()) return@button
            val ruleKind=if(kind.selectedItemPosition==0) "domain" else "ip"
            val ruleValue=value.text.toString().trim()
            runTask {
                if(state.getJSONArray("nodes").length()==0) throw UserMessage("请先添加节点")
                val next=JSONArray(rules.toString()).put(JSONObject().put("kind",ruleKind).put("value",ruleValue))
                val validation=JSONObject(CoreBridge.validateSession(JSONObject(store.session()).put("direct",next).toString()))
                if(validation.has("error")) throw UserMessage("规则格式不正确或超出首版支持范围")
                state.put("direct",next); store.save(state); "规则已保存"
            }
        }
        if(Build.VERSION.SDK_INT>=33 && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)!=PackageManager.PERMISSION_GRANTED)
            button("允许连接状态通知") {requestPermissions(arrayOf(android.Manifest.permission.POST_NOTIFICATIONS),110)}
        label("本地诊断",20f)
        label(PrismVpnService.logs().joinToString("\n").ifEmpty {"暂无运行事件"},13f)
        label("节点和订阅地址使用 Android Keystore 加密存储；不记录节点密钥和订阅凭据。此版本没有云同步、局域网共享或桌面更新通道。",13f)
    }
    private fun downloadSubscription(address: String): String {
        var link=try {URL(address)} catch (_: Exception) {throw UserMessage("请输入 HTTPS 订阅地址")}
        repeat(4) { attempt ->
            if(link.protocol!="https" || link.userInfo!=null) throw UserMessage("订阅及重定向仅接受无用户名密码的 HTTPS 地址")
            val connection=link.openConnection() as HttpsURLConnection
            connection.connectTimeout=15000; connection.readTimeout=15000; connection.instanceFollowRedirects=false
            try {
                val code=connection.responseCode
                if(code in listOf(301,302,303,307,308)) {
                    if(attempt==3) throw UserMessage("订阅重定向次数过多")
                    val location=connection.getHeaderField("Location") ?: throw UserMessage("订阅重定向地址为空")
                    link=URL(link,location)
                } else {
                    if(code!=200) throw UserMessage("订阅请求失败，原配置保留")
                    val out=java.io.ByteArrayOutputStream()
                    connection.inputStream.use { input -> val bytes=ByteArray(8192); while(true) { val n=input.read(bytes); if(n<0) break; if(out.size()+n>4*1024*1024) throw UserMessage("订阅超过 4 MiB"); out.write(bytes,0,n) } }
                    return out.toString("UTF-8")
                }
            } finally {connection.disconnect()}
        }
        throw UserMessage("订阅请求失败，原配置保留")
    }
    private fun formatBytes(v: Long): String = when {v>=1024*1024 -> "%.1f MiB".format(v/1048576.0); v>=1024 -> "%.1f KiB".format(v/1024.0); else -> "$v B"}
    override fun onResume() {
        super.onResume()
        main.post(tick)
    }
    override fun onPause() { main.removeCallbacks(tick); super.onPause() }
    override fun onDestroy() { main.removeCallbacksAndMessages(null); io.shutdownNow(); super.onDestroy() }
    private class UserMessage(message: String) : Exception(message)
}
