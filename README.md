# Prism

类似 Clash Verge Rev 的桌面代理客户端，带**自研 Rust 代理内核**（`prism-core`），并可一键切换到 **mihomo** 或 **sing-box** 内核。

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
| Clash YAML            | 原样使用（覆盖端口 / API 设置）      | 转换为 sing-box JSON（rule_set 远程 srs） |
| sing-box JSON         | 转换为 Clash YAML                   | 原样使用（inbounds 替换为 mixed）  |
| 分享链接 / Base64     | 转换为 Clash YAML + 默认分组和规则   | 转换为 sing-box JSON              |

## prism-core 功能

- 入站：mixed（SOCKS5 + HTTP CONNECT + 普通 HTTP）单端口
- 出站：**AnyTLS**、Shadowsocks AEAD（aes-128/256-gcm、chacha20-ietf-poly1305）、Trojan、VLESS（TCP/TLS）、SOCKS5、HTTP、DIRECT、REJECT
- 代理组：select / url-test / fallback / load-balance，以及 GLOBAL
- 规则：DOMAIN、DOMAIN-SUFFIX、DOMAIN-KEYWORD、DOMAIN-REGEX、GEOSITE、GEOIP、IP-CIDR(6)、SRC-IP-CIDR、DST-PORT、SRC-PORT、IN-TYPE、MATCH；读取 v2ray 格式 `geosite.dat` / `geoip.dat`
- 模式：规则 / 全局 / 直连
- Clash API：`/proxies`、`/proxies/{name}/delay`、`/group/{name}/delay`、`/connections`、`/rules`、`/configs`、`/traffic`、`/version`

**暂不支持**（需要时切换到 mihomo / sing-box 内核）：UDP、TUN、DNS 模块 / fake-ip、VMess、Hysteria2、VLESS Reality/Vision、WebSocket/gRPC 传输、proxy-providers、rule-providers。

## 开发

需要 Rust（stable，MSVC）、Node 18+、WebView2。

```bash
npm install
npm run app          # 先编译 prism-core，再 tauri dev
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

打包安装程序（NSIS）：

```bash
npm run bundle
```

## 目录结构

```
crates/prism-core/     自研内核（库 + prism-core 可执行文件）
  src/config/          Clash / sing-box / 分享链接 解析与互转
  src/outbound/        anytls、shadowsocks、trojan、vless、socks5/http、代理组
  src/rules.rs geo.rs  规则匹配、geosite/geoip.dat 解码
  src/api.rs           Clash 兼容 REST API
src-tauri/             Tauri 后端：内核进程、订阅、系统代理、下载、托盘
src/                   React 界面
```

数据目录：`%APPDATA%\dev.prism.client\`，包含 `profiles/`、`runtime/`（生成的内核配置与 geodata）、`cores/`（下载的 mihomo / sing-box）。
