# Prism 使用手册（Windows）

适用版本：v0.1.1

- [1. 安装与首次使用](#1-安装与首次使用)
- [2. 界面说明](#2-界面说明)
- [3. 三个内核怎么选](#3-三个内核怎么选)
- [4. 系统代理与 TUN 模式](#4-系统代理与-tun-模式)
- [5. 订阅格式](#5-订阅格式)
- [6. 配置语法（Clash YAML）](#6-配置语法clash-yaml)
- [7. 分享链接语法](#7-分享链接语法)
- [8. sing-box JSON 与格式转换](#8-sing-box-json-与格式转换)
- [9. prism-core 命令行](#9-prism-core-命令行)
- [10. 数据目录与文件](#10-数据目录与文件)
- [11. 常见问题](#11-常见问题)

---

## 1. 安装与首次使用

1. 从 [Releases](https://github.com/yuuolo/prism/releases) 下载 `Prism_<版本>_x64-setup.exe` 并安装。安装程序未做代码签名，SmartScreen 提示时选择「更多信息 → 仍要运行」。
2. 打开 Prism，进入 **订阅** 页，粘贴订阅链接，点 **导入**。第一个导入的订阅会自动启用。
3. 回到 **概览** 页，点 **启动内核**。
4. 在 **流量接管** 卡片里打开 **系统代理**（或 **TUN 模式**，见第 4 节）。
5. 在 **代理** 页选择节点，点 **测速** 查看延迟。

之后每次打开 Prism，只要有已启用的订阅，内核会自动启动，并恢复上次的系统代理状态和节点选择。

> 默认代理端口是 **7899**（不是 Clash 常用的 7890，以免与已安装的其他客户端冲突），可在设置页修改。

关闭窗口不会退出 Prism，它会缩到托盘。真正退出请用托盘菜单的 **退出**；退出时会自动关闭系统代理并停止内核。

## 2. 界面说明

### 概览
- **实时速度 / 流量统计**：上传、下载速度曲线，总流量，活动连接数，代理端口。
- **内核**：在 Prism 内核、Mihomo、sing-box 之间切换（见第 3 节）。未安装的内核会显示「未安装 · 去设置下载」。
- **流量接管**：系统代理、TUN 模式开关（见第 4 节）。
- **代理模式**：
  - **规则**：按订阅里的规则分流（通常国内直连、其余走代理）。
  - **全局**：全部流量走 `GLOBAL` 组选中的节点。
  - **直连**：全部直连，不经过代理。
- **当前订阅**：启用中的订阅名称和前几个代理组当前选中的节点。

### 代理
- 按代理组显示节点；组的顺序与订阅中一致。
- 点击节点卡片即可选择（只有 `select` 类型的组可以手动选择；`url-test` / `fallback` / `load-balance` 是自动的）。
- 点节点上的延迟数字测单个节点，点组标题右侧的 **测速** 测整组。
- 延迟颜色：绿色 < 300 ms，黄色 < 800 ms，橙色更慢，红色「超时」表示测试失败。
- 节点选择会按订阅分别记住，重启内核或 Prism 后自动恢复。

### 订阅
- **导入**：粘贴订阅链接后导入。Prism 以 `clash.meta` 作为 User-Agent 下载，所以多数机场会返回 Clash 格式。若直连下载失败且内核正在运行，会自动经代理重试。
- **新建本地配置**：直接粘贴配置内容（Clash YAML、sing-box JSON 或分享链接）。
- 卡片上显示节点数、格式、更新时间；机场返回 `subscription-userinfo` 时还会显示已用流量 / 总流量和到期日。
- 点击卡片即切换到该订阅（内核运行中会自动重启生效）。
- **更新**：重新下载远程订阅；**全部更新**：更新所有远程订阅。
- **编辑**：修改名称和内容。注意远程订阅更新时会覆盖手动修改。
- **删除**：内核运行时不能删除正在使用的订阅，请先停止内核或切换到别的订阅。

### 连接
- 实时列出经过内核的连接：主机、入站类型、命中的规则、链路（如 `节点选择 → 自动选择 → 东京`）、速度和流量。
- 可按主机 / 规则 / 节点过滤，**暂停** 刷新，单独 **关闭** 某个连接或 **全部关闭**。

### 规则
- 显示内核实际加载的规则（不受支持而被跳过的规则不会出现在这里，原因见日志）。

### 日志
- 内核的输出和 Prism 自身的提示（以 `[prism]` 开头，紫色显示），警告为黄色，错误为红色。
- 每条经过代理的连接会记录一行，例如：
  `[TCP] 127.0.0.1:51234 --> www.google.com:443 match DomainSuffix(google.com) using 节点选择[东京]`

### 设置
| 项目 | 说明 |
|---|---|
| 内核 | 三个内核的安装状态与版本。Mihomo / sing-box 可点 **下载** 从 GitHub 获取最新版，也可填写自定义 exe 路径 |
| 混合代理端口 | HTTP 和 SOCKS5 共用的端口，默认 7899 |
| API 端口 / API 密钥 | 内核的 Clash 兼容 API，默认 9097，密钥首次启动时随机生成。一般无需修改 |
| 允许局域网连接 | 开启后局域网其他设备可把本机作为代理（监听所有网卡） |
| 日志级别 | debug / info / warning / error / silent |
| 绕过地址 | 系统代理不代理的地址，分号分隔，`<local>` 表示所有不含点的主机名 |
| 测速地址 | 延迟测试访问的 URL，默认 `https://www.gstatic.com/generate_204` |
| 统一延迟 | 开启后测速只计第二次请求，排除建立连接和 TLS 握手的耗时，结果更稳定，与 mihomo 的 `unified-delay` 相同。对 Prism 和 Mihomo 内核生效 |
| 更新 GeoSite / GeoIP | 重新下载 `geosite.dat` / `geoip.dat`；使用 sing-box 内核时同时刷新规则集 |
| 查看生成的内核配置 | 查看 Prism 根据订阅为当前内核生成的实际配置文件 |
| 打开数据目录 | 在资源管理器中打开数据目录（见第 10 节） |

修改端口、密钥、局域网、日志级别或统一延迟后点 **保存**，运行中的内核会自动重启。

### 托盘菜单
单击托盘图标显示主界面；右键菜单可切换 **系统代理**、**TUN 模式**、**规则 / 全局 / 直连模式**，以及 **退出**。

## 3. 三个内核怎么选

| | Prism 内核 | Mihomo | sing-box |
|---|---|---|---|
| 来源 | 本项目自研（Rust），随安装包自带 | 设置页下载 | 设置页下载 |
| AnyTLS / Shadowsocks / Trojan / VLESS | ✅（VLESS 仅 TCP/TLS） | ✅ | ✅ |
| VMess / Hysteria2 / Reality / WebSocket / gRPC | ❌ | ✅ | ✅ |
| 经代理转发 UDP | ❌ | ✅ | ✅ |
| TUN 模式 | ✅（未经实网测试） | ✅ | ✅ |
| DNS 模块 / fake-ip | ❌ | ✅ | ✅ |
| proxy-providers / rule-providers | ❌ | ✅ | — |

- 订阅只用 AnyTLS / Shadowsocks / Trojan / VLESS(TCP) 节点、日常网页和应用使用：**Prism 内核**即可，体积小、启动快。
- 需要游戏 / 语音通话等 UDP 流量走代理，或订阅含 VMess、Hysteria2、Reality 等：切换到 **Mihomo** 或 **sing-box**。
- 同一份订阅三个内核通用，Prism 会自动转换格式（见第 5、8 节）。Prism 内核遇到不支持的节点或规则会跳过，并在日志中给出警告。

## 4. 系统代理与 TUN 模式

**系统代理**：把 Windows 的代理设置指向 `127.0.0.1:<代理端口>`。浏览器和大多数遵守系统代理的程序会生效；游戏、命令行工具、部分应用不会走系统代理。

**TUN 模式**：创建名为 `Prism` 的虚拟网卡接管全部流量，不依赖应用是否支持代理。
- 需要**管理员权限**。未以管理员身份运行时打开 TUN，Prism 会询问是否以管理员身份重新启动。
- Prism 内核首次使用 TUN 时会自动下载官方签名的 `wintun.dll`（校验 SHA-256）。
- Prism 内核的 TUN：
  - 通过 TLS SNI / HTTP Host 识别域名，按域名规则分流；
  - 接管发往 53 端口的 DNS 请求：发给某个 DNS 服务器的查询会原样转发给该服务器（绕过虚拟网卡），发给虚拟网卡自身 DNS 地址的查询转发到 `dns.default-nameserver`（默认 `223.5.5.5`、`119.29.29.29`）；
  - 丢弃 QUIC（UDP 443），浏览器会自动改用 TCP；
  - 其他 UDP 仅在规则为 DIRECT 时直连转发，不经代理；
  - 只接管 IPv4。
- 不要同时开启其他客户端（如 FlClash、Clash Verge）的 TUN，两个虚拟网卡会互相抢路由。
- 按设计，内核退出时虚拟网卡和路由会随进程一起删除，异常退出也不会残留。（Prism 内核的 TUN 尚未在真实网卡上做过端到端测试，遇到问题请改用 Mihomo / sing-box 内核的 TUN。）
- 开着 TUN 时如果以普通权限启动 Prism，内核不会自动启动（日志中会提示需要管理员权限）。请以管理员身份运行，或先关闭 TUN。

一般情况下二选一即可：日常用系统代理，需要接管所有程序时用 TUN。

## 5. 订阅格式

Prism 自动识别三种订阅格式：

| 格式 | 识别方式 | 说明 |
|---|---|---|
| Clash / mihomo YAML | 含 `proxies:` / `proxy-groups:` / `proxy-providers:` | 见第 6 节 |
| sing-box JSON | 以 `{` 开头且含 `outbounds` / `inbounds` | 见第 8 节 |
| 分享链接 | 其他情况；可以是明文或整体 Base64 编码，每行一个链接 | 见第 7 节 |

只包含节点的订阅（分享链接）会自动生成分组和规则：

```yaml
proxy-groups:
  - name: 节点选择
    type: select
    proxies: [自动选择, <所有节点>, DIRECT]
  - name: 自动选择
    type: url-test
    proxies: [<所有节点>]
rules:
  - GEOSITE,private,DIRECT
  - GEOIP,private,DIRECT,no-resolve
  - GEOSITE,cn,DIRECT
  - GEOIP,CN,DIRECT
  - MATCH,节点选择
```

导入时如果订阅无法解析或没有可用节点，会直接报错，不会保存。

## 6. 配置语法（Clash YAML）

Prism 内核读取的是 Clash / mihomo 格式的 YAML。使用 Mihomo 内核时，Clash YAML 订阅会**原样**交给 mihomo（只覆盖下面「由 Prism 管理的字段」），mihomo 的全部语法都可用；本节说明的是 **Prism 内核**支持的部分。

### 6.1 完整示例

```yaml
mixed-port: 7890
allow-lan: false
mode: rule
log-level: info
unified-delay: true

proxies:
  - name: "东京"
    type: anytls
    server: jp.example.com
    port: 443
    password: "your-password"
    sni: jp.example.com
    udp: true

  - name: "香港 SS"
    type: ss
    server: 1.2.3.4
    port: 8388
    cipher: aes-256-gcm
    password: "your-password"

proxy-groups:
  - name: PROXY
    type: select
    proxies: [自动选择, 东京, 香港 SS, DIRECT]
  - name: 自动选择
    type: url-test
    proxies: [东京, 香港 SS]
    url: https://www.gstatic.com/generate_204
    interval: 300
    tolerance: 50

rules:
  - DOMAIN-SUFFIX,ads.example.com,REJECT
  - DOMAIN-KEYWORD,google,PROXY
  - GEOSITE,private,DIRECT
  - GEOIP,private,DIRECT,no-resolve
  - GEOSITE,cn,DIRECT
  - GEOIP,CN,DIRECT
  - MATCH,PROXY
```

### 6.2 通用字段

| 字段 | 说明 |
|---|---|
| `mixed-port` | HTTP + SOCKS5 混合端口。没有时依次取 `port`、`socks-port` |
| `allow-lan` | 是否监听所有网卡 |
| `bind-address` | `allow-lan` 时监听的地址，`*` 表示全部 |
| `mode` | `rule` / `global` / `direct` |
| `log-level` | `debug` / `info` / `warning` / `error` / `silent` |
| `external-controller` | Clash API 监听地址，如 `127.0.0.1:9090` |
| `secret` | Clash API 密钥 |
| `unified-delay` | 测速只计第二次请求 |
| `tun.enable` | 启用 TUN（需管理员权限） |
| `tun.auto-route` | 自动添加路由，默认 `true` |
| `dns.default-nameserver` | TUN 模式下，发给虚拟网卡自身 DNS 地址的查询转发到这里（只取纯 IP 地址），默认 `223.5.5.5`、`119.29.29.29` |

**由 Prism 管理的字段**：在客户端中运行时，`mixed-port`、`external-controller`、`secret`、`mode`、`allow-lan`、`log-level`、`unified-delay`、`tun` 会被设置页的值覆盖；`port`、`socks-port`、`redir-port`、`tproxy-port`、`external-ui` 会被删除。在订阅里写这些字段不会生效，请在设置页修改。

### 6.3 proxies（节点）

每个节点都需要 `name`、`type`、`server`、`port`。`name` 必须唯一。

**AnyTLS**（`type: anytls`）

| 字段 | 说明 |
|---|---|
| `password` | 密码 |
| `sni` | TLS 服务器名，默认同 `server` |
| `skip-cert-verify` | 跳过证书校验（自签证书时使用），默认 `false` |
| `alpn` | ALPN 列表，如 `[h2, http/1.1]` |
| `client-fingerprint` | TLS 指纹（Prism 内核忽略，Mihomo / sing-box 使用） |

Prism 内核会复用与服务端的 TLS 会话：新连接不需要重新握手，空闲 30 秒后关闭。

**Shadowsocks**（`type: ss`）

| 字段 | 说明 |
|---|---|
| `cipher` | Prism 内核支持 `aes-128-gcm`、`aes-256-gcm`、`chacha20-ietf-poly1305` |
| `password` | 密码 |

带 `plugin`（obfs / v2ray-plugin 等）的节点会被跳过，只能在 Mihomo 内核下使用；`2022-blake3-*` 系列加密 Prism 内核不支持（节点被跳过），Mihomo / sing-box 内核可用。

**Trojan**（`type: trojan`）

| 字段 | 说明 |
|---|---|
| `password` | 密码 |
| `sni`、`skip-cert-verify`、`alpn` | 同 AnyTLS |
| `network` | Prism 内核只支持默认的 TCP；`ws` / `grpc` 需切换到 Mihomo / sing-box |

**VLESS**（`type: vless`）

| 字段 | 说明 |
|---|---|
| `uuid` | 用户 UUID |
| `tls` | 是否启用 TLS |
| `servername` | TLS 服务器名 |
| `skip-cert-verify`、`alpn` | 同上 |
| `flow`、`reality-opts`、`network: ws/grpc` | Prism 内核不支持（含这些字段的节点会被跳过） |

**SOCKS5**（`type: socks5`）、**HTTP**（`type: http`）

| 字段 | 说明 |
|---|---|
| `username`、`password` | 认证信息，可省略 |
| `tls` | 通过 TLS 连接上游代理 |
| `sni`、`skip-cert-verify` | TLS 参数 |

**仅 Mihomo / sing-box 支持的类型**：`vmess`、`hysteria2`，以及上面标注不支持的传输方式。Prism 能识别并在转换时保留它们（`ws-opts`、`grpc-opts`、`reality-opts`、`obfs` 等），但 Prism 内核运行时会跳过这些节点。

### 6.4 proxy-groups（代理组）

```yaml
proxy-groups:
  - name: 组名
    type: select | url-test | fallback | load-balance
    proxies: [节点或组名, ..., DIRECT, REJECT]
    url: https://www.gstatic.com/generate_204   # 健康检查地址（自动类型使用）
    interval: 300                               # 检查间隔（秒），最小 10
    tolerance: 50                               # url-test 切换阈值（毫秒）
```

| 类型 | 行为 |
|---|---|
| `select` | 手动选择，默认第一个 |
| `url-test` | 每隔 `interval` 秒测速，当前节点比最快节点慢超过 `tolerance` 毫秒时切换到最快的 |
| `fallback` | 使用列表中第一个可用（最近一次测速未失败）的节点 |
| `load-balance` | 在可用节点间分配；同一个目标主机始终走同一个节点 |

- 组可以嵌套组（`proxies` 里写其他组的名字）。
- 内置的 `DIRECT`（直连）、`REJECT`（拒绝）可直接使用；`GLOBAL` 组由内核自动生成，包含所有组和节点，供全局模式使用。
- 引用了不存在名字的成员会被删除并记录警告；成员全部无效的组会变成只含 `DIRECT`。
- 不支持 `use`（proxy-providers）。

### 6.5 rules（规则）

格式：

```
类型,内容,目标[,no-resolve]
MATCH,目标
```

规则**从上到下**匹配，命中第一条即停止；全部不命中时直连。`目标` 可以是节点名、组名、`DIRECT` 或 `REJECT`；目标不存在的规则会被删除并记录警告。

| 类型 | 匹配 | 示例 |
|---|---|---|
| `DOMAIN` | 完整域名 | `DOMAIN,www.google.com,PROXY` |
| `DOMAIN-SUFFIX` | 域名及其所有子域名 | `DOMAIN-SUFFIX,google.com,PROXY`（匹配 `google.com`、`a.google.com`，不匹配 `notgoogle.com`） |
| `DOMAIN-KEYWORD` | 域名包含关键字 | `DOMAIN-KEYWORD,youtube,PROXY` |
| `DOMAIN-REGEX` | 域名匹配正则（域名已转为小写） | `DOMAIN-REGEX,^ads?\.,REJECT` |
| `GEOSITE` | `geosite.dat` 中的域名分类 | `GEOSITE,cn,DIRECT`、`GEOSITE,google@cn,DIRECT` |
| `GEOIP` | `geoip.dat` 中的 IP 分类（国家代码不区分大小写） | `GEOIP,CN,DIRECT` |
| `GEOIP,private` | 私有地址、本机、链路本地、CGNAT 等（`lan` 同义，不需要数据文件） | `GEOIP,private,DIRECT,no-resolve` |
| `IP-CIDR` / `IP-CIDR6` | 目标 IP 网段 | `IP-CIDR,10.0.0.0/8,DIRECT,no-resolve` |
| `SRC-IP-CIDR` | 来源 IP 网段（局域网设备使用时有用） | `SRC-IP-CIDR,192.168.1.50/32,DIRECT` |
| `DST-PORT` | 目标端口，可用 `-` 表示范围、`/` 分隔多个 | `DST-PORT,22,DIRECT`、`DST-PORT,8000-9000,PROXY`、`DST-PORT,80/443,PROXY` |
| `SRC-PORT` | 来源端口 | `SRC-PORT,50000-60000,DIRECT` |
| `IN-TYPE` | 入站类型：`HTTP`（普通 HTTP 代理）、`HTTPS`（CONNECT）、`SOCKS5`、`TUN` | `IN-TYPE,TUN,PROXY` |
| `MATCH`（或 `FINAL`） | 匹配所有，放在最后 | `MATCH,PROXY` |

**no-resolve**：`GEOIP`、`IP-CIDR` 类规则遇到域名目标时，默认会先解析出 IP 再匹配（超时 3 秒）。加上 `no-resolve` 则域名目标直接跳过这条规则，避免为了匹配规则而做 DNS 查询。一般给 IP 类规则都加上 `no-resolve`，只在确实需要按 IP 分流的规则上去掉。

**GEOSITE / GEOIP 数据**：Prism 内核读取 v2ray 格式的 `geosite.dat`、`geoip.dat`（MetaCubeX/meta-rules-dat），首次使用时自动下载，设置页可手动更新。数据文件缺失时这两类规则会被整体跳过；数据文件中不存在的分类，对应规则也会被忽略。

**Prism 内核不支持的规则**（会被跳过并在日志中警告，Mihomo 内核可用）：`RULE-SET`、`PROCESS-NAME`、`PROCESS-PATH`、`AND` / `OR` / `NOT`、`NETWORK`、`GEOIP` 之外的 `SRC-GEOIP` 等。

## 7. 分享链接语法

分享链接订阅每行一个链接，`#` 后面是节点名（需 URL 编码，如中文）。整段内容也可以是 Base64 编码。重名节点会自动加序号。

**AnyTLS**

```
anytls://密码@服务器:端口/?sni=域名&insecure=1#节点名
```

| 参数 | 说明 |
|---|---|
| `sni`（或 `peer`） | TLS 服务器名 |
| `insecure=1` / `allowInsecure=1` | 跳过证书校验 |
| `alpn` | 逗号分隔 |
| `fp` | TLS 指纹 |

密码中的特殊字符需 URL 编码（`@` 写作 `%40`）。

**Shadowsocks**（SIP002，以及旧式整体 Base64）

```
ss://BASE64(加密方式:密码)@服务器:端口#节点名
ss://BASE64(加密方式:密码@服务器:端口)#节点名
```

带 `plugin=` 的链接不支持。

**Trojan**

```
trojan://密码@服务器:端口?sni=域名&allowInsecure=1&type=ws&path=/路径&host=域名#节点名
```

**VLESS**

```
vless://UUID@服务器:端口?security=tls&sni=域名&type=tcp#节点名
vless://UUID@服务器:端口?security=reality&sni=域名&pbk=公钥&sid=短ID&fp=chrome&flow=xtls-rprx-vision#节点名
```

| 参数 | 说明 |
|---|---|
| `security` | `none` / `tls` / `reality` |
| `type` | `tcp`（默认）/ `ws` / `grpc` |
| `path`、`host` | WebSocket 路径和 Host |
| `serviceName` | gRPC 服务名 |
| `flow`、`pbk`、`sid` | Vision / Reality 参数 |

**VMess**（v2rayN 格式）

```
vmess://BASE64({"ps":"节点名","add":"服务器","port":"443","id":"UUID","aid":"0","scy":"auto","net":"ws","host":"域名","path":"/路径","tls":"tls","sni":"域名"})
```

**Hysteria2**

```
hysteria2://密码@服务器:端口?sni=域名&obfs=salamander&obfs-password=混淆密码&insecure=1#节点名
hy2://...（同上）
```

**SOCKS5**

```
socks5://用户名:密码@服务器:端口#节点名
```

VLESS 的 Reality / Vision / WS / gRPC、Trojan 的 WS / gRPC、VMess、Hysteria2 只能在 Mihomo / sing-box 内核下使用。

## 8. sing-box JSON 与格式转换

### 8.1 使用 sing-box 内核时

- **sing-box JSON 订阅**原样使用，但 `inbounds` 会被替换为 Prism 的混合入站（开启 TUN 时再加 TUN 入站），并强制设置 Clash API 和日志级别。
- **Clash YAML / 分享链接订阅**会转换为 sing-box 配置（要求 sing-box 1.12 及以上）：

| Clash 规则 | sing-box 路由规则 |
|---|---|
| `DOMAIN` / `DOMAIN-SUFFIX` / `DOMAIN-KEYWORD` / `DOMAIN-REGEX` | `domain` / `domain_suffix` / `domain_keyword` / `domain_regex` |
| `IP-CIDR` / `IP-CIDR6` | `ip_cidr` |
| `SRC-IP-CIDR` | `source_ip_cidr` |
| `DST-PORT` | `port` / `port_range` |
| `PROCESS-NAME` | `process_name` |
| `GEOIP,private` | `ip_is_private` |
| `GEOIP,xx` | 规则集 `geoip-xx` |
| `GEOSITE,xx` | 规则集 `geosite-xx` |
| 目标 `REJECT` | `action: reject` |
| `MATCH` | `route.final` |

  其他类型的规则在转换时会被丢弃。代理组 `select` 转为 `selector`，其余自动类型都转为 `urltest`。

- **规则集**：GEOSITE / GEOIP 对应的 `.srs` 规则集由 Prism 下载到 `runtime/rule-set/`（优先经运行中的代理，其次 jsdelivr 镜像，最后 GitHub），再以本地规则集交给 sing-box。这样 sing-box 启动时不依赖网络，节点不可用也能启动。超过 7 天的规则集会在后台刷新；实在下载不到的规则集会连同使用它的规则一起跳过，并在日志中提示。原生 sing-box 订阅里的远程规则集也按同样方式处理。
- 全局 / 直连模式通过 sing-box 的 `clash_mode`（`Global` / `Direct`）实现。

### 8.2 sing-box JSON 订阅用于 Prism / Mihomo 内核时

sing-box 配置会转换为 Clash YAML：

- 出站：`anytls`、`shadowsocks`、`trojan`、`vless`、`vmess`、`hysteria2`、`socks`、`http` 转为节点；`selector` / `urltest` 转为代理组；`direct` 视为 `DIRECT`，`block` 视为 `REJECT`。
- 路由规则：`domain`、`domain_suffix`、`domain_keyword`、`domain_regex`、`ip_cidr`、`port`、`process_name`、`ip_is_private`，以及名为 `geosite-xx` / `geoip-xx` 的规则集，转为对应的 Clash 规则；`route.final` 转为 `MATCH`。
- 含 `clash_mode`、`protocol`、`inbound` 条件的规则会跳过。
- 一条 sing-box 规则里的多个条件会拆成多条 Clash 规则（相当于「或」），属于近似转换；需要精确语义时请直接使用 sing-box 内核。

在设置页点 **查看生成的内核配置** 可以看到转换结果。

## 9. prism-core 命令行

`prism-core.exe` 位于安装目录，也可在 Releases 单独下载。参数与 mihomo 一致：

```
prism-core -f config.yaml -d 目录
```

| 参数 | 说明 |
|---|---|
| `-f, --config <文件>` | 配置文件：Clash YAML、sing-box JSON 或分享链接列表 |
| `-d, --dir <目录>` | 工作目录，`geosite.dat`、`geoip.dat`、`wintun.dll` 放在这里；默认为配置文件所在目录 |
| `-t, --test` | 只检查配置并输出警告，不启动 |
| `--ext-ctl <地址>` | 覆盖 `external-controller` |
| `--secret <密钥>` | 覆盖 API 密钥 |
| `--tun` | 启用 TUN（需管理员权限） |
| `--convert clash\|singbox` | 把配置转换为 Clash YAML 或 sing-box JSON 并输出，然后退出 |
| `--rule-set-dir <目录>` | 配合 `--convert singbox`：使用目录中的 `<tag>.srs` 作为本地规则集 |

日志级别也可以用环境变量覆盖，例如 `set RUST_LOG=debug`（会显示每次测速失败的原因等调试信息）。

内核提供 Clash 兼容 API（需在请求头带 `Authorization: Bearer <secret>`）：

| 接口 | 说明 |
|---|---|
| `GET /version` | 版本 |
| `GET /configs`、`PATCH /configs` | 读取配置；`{"mode":"global"}` 切换模式 |
| `GET /proxies`、`GET /proxies/{名称}` | 节点和组 |
| `PUT /proxies/{组名}` | `{"name":"节点名"}` 选择节点 |
| `GET /proxies/{名称}/delay?url=&timeout=` | 测速 |
| `GET /group/{组名}/delay?url=&timeout=` | 整组测速 |
| `GET /connections`、`DELETE /connections`、`DELETE /connections/{id}` | 连接列表与关闭 |
| `GET /rules` | 已加载的规则 |
| `GET /traffic` | 每秒一行的实时速度 |

## 10. 数据目录与文件

数据目录：`%APPDATA%\dev.prism.client\`

| 路径 | 内容 |
|---|---|
| `settings.json` | 设置 |
| `profiles.json`、`profiles\` | 订阅列表和订阅原文 |
| `runtime\config.yaml` / `config.json` | 为当前内核生成的配置 |
| `runtime\geosite.dat`、`geoip.dat` | Prism / Mihomo 使用的规则数据 |
| `runtime\rule-set\` | sing-box 使用的规则集 |
| `runtime\wintun.dll` | Prism 内核 TUN 所需的驱动库 |
| `cores\` | 下载的 `mihomo.exe`、`sing-box.exe` |

想恢复初始状态：退出 Prism 后删除整个数据目录即可。

## 11. 常见问题

**内核启动后立即退出 / API 无响应**
多半是端口被占用（例如其他 Clash 客户端也在用相同端口）。查看 **日志** 页的报错，在设置页换一个代理端口或 API 端口。

**开了系统代理，某些程序还是不走代理**
这些程序不读取系统代理设置，请改用 TUN 模式。

**Prism 内核和 Mihomo 的测速数字不一样**
先确认设置页的 **统一延迟** 两边一致：开启时只计第二次请求，数字会明显小于不开启时。测速地址为 https 时，不开启统一延迟的结果包含一次 TLS 握手。

**导入订阅提示「订阅中没有可用节点」或 YAML 错误**
订阅内容无法解析。用浏览器打开订阅链接看看返回的内容；若是 YAML，错误信息会指出具体问题（例如重复的键）。

**日志里出现「不支持」的警告**
该节点类型或规则 Prism 内核不支持，已被跳过，其余部分正常工作。需要这些功能时切换到 Mihomo 或 sing-box 内核。

**GEOSITE / GEOIP 规则不生效**
数据文件可能下载失败。在设置页点 **更新 GeoSite / GeoIP** 重试，或查看日志中的下载错误。

**sing-box 日志提示「规则集 xxx 未能下载」**
对应规则已被跳过，其余规则正常。网络恢复后在设置页点 **更新 GeoSite / GeoIP** 重新下载，再重启内核。

**TUN 打不开**
确认以管理员身份运行，并关闭其他客户端的 TUN 模式。Prism 内核需要 `wintun.dll`，首次使用会自动下载；下载失败时可手动从 [wintun.net](https://www.wintun.net/) 下载，把 `wintun/bin/amd64/wintun.dll` 放到数据目录的 `runtime\` 下。

**内核下载失败**
GitHub 在部分网络下访问较慢。先用 Prism 内核连上代理，再到设置页下载 Mihomo / sing-box，下载会自动经过代理。
