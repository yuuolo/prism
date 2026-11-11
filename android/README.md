# Prism Android 精简版

独立原生 Kotlin 界面与 Android VpnService，内嵌 Rust prism-core。开发版本 `0.1.0-alpha.1`，与桌面版本、设置、签名及更新渠道分开。当前处于开发验证阶段，不作为稳定版发布。

## 范围

- 连接/断开、系统 VPN 授权、前台连接通知。
- HTTPS 订阅手动导入/更新、粘贴配置和单节点添加；单独添加的节点保留。
- 节点选择、按需测试当前节点、基础域名后缀/IPv4 CIDR 直连规则。
- IPv4/IPv6 均路由进入 VPN；首版仅开放 IPv4 目标，IPv6 目标在内核拒绝，避免移除 IPv6 路由造成旁路。
- DNS TCP/UDP 53 由内核处理，使用经所选节点的 DoH；代理服务器域名使用独立 IP 地址 DoH 引导，普通查询不回退系统 DNS。
- 用户配置和订阅凭据使用 Android Keystore AES-GCM 加密，不参加系统备份；诊断事件有容量上限，不记录节点密钥和订阅 URL。

首版允许基础 TCP/TLS AnyTLS、Shadowsocks、Trojan、VLESS、SOCKS5、HTTP 节点；禁用跳过证书校验、链式代理、指纹/Reality/Vision 和高级传输。不支持的节点会跳过并显示数量，不能用时明确报错。订阅只导入节点，不执行桌面规则、providers、覆写或脚本；手机规则由用户独立设置。

不包含 Tailscale、多内核下载、桌面服务、地图、应用流量统计、云同步和桌面看门狗。首版不启用系统常开 VPN，也不承诺进程被系统终止后的断网保护。

## 生命周期

所有 Rust TCP/UDP 出站 socket 在连接前调用 VpnService.protect，防止回环。Java 的普通应用请求仍进入 VPN；不全局排除自身 UID。手机系统管理底层网络，底层网络变化时有间隔限制地重建连接池/DNS，保留 VPN 描述符；不以流量空闲反复重启。

用户断开和撤销 VPN 授权使旧连接请求失效；关闭 native runtime 后再释放 Java 的 TUN 描述符。后台不持续测速、不轮询连接列表、不持有无限保活任务。

## 构建

最低计划 Android 8.0/API 26，64 位（arm64-v8a；x86_64 用于模拟器）。主要适配和测试目标为 Android 17/API 37。尚需分别验证旧系统和具体厂商设备。

依赖：JDK 17、Android SDK 37、build-tools 36.0.0、NDK 28.2.13676358、Rust stable，以及 arm64/x86_64 Android targets。Gradle Wrapper 和分发包摘要已固定；AGP 9.1.1 使用内置 Kotlin。

```sh
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk --version 4.0.0 --locked
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/28.2.13676358"
bash scripts/android/build-native.sh
bash android/gradlew -p android assembleDebug
```

产物：`android/app/build/outputs/apk/debug/app-debug.apk`。这是调试签名 APK，不复用桌面更新密钥；正式分发前需要确定持久的 Android 发布签名。不同调试签名不能互相覆盖安装。

CI：`.github/workflows/android.yml` 构建 ARM64 与 x86_64 JNI 库、APK、Lint，并在 Android 17 模拟器进行真实 VPN/DNS/TCP、IPv6 阻断、会话重建和断开测试。测试使用本地 SOCKS 服务，不使用真实订阅或远程账号。模拟器授权只作用于隔离 CI 设备，普通手机仍需要用户确认系统 VPN 对话框。

在真机完成订阅节点、Wi-Fi/蜂窝切换、锁屏、后台、省电、撤权、内存和耗电验证前，不宣称已达到稳定发布标准。
