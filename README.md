# Prism

类似 Clash Verge Rev 的桌面代理客户端，带**自研 Rust 代理内核**（`prism-core`），并可一键切换到 **mihomo** 或 **sing-box** 内核。

**[使用手册](docs/manual.md)**：安装与界面说明、系统代理与 TUN、订阅格式、配置语法（Clash YAML / 分享链接 / sing-box）、命令行与常见问题。

```
┌──────────── Prism GUI (Tauri 2 + React) ────────────┐
│ 概览 · 代理 · 订阅 · 连接 · 规则 · 日志 · 设置        │
└───────────────┬─────────────────────────────────────┘
                │ Clash REST API（三个内核都实现了它）
     ┌──────────┼───────────────┬─────────────────┐
  prism-core（自研）        mihomo            sing-box (clash_api)
```

GUI 只和 Clash 兼容 API 通信；订阅会按所选内核自动转换：

| 订阅格式              | prism-core / mihomo                 | sing-box                          |
|-----------------------|-------------------------------------|-----------------------------------|
| Clash YAML            | 原样使用（覆盖端口 / API 设置）      | 转换为 sing-box JSON（geosite/geoip 规则集） |
| sing-box JSON         | 转换为 Clash YAML                   | 原样使用（inbounds 替换为 mixed）  |
| 分享链接 / Base64     | 转换为 Clash YAML + 默认分组和规则   | 转换为 sing-box JSON              |

## prism-core 功能

- 入站：mixed（SOCKS5 + HTTP CONNECT + 普通 HTTP）单端口，含 SOCKS5 UDP ASSOCIATE；TUN
- 出站：**AnyTLS**（会话复用，新连接不再重复握手）、Shadowsocks AEAD（aes-128/256-gcm、chacha20-ietf-poly1305）、Trojan、VLESS（TCP/TLS）、SOCKS5、HTTP、DIRECT、REJECT
- **UDP 经代理**：Shadowsocks UDP、Trojan UDP、VLESS UDP、AnyTLS（sing-box UDP-over-TCP v2）、SOCKS5 UDP ASSOCIATE、直连（每个目标一条会话）
- 代理组：select / url-test / fallback / load-balance，以及 GLOBAL；`use` / `filter` / `exclude-filter`
- 规则：DOMAIN、DOMAIN-SUFFIX、DOMAIN-KEYWORD、DOMAIN-REGEX、GEOSITE、GEOIP、IP-CIDR(6)、SRC-IP-CIDR、DST-PORT、SRC-PORT、IN-TYPE、NETWORK、RULE-SET、MATCH；读取 v2ray 格式 `geosite.dat` / `geoip.dat`
- **DNS 模块**：UDP / TCP / DoT / DoH 上游，`#代理` 经代理查询，nameserver-policy（含 `geosite:`）、fake-ip + fake-ip-filter、缓存、hosts、`listen` 端口
- **proxy-providers / rule-providers**：http / file / inline，缓存文件、定时刷新、内容变化自动热重载（domain / ipcidr / classical，yaml / text）
- 热重载：`PUT /configs`，监听端口不中断，节点选择和 fake-ip 映射保留
- 模式：规则 / 全局 / 直连
- 延迟测试支持 mihomo 的 `unified-delay`（只计第二次请求），设置页「统一延迟」开关
- Clash API：`/proxies`、`/proxies/{name}/delay`、`/group/{name}/delay`、`/connections`、`/rules`、`/configs`、`/traffic`、`/providers/proxies`、`/providers/rules`、`/version`

- TUN 模式（Windows，需管理员权限 + wintun.dll）：wintun 虚拟网卡 + smoltcp 用户态协议栈；自动路由 0.0.0.0/1、128.0.0.0/1，内核自身连接绑定物理网卡防回环；嗅探 TLS SNI / HTTP Host 按域名分流；DNS 由 DNS 模块应答（未启用时转发上游）；UDP 按规则分流、可经代理。命令行：`prism-core -f config.yaml --tun`

- **应用分流**：PROCESS-NAME / PROCESS-PATH（含 `-REGEX`）规则，通过 Windows TCP/UDP 连接表识别程序；客户端「应用分流」页按程序选择出口，三种内核通用
- **软件更新**：启动时检查 GitHub Releases，下载后校验发布签名（minisign，版本绑定）再安装
- **代理页**：全部测速、按延迟 / 名称排序、搜索节点；托盘菜单直接切换节点；**解锁检测**（Netflix、YouTube Premium、ChatGPT、Claude、Gemini 等）
- **覆写**：全局或按订阅的 Merge（YAML）和 Script（JavaScript，内置 Boa 引擎）
- **备份与同步**：WebDAV 云端备份 / 恢复、本地导出导入（密码用 DPAPI 加密）
- **外观**：浅色 / 深色主题、中文 / English 界面、全局快捷键、最近 30 天流量统计
- **TUN 服务**：`prism-service`（Windows 服务）替普通权限的 Prism 运行 TUN 内核；只运行受保护目录中的内核、只接受安装用户的 Prism 请求
- **开机自启 / 静默启动**：登录 Windows 时启动并只显示托盘；单实例运行
- **订阅定时更新**：按订阅的间隔（默认 24 小时，或机场 `profile-update-interval`）自动更新，正在使用的订阅在 prism-core / mihomo 上热重载、不断开连接
- **一键导入**：`clash://install-config?url=…&name=…` 与 `prism://` 链接，导入前弹窗确认；`clash://` 可选接管，关闭时还原给原来的客户端

- **TLS 指纹**：`client-fingerprint` 时使用自研 TLS 1.3 客户端，ClientHello 与 Chrome 一致（JA4 `t13d1516h2_8daaf6152771_d8a2da3f94cd`，X25519MLKEM768）
- **VLESS Reality / Vision**（UDP 经 XUDP）、**Hysteria2**（QUIC，salamander 混淆，UDP 分片）
- **TUIC v5**（native / quic 两种 UDP 模式，cubic / new_reno / bbr）、**WireGuard**（程序内协议栈，支持 WARP 的 reserved）
- **链式代理**：`dialer-proxy` 和 `relay` 组，UDP 类协议也可经前一跳转发；**组合规则** `AND` / `OR` / `NOT` 与 `SRC-GEOIP`

**暂不支持**（需要时切换到 mihomo / sing-box 内核）：VMess、WebSocket/gRPC 传输、Shadowsocks 插件、`mrs` 格式规则集、DNS fallback、QUIC/DoH3 上游、TUN 的 IPv6 路由。

> prism-core 的 TUN 已在真实网卡上使用：路由接管、DNS 劫持 + fake-ip、直连 TCP、经代理的 UDP 均正常；经代理的 TCP 曾因 fake-ip 被 IP 规则误判为直连，已修复（`crates/prism-core/tests/fake_ip_rules.rs`）。

## 开发

需要 Rust（stable，MSVC）、Node 18+、WebView2。

```bash
npm install
npm run app          # 先编译 prism-core，再 tauri dev
npm run dev          # 只启动界面：在普通浏览器打开 http://localhost:1420 时使用示例数据预览
```

单独运行内核（参数与 mihomo 一致）：

```bash
cargo run -p prism-core -- -f config.yaml -d ./home
cargo run -p prism-core -- -f config.yaml -t       # 只检查配置
```

测试：

```bash
cargo test --workspace
```

## 发布新版本

1. 修改版本号：`Cargo.toml`（workspace）、`package.json`、`package-lock.json`、`src-tauri/tauri.conf.json`、`Cargo.lock`（`cargo update -w`）。
2. 打包并签名：

   ```bash
   npm run bundle
   ```

   需要更新签名私钥：默认读取 `%USERPROFILE%\.tauri\prism-updater.key`（或环境变量 `PRISM_SIGNING_KEY` / `TAURI_SIGNING_PRIVATE_KEY`）。可用 `PRISM_RELEASE_NOTES_FILE` 指定写入更新清单的说明。产物在 `target/release/bundle/nsis/`：
   `Prism_<版本>_x64-setup.exe`、`Prism_<版本>_x64-setup.exe.sig`、`latest.json`。
3. 创建 GitHub Release（tag `v<版本>`），**上传这三个文件**。客户端检查的是 `releases/latest/download/latest.json`。

> **私钥务必备份。** 公钥写在 `src-tauri/tauri.conf.json` 中，已安装的客户端只接受该私钥签名的更新。私钥丢失后无法再推送自动更新，用户需要手动重装一次换了新公钥的版本。

测试更新校验（需先 `npm run bundle`）：`PRISM_UPDATE_TEST=1 cargo test -p prism --test updater`。

## 目录结构

```
crates/prism-core/     自研内核（库 + prism-core 可执行文件）
  src/config/          Clash / sing-box / 分享链接 解析与互转
  src/outbound/        anytls、shadowsocks、trojan、vless、socks5/http、代理组
  src/rules.rs geo.rs  规则匹配、geosite/geoip.dat 解码
  src/api.rs           Clash 兼容 REST API
crates/prism-service/  TUN 服务（Windows 服务，替普通权限的 Prism 运行 TUN 内核）
src-tauri/             Tauri 后端：内核进程、订阅、系统代理、下载、托盘
src/                   React 界面
```

sing-box 的规则集（`.srs`）由客户端下载到 `runtime/rule-set/` 并以本地规则集交给 sing-box：启动时不依赖网络，节点不可用也能启动；文件超过 7 天在后台刷新，下载失败的规则集及其规则会被跳过并记入日志。

数据目录：`%APPDATA%\dev.prism.client\`，包含 `profiles/`、`runtime/`（生成的内核配置与 geodata）、`cores/`（下载的 mihomo / sing-box）。
