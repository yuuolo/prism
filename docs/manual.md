# Prism 使用手册（Windows）

适用版本：v0.2.7（v0.2.7 新增真实 IP 页面（内网域名不走假 IP、指定 DNS、直连）；v0.2.6 新增 AnyTLS 填充和失效会话检测；v0.2.5 新增链式代理可选代理组；v0.2.4 新增自定义 GeoSite / GeoIP 地址；v0.2.3 新增链式代理、出口异常时拒绝、断网保护；v0.2.0 新增 Shadowsocks 2022、xhttp / HTTP/2 传输、mrs 规则集、DNS fallback / DoQ / DoH3、TUN 接管 IPv6、规则测试、节点质量、带宽测速、连接地图、应用流量、局域网共享、托盘网速、命令面板；v0.1.9 新增 macOS 版；v0.1.8 新增域名分流；v0.1.7 新增合并订阅；v0.1.6 新增 VMess、WebSocket / HTTPUpgrade / gRPC 传输、TUIC、WireGuard、Vision 的 UDP、链式代理、组合规则、按订阅覆写、连接导出）

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

1. 从 [Releases](https://github.com/yuuolo/prism/releases) 下载并安装：
   - **Windows**：`Prism_<版本>_x64-setup.exe`。安装程序未做代码签名，SmartScreen 提示时选择「更多信息 → 仍要运行」。
   - **macOS**（10.15 及以上）：Apple 芯片（M1–M5）下载 `Prism_<版本>_aarch64.dmg`，Intel 芯片下载 `Prism_<版本>_x64.dmg`（「关于本机」可查看芯片）。打开 dmg，把 Prism 拖进「应用程序」。应用没有 Apple 签名，第一次打开要在 Finder 里右键 Prism → **打开** → 再点 **打开**；如果提示「已损坏，无法打开」，在终端执行 `xattr -cr /Applications/Prism.app` 后再打开。
2. 打开 Prism，进入 **订阅** 页，粘贴订阅链接，点 **导入**。第一个导入的订阅会自动启用。
3. 回到 **概览** 页，点 **启动内核**。
4. 在 **流量接管** 卡片里打开 **系统代理**（或 **TUN 模式**，见第 4 节）。
5. 在 **代理** 页选择节点，点 **测速** 查看延迟。

之后每次打开 Prism，只要有已启用的订阅，内核会自动启动，并恢复上次的系统代理状态和节点选择。想开机就连上代理，在设置页打开 **开机自启**（见 2 节「开机自启与一键导入」）。

> 默认代理端口是 **7899**（不是 Clash 常用的 7890，以免与已安装的其他客户端冲突），可在设置页修改。

关闭窗口不会退出 Prism，它会缩到托盘。真正退出请用托盘菜单的 **退出**；退出时会自动关闭系统代理并停止内核。Prism 只会运行一个：再次打开它（或点击导入链接）只会把已运行的 Prism 窗口调出来。

## 2. 界面说明

### 概览
- **实时速度 / 流量统计**：上传、下载速度曲线，总流量，活动连接数，代理端口。
- **内核**：在 Prism 内核、Mihomo、sing-box 之间切换（见第 3 节）。未安装的内核会显示「未安装 · 去设置下载」。
- **流量接管**：系统代理、TUN 模式开关（见第 4 节），以及 **局域网共享**（见下）。
- **代理模式**：
  - **规则**：按订阅里的规则分流（通常国内直连、其余走代理）。
  - **全局**：全部流量走 `GLOBAL` 组选中的节点。
  - **直连**：全部直连，不经过代理。
- **当前订阅**：启用中的订阅名称和前几个代理组当前选中的节点。
- **最近 30 天流量**：每天经过内核的上传（紫色）和下载（蓝色）流量，鼠标移上去显示当天数字，右上角是本月合计。数据保存在数据目录的 `traffic.json`，只在 Prism 运行时统计（三种内核都支持），保留约 400 天。
- **应用流量**：今天 / 7 天 / 30 天里每个程序用了多少流量（上传 / 下载），以及主要走了哪些出口（如 `东京 77% · DIRECT 15%`）。按连接统计，内核需要能识别程序（Windows / macOS 上三种内核都可以）；识别不到程序的连接记为「未知程序」。数据保存在 `app_traffic.json`，保留 90 天。
- **局域网共享**：点 **二维码** 打开，开关即「允许局域网连接」。窗口列出本机的局域网地址（有多个网卡时可选）和代理端口，并显示 `http://地址:端口` 的二维码；手机在 Wi-Fi 设置里把代理设为「手动」，填入地址和端口即可（HTTP 和 SOCKS5 都在这个端口）。共享没有密码，只在可信的网络中开启；手机连不上时，检查防火墙是否允许内核在「专用网络」上接收连接。

### 代理
- 按代理组显示节点；组的顺序与订阅中一致。
- 点击节点卡片即可选择（只有 `select` 类型的组可以手动选择；`url-test` / `fallback` / `load-balance` 是自动的）。
- 点节点上的延迟数字测单个节点，点组标题右侧的 **测速** 测整组；页面右上角的 **全部测速** 一次测完所有组里的节点（每个节点只测一次，按钮上显示进度）。
- **搜索**：在搜索框输入节点或代理组名称的一部分，只显示匹配的节点（组名匹配时显示整组），按 Esc 清空。
- **排序**：默认（订阅中的顺序）、延迟（快的在前，超时和未测速的排在最后）、名称。选择会被记住。
- 延迟颜色：绿色 < 300 ms，黄色 < 800 ms，橙色更慢，红色「超时」表示测试失败。
- 节点选择会按订阅分别记住，重启内核或 Prism 后自动恢复。
- **解锁检测**：检测 Netflix、YouTube Premium、Prime Video、TikTok、ChatGPT、Claude、Gemini 能否使用及所在地区。检测经当前的规则和所选节点进行（和浏览器开着系统代理访问时一样），需要先启动内核；换节点后可 **重新检测**。结果为「可用 / 部分可用（如 Netflix 仅自制剧）/ 不可用 / 无法判断（连接失败或遇到验证页）」。
- **节点质量**：每个节点最近 1 / 6 / 24 小时的延迟曲线（红色竖线是失败），以及平均延迟、抖动和失败比例，按稳定性排序（延迟 + 2 × 抖动 + 失败惩罚）。数据来自自动测速组（url-test 等）的定时测试、手动测速，以及 Prism 每 5 分钟对正在使用的节点的测试；保存在 `quality.json`，保留 24 小时。对可手动选择的组，可以一键 **选用最稳定的** 节点。
- **带宽测速**：逐个经节点下载测试文件（Cloudflare 或 CacheFly），测出实际下载速度和首包时间，结果按速度排序。每个节点测 5 / 8 / 15 秒，会消耗相应的流量。测速在 Prism 里直接用 Prism 内核的协议实现进行，所以三种内核下都能用（Prism 内核不支持的节点协议会显示「失败」）；开着 TUN 时测速连接会绕过虚拟网卡。
- 订阅含 `proxy-providers` 时，顶部的 **代理集合** 卡片列出每个集合的类型、节点数和更新时间，可单独 **测速** 或 **更新**（HTTP 类型）。更新后内核热重载，已选节点保持不变。

### 订阅
- **导入**：粘贴订阅链接后导入。Prism 以 `clash.meta` 作为 User-Agent 下载，所以多数机场会返回 Clash 格式。若直连下载失败且内核正在运行，会自动经代理重试。也可以在机场面板点「一键导入」（见下面的「开机自启与一键导入」）。
- **新建本地配置**：直接粘贴配置内容（Clash YAML、sing-box JSON 或分享链接）。
- 卡片上显示节点数、格式、更新时间和自动更新间隔；机场返回 `subscription-userinfo` 时还会显示已用流量 / 总流量和到期日。
- 点击卡片即切换到该订阅（内核运行中会自动重启生效）。
- **更新**：重新下载远程订阅；**全部更新**：更新所有远程订阅。正在使用的订阅更新后：Prism / Mihomo 内核**热重载**（不断开现有连接，节点选择保持不变），sing-box 内核会重启。新内容有问题、内核拒绝加载时，内核继续使用旧配置并提示错误。
- **自动更新**：远程订阅按间隔自动重新下载，默认每 24 小时；机场在响应头 `profile-update-interval` 中给出间隔时，导入时采用机场的值。在 **编辑** 中可改为任意分钟数，填 `0` 关闭。自动更新的结果记录在日志中；失败时 30 分钟后重试。
- **编辑**：修改名称、自动更新间隔和内容。注意远程订阅更新时会覆盖对内容的手动修改。
- **删除**：内核运行时不能删除正在使用的订阅，请先停止内核或切换到别的订阅。
- **覆写**：对所有订阅生效的修改，见下面的「覆写」。
- **合并**：其他订阅卡片上的 **合并** 开关打开后，这个订阅的节点会加入当前订阅，让同一套规则有更多节点可用：
  - 规则、DNS 等仍然用当前订阅的；合并的只是节点。
  - 节点会加入当前订阅中列有节点的代理组（如「节点选择」「自动选择」），遵守组的 `filter` / `exclude-filter`；只包含其他组或 DIRECT 的组（如「国内直连」）不变。当前订阅的组都不含节点时，加入它的 select / url-test 组。
  - 和已有节点重名时，名字后加 `[订阅名]`。
  - 被合并的订阅照常按自己的间隔自动更新，更新后内核热重载；流量和到期日仍显示在它自己的卡片上。
  - 当前订阅卡片显示「+N 个订阅」，被合并的卡片显示「已合并」。切换到别的订阅时，合并设置保留，对新的当前订阅同样生效（当前订阅自己不会被合并进自己）。
  - 合并在覆写之前进行，所以覆写脚本能看到合并后的节点。三个内核都支持。
  - 被合并订阅里指向它自己代理组的 `dialer-proxy` 会被忽略，并在日志中提示。

### 连接
- 实时列出经过内核的连接：主机、发起连接的**程序**（鼠标悬停显示完整路径）、入站类型和 TCP / UDP、命中的规则、链路（如 `节点选择 → 自动选择 → 东京`）、速度和流量。
- 可按主机 / 程序 / 规则 / 节点过滤，**暂停** 刷新，单独 **关闭** 某个连接或 **全部关闭**。
- **地图**：切换到地图视图，在世界地图上显示连接去向：绿色圆圈是本机（按系统时区推断所在地），菱形是出口节点所在地，蓝点是目标所在地（越大连接越多，有流量时闪动）；连线分为直连（绿）、到节点（紫）和节点到目标（蓝），越粗速度越快。右侧列出目标地区和出口节点的连接数与速度，鼠标移到某个地区上会突出相关连线。位置用本地的 `geoip.dat` 查询，不会联网；内网地址和解析不到的域名计入「位置未知」。过滤条件同样作用于地图。
- 某条连接后面的 **应用** 按钮：直接为这个程序添加应用分流规则（见下面的「应用分流」）。
- **导出**：把当前列表导出为 Excel（`.xlsx`）、CSV、JSON、HTML 或 Markdown 文件。输入了过滤条件时只导出过滤后的连接；想导出某一刻的状态，可先 **暂停**。文件保存到「下载」文件夹（`prism-connections-日期-时间.扩展名`），并在资源管理器中选中。
  - 导出的列：开始时间、主机、目标 IP、端口、来源、程序、程序路径、入站、网络、规则、规则内容、链路、上传 / 下载（字节）、导出时的上传 / 下载速度（字节/秒）。流量和速度在 Excel 和 JSON 中是数字，可以直接排序、求和。
  - CSV 带 UTF-8 BOM，用 Excel 打开中文不会乱码；以 `=`、`+`、`-`、`@` 开头的内容前会加 `'`，防止被当作公式执行。

### 规则
- **规则测试**：输入域名、IP 或网址（如 `https://chatgpt.com/c/1`），可选填程序名（`chrome.exe`）并选 TCP / UDP，显示会命中第几条规则、规则原文，以及最终出口（沿代理组当前的选择一直到具体节点，如 `节点选择 → 自动选择 → 香港 01`）。需要 IP 的规则（`IP-CIDR`、`GEOIP` 等）会先解析域名：内核运行时用内核的 DNS（不会得到 fake-ip），否则用系统解析。测试用的是当前订阅加上应用分流、域名分流规则，与三种内核实际使用的规则相同；不需要内核运行（运行时还能显示组当前选择的节点）。全局 / 直连模式下会提示规则不生效。
- 显示内核实际加载的规则（不受支持而被跳过的规则不会出现在这里，原因见日志）。
- 订阅含 `rule-providers` 时，顶部的 **规则集合** 卡片列出每个规则集的类型、条数和更新时间，HTTP 类型可单独 **更新**。

### 应用分流
指定某个程序的连接走固定出口，不受订阅规则影响。
- 点 **添加应用**，从正在运行的程序列表中选择（可搜索），或手动输入程序名 / 完整路径。
- **匹配方式**：按程序名（如 `Telegram.exe`，不区分大小写，所有同名程序都匹配；只写 `Telegram` 也会自动补上 `.exe`），或按完整路径（只匹配该位置的程序）。
- **出口**：DIRECT（直连）、REJECT（拒绝），或当前订阅中的任一代理组 / 节点。
- 每条规则可单独启用 / 停用、修改出口、编辑或删除；保存后运行中的内核会自动重新加载。
- 规则只在**规则模式**下生效，排在订阅规则之前；全局 / 直连模式不受影响。
- 三种内核都支持（Prism / Mihomo 生成 `PROCESS-NAME` / `PROCESS-PATH` 规则，sing-box 生成 `process_name` / `process_path`）。所选出口在当前订阅中不存在时，该条规则会被跳过并在日志中提示；勾选 **出口异常时拒绝** 后改为拒绝连接（见「域名分流」）。
- 系统代理模式下只对使用系统代理的程序有效；游戏、命令行等不走系统代理的程序，请开启 TUN 模式。

### 真实 IP
让公司内网这类地址拿到真实 IP 并直连。开启 fake-ip 时，内核会给域名返回 `198.18.x.x` 这样的假 IP，内网系统和只有公司 DNS 才能解析的域名容易出问题，在这里添加就能解决。

- 点 **添加规则**，选类型：
  - **域名**：这个域名（可选包含所有子域名）不再返回假 IP，连接直连。**解析用的 DNS** 填公司 DNS（如 `10.1.1.1`、`tcp://10.1.1.1`），留空时用订阅的 DNS。这里的 DNS 总是直接连接，不能写 `#代理`。
  - **IP / 网段**：这个网段的连接直连。按 IP 访问不经过 DNS，不需要其他设置。
- 规则优先于应用分流和域名分流。保存后运行中的内核会自动重新加载，三种内核都支持，换订阅也保留。
- 页面上的 **测试** 显示这台电脑现在把域名解析成什么，并判断是不是假 IP。TUN 模式未开启时，看到的是系统 DNS 的结果。
- 解析设置在任何模式下都生效，直连规则只在**规则模式**下起作用。
- 订阅是 sing-box 原始配置时，要有 `dns` 段才能指定 DNS。
- TUN 模式下内核的连接绑定物理网卡，所以只有连 VPN 才能访问的内网可能仍然连不上。

### 链式代理
让流量依次经过多个节点或代理组（如 `中转 → 落地`），网站看到的是最后一跳的 IP。可以用来固定出口 IP，或者在直连落地节点太慢时先经过一个线路好的中转。

- 点 **添加链路**，起一个名字，按顺序选至少两个不同的节点或代理组。来自 proxy-providers 的节点不能直接选；成员来自 proxy-providers 的代理组只能作为第一跳。
- 链路是单独的一类，不属于代理组：在应用分流、域名分流的出口列表里，以及代理页上，都单独列在「链式代理」下。保存后可以作为这两种分流的出口，也可以在全局模式下选择。运行中的内核会自动重新加载。
- 第一跳选代理组时，三种内核都跟随这个组当前的选择。后面的跳选代理组时：Prism 内核跟随原组当前的选择；mihomo、sing-box 内核会为这条链路生成这个组的副本（名字形如 `链路名 · 节点名`），手动选择的组副本在代理页的链路下面单独选择，不跟随原组。
- 三种内核都支持。订阅是 sing-box 原始配置时不支持（日志中会提示）。
- 链路用到的节点或代理组从当前订阅中消失时，这条链路不生效并在日志中提示；链路页会标出缺少的项目。
- 每多一跳，延迟通常更高，任何一个节点出问题都会让整条链路中断。走 UDP 的节点（TUIC、Hysteria2、WireGuard）要求前一跳支持 UDP。

### 域名分流
指定网站或 IP 走固定出口，不受订阅规则影响，换订阅也保留。
- 点 **添加规则**，选匹配方式，填内容（添加时可以每行一个，一次加多条），选出口。也可以在 **连接** 页某条连接后面点 **域名**（没有域名时为 **IP**），直接为它添加规则。

| 匹配方式 | 对应规则 | 示例 |
|---|---|---|
| 域名及子域名 | `DOMAIN-SUFFIX` | `openai.com`（也匹配 `chat.openai.com`）；可以直接粘贴网址，会自动取出域名；开头的 `*.`、`+.` 会去掉 |
| 完整域名 | `DOMAIN` | `www.google.com` |
| 域名关键字 | `DOMAIN-KEYWORD` | `youtube` |
| IP / 网段 | `IP-CIDR` | `8.8.8.8`（自动写成 `8.8.8.8/32`）、`192.168.1.0/24`、IPv6 地址 |
| 域名分类 | `GEOSITE` | `netflix`、`openai`、`category-ads-all`（分类名见 MetaCubeX meta-rules-dat） |
| IP 地区 | `GEOIP` | `JP`、`telegram` |

- **出口**：DIRECT（直连）、REJECT（拒绝），或当前订阅中的任一代理组 / 节点。所选出口在当前订阅中不存在时，该条规则会被跳过并在日志中提示。
- **出口异常时拒绝**（应用分流、域名分流都有）：节点连不上，连接直接失败，不会直连，也不会改走其他规则。出口从订阅中消失时（例如订阅更新后节点改名），这条规则也不会被跳过，而是拒绝连接。有几点要注意：
  - 出口是代理组时，组内其他节点或 DIRECT 仍可能接管，要完全锁定请选具体节点。
  - 只在规则模式下生效；切到全局或直连模式后，这些连接不再受规则约束。
  - 域名分类 / IP 地区规则需要的数据（geosite、geoip 或 sing-box 规则集）下载失败时，该规则匹配不到连接，不受这个选项保护。
  - 只管经过内核的连接。不走系统代理的程序要开 TUN 模式；内核退出后规则也随之失效，详见下方「客户端或内核异常退出时」。
- 规则只在**规则模式**下生效，顺序为：应用分流 → 域名分流（从上到下，第一条匹配的生效，可用 **↑** 调整顺序）→ 订阅规则。
- IP 类规则（IP / 网段、IP 地区）默认只匹配直接按 IP 访问的连接（规则带 `no-resolve`），不会为了匹配而解析域名。勾选「也匹配解析到这些 IP 的域名」后，访问域名时会先解析再匹配，会慢一些；sing-box 内核不支持这个选项（按不勾选处理）。
- 每条规则可单独启用 / 停用、修改出口、编辑或删除；保存后运行中的内核会自动重新加载。三种内核都支持；sing-box 内核下，域名分类 / IP 地区规则使用的规则集由 Prism 自动下载。

### 日志
- 内核的输出和 Prism 自身的提示（以 `[prism]` 开头，紫色显示），警告为黄色，错误为红色。
- 每条经过代理的连接会记录一行（括号中是发起连接的程序），例如：
  `[TCP] 127.0.0.1:51234(chrome.exe) --> www.google.com:443 match DomainSuffix(google.com) using 节点选择[东京]`

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
| GeoSite / GeoIP 地址 | 自定义下载地址。留空使用内置地址；填写后优先从这里下载，失败时仍会尝试内置地址。`geosite.dat` / `geoip.dat` 填完整网址，sing-box 规则集地址用 `{}` 代表分类名（如 `https://example.com/geosite-{}.srs`）。订阅自带地址的规则集不受影响。mihomo 自己下载缺失的数据文件时也用这两个 `.dat` 地址（写入 `geox-url`），但它默认用 `country.mmdb` 判断 IP 地区，这个文件不受这里的地址影响。 |
| 查看生成的内核配置 | 查看 Prism 根据订阅为当前内核生成的实际配置文件 |
| 打开数据目录 | 在资源管理器中打开数据目录（见第 10 节） |
| 主题 | 跟随系统 / 浅色 / 深色 |
| 界面语言 | 中文 / English。英文界面下，内核和后台返回的部分错误提示仍为中文 |
| 快捷键 | 显示 / 隐藏主界面、开关系统代理、开关 TUN、切换规则 / 全局 / 直连模式的全局快捷键（见下） |
| 开机自启 | 登录 Windows 时自动启动 Prism，切换后立即生效（见下） |
| 静默启动 | 开机自启时只在托盘运行，不弹出主界面；需点 **保存** |
| 托盘图标显示网速 / 菜单栏网速 | Windows：托盘图标改为两行数字，上行是上传、下行是下载（每秒字节，如 `1.2M`）；macOS：菜单栏图标旁显示 `↑1.2M ↓3.4M`（macOS 默认开启）。无论是否开启，鼠标悬停在托盘图标上都会显示当前网速；需点 **保存** |
| 接管 clash:// 链接 | 让机场面板的「一键导入 Clash」打开 Prism，切换后立即生效（见下） |
| TUN 服务 | 安装、更新、卸载 TUN 服务（见第 4 节） |
| 备份与同步 | WebDAV 云端备份与恢复、导出 / 导入备份文件（见下） |
| 软件更新 | 当前版本、**检查更新**、**启动时自动检查**开关，以及新版本的说明和 **下载并安装**（见下） |

修改端口、密钥、局域网、日志级别或统一延迟后点 **保存**，运行中的内核会自动重启。

### 快捷键
- **命令面板**：在 Prism 窗口中按 `Ctrl+K`（macOS 为 `⌘K`），或点侧栏的「搜索 / 命令」。可搜索并跳转页面、切换规则 / 全局 / 直连模式、开关 TUN 和系统代理、启动 / 停止内核；内核运行时输入节点名即可直接切换到该节点（只列出可手动选择的组）。用 ↑↓ 选择、Enter 执行、Esc 关闭。
- 在设置页「外观与快捷键」中点击某个动作的输入框，按下组合键（如 `Ctrl+Alt+P`）即可设置，按 Backspace 清除；点 **保存** 后生效。
- 快捷键是全局的，Prism 最小化到托盘时也有效。
- 组合键需包含 Ctrl / Alt / Shift / Win 中的至少一个（F1–F12 可单独使用）。格式不对或已被其他程序占用时，保存后会提示哪一个没有注册成功。

### 覆写
在 **订阅** 页点 **全局覆写**，可以写对所有订阅生效的修改；点某个订阅卡片上的 **覆写**，写只对这个订阅生效的修改（卡片上会显示「已覆写」）。它们在生成内核配置前应用：先全局覆写，再这个订阅自己的覆写，每一份都是先 Merge 后 Script。远程订阅更新后依然有效，三种内核都支持。
- **Merge（YAML）**：
  - `prepend-rules` / `append-rules`：在规则最前 / 最后的 `MATCH` 之前加入规则；
  - `prepend-proxies` / `append-proxies`、`prepend-proxy-groups` / `append-proxy-groups`：加入节点和代理组；
  - 其他键直接覆盖订阅中的同名键，字典逐项合并（例如只写 `dns: {nameserver: [...]}` 只替换 nameserver）。
- **Script（JavaScript）**：定义 `function main(config, profileName) { …; return config; }`。`config` 是订阅转换成的 Clash 配置对象，`profileName` 是订阅名称。脚本不能联网或读写文件，死循环会被中止。
- 保存时会先试运行（全局覆写用当前订阅，订阅自己的覆写用那个订阅），出错（YAML 格式错误、脚本异常、结果无法使用）会提示原因且不保存。影响到当前订阅时，运行中的 Prism / Mihomo 内核热重载，sing-box 内核重启。
- 删除订阅时，它自己的覆写一起删除；备份包含全局覆写和每个订阅的覆写。
- 覆写后，sing-box JSON 订阅会先转换成 Clash 格式再应用。

示例（Merge）：

```yaml
prepend-rules:
  - DOMAIN-SUFFIX,example.com,DIRECT
append-proxies:
  - { name: 我的节点, type: ss, server: 1.2.3.4, port: 8388, cipher: aes-256-gcm, password: pw }
```

### 备份与同步
- **WebDAV**：在设置页填写地址（如坚果云 `https://dav.jianguoyun.com/dav/`）、用户名和密码（坚果云等请使用「应用密码」），点 **保存**、**测试连接**。
  - **立即备份**：把所有订阅和设置打包上传到 WebDAV 的 `prism/` 目录，文件名包含电脑名和时间。
  - **查看云端备份**：列出备份，可 **恢复** 或 **删除**。
- **导出到文件 / 从文件恢复**：备份保存到「下载」文件夹，或从一个备份文件恢复，不需要 WebDAV。
- **备份不包含**：WebDAV 密码、本机的 Mihomo / sing-box 路径。
- **恢复时保留**：本机的内核路径、系统代理和 TUN 开关、WebDAV 登录信息；其余设置和全部订阅（含覆写）换成备份中的，内核按备份重新启动。
- WebDAV 密码用 Windows 账户加密（DPAPI）保存在本机，换电脑或换 Windows 用户后需要重新填写。

### 开机自启与一键导入
- **开机自启**：在当前用户的「启动」项中登记 Prism（任务管理器的「启动应用」里能看到，名为 Prism）。开机后 Prism 自动启动内核并恢复系统代理；开着 **静默启动** 时不弹出窗口，只显示托盘图标。卸载 Prism 时会自动移除这一项。
- 开机自启以普通权限运行。安装了 **TUN 服务**（见第 4 节）时，开机后可以直接使用 TUN；没有安装时，如果上次开着 TUN 模式，开机时 Prism 会先不开 TUN 启动内核（日志中有提示，TUN 开关会关闭）。
- **一键导入链接**：机场 / 订阅面板（包括 sbm）的「一键导入」按钮生成 `clash://install-config?url=<订阅地址>&name=<名称>` 链接。点击后 Prism 会弹出确认框，显示订阅名称（可修改）和完整地址，点 **导入** 才会下载；已有相同地址的订阅时改为 **更新** 它。链接不会在你确认前做任何事。
- **prism:// 链接**：`prism://install-config?url=…&name=…`，格式相同，安装版 Prism 启动时自动登记，始终由 Prism 打开。
- **clash:// 链接**：这个链接类型通常已被其他客户端（FlClash、Clash Verge 等）登记。设置页会显示当前由哪个程序打开；打开 **接管 clash:// 链接** 后由 Prism 打开，关闭开关（或卸载 Prism）时还给原来的程序。若之后其他客户端重新登记了它，Prism 不会再抢回来，开关会显示为关闭。
- 导入链接中的订阅地址只接受 http / https。

> 浏览器第一次打开这类链接时会询问「是否打开 Prism」，确认即可。

### 软件更新
- Prism 启动约 5 秒后会到 GitHub Releases 检查新版本（可在设置页关闭）；发现新版本时侧边栏会出现 **发现新版本** 提示，日志中也有记录。
- 在设置页 **软件更新** 卡片点 **下载并安装**：下载时显示进度；下载完成后会**校验发布签名**，只有用本项目私钥签名、且签名版本与公告版本一致的安装包才会被安装，被篡改的安装包会被拒绝。
- 安装前 Prism 会先停止内核并暂时关闭系统代理，然后运行安装程序（只显示进度条），装完自动重启 Prism，系统代理等设置保持不变。
- 内核运行中时，检查和下载会经过本机代理（GitHub 直连较慢时更快）。
- 自动更新从包含更新功能的版本开始生效；更早安装的 v0.1.0 / v0.1.1 需要手动下载安装一次新版本。

### 托盘菜单
单击托盘图标显示主界面；右键菜单可切换 **系统代理**、**TUN 模式**、**规则 / 全局 / 直连模式**，以及 **退出**。

内核运行时，右键菜单还列出可手动选择的代理组（最多 8 个，全局模式下 `GLOBAL` 排在最前）：每个组是一个子菜单，标题显示当前节点，子菜单里列出成员和最近一次测速的延迟，点一下即切换，效果与在代理页选择相同（会被记住）。自动选择类的组（url-test 等）不出现在托盘中。

## 3. 三个内核怎么选

| | Prism 内核 | Mihomo | sing-box |
|---|---|---|---|
| 来源 | 本项目自研（Rust），随安装包自带 | 设置页下载 | 设置页下载 |
| AnyTLS / Shadowsocks / Trojan / VLESS | ✅ | ✅ | ✅ |
| VLESS Reality / Vision（UDP 经 XUDP）、Hysteria2 | ✅ | ✅ | ✅ |
| TUIC v5、WireGuard | ✅ | ✅ | ✅ |
| 链式代理（`dialer-proxy`、`relay` 组） | ✅ | ✅（`relay` 组转换为 `dialer-proxy`） | ✅（转换为 `detour`） |
| 组合规则 `AND` / `OR` / `NOT`、`SRC-GEOIP` | ✅ | ✅ | ✅ |
| TLS 指纹（`client-fingerprint`） | ✅（Chrome 指纹） | ✅ | ✅ |
| VMess（AEAD） | ✅ | ✅ | ✅ |
| WebSocket / HTTPUpgrade / gRPC 传输 | ✅ | ✅ | ✅ |
| HTTP/2 传输 | ✅ | ✅ | ✅ |
| xhttp 传输（stream-one / stream-up / packet-up） | ✅ | ✅ | ❌ |
| Shadowsocks 2022（`2022-blake3-*`，含多用户 `iPSK:uPSK`） | ✅ | ✅ | ✅ |
| 经代理转发 UDP | ✅（每个目标一条会话） | ✅ | ✅ |
| TUN 模式 | ✅ | ✅ | ✅ |
| DNS 模块 / fake-ip | ✅ | ✅ | ✅ |
| proxy-providers / rule-providers | ✅（规则集 yaml / text / mrs） | ✅ | — |
| DNS `fallback`、DoQ（`quic://`）、DoH3 | ✅ | ✅ | 部分 |
| 应用分流（按程序选择出口） | ✅ | ✅ | ✅ |

- 订阅只用 AnyTLS / Shadowsocks（含 2022）/ Trojan / VLESS（含 Reality / Vision）/ VMess / Hysteria2 / TUIC / WireGuard 节点：**Prism 内核**即可，体积小、启动快，TCP 和 UDP 都可经代理。
- 订阅含 kcp / quic 等其他传输的节点、带插件的 Shadowsocks 节点：切换到 **Mihomo** 或 **sing-box**（xhttp 节点用 Prism 内核或 Mihomo）。
- 同一份订阅三个内核通用，Prism 会自动转换格式（见第 5、8 节）。Prism 内核遇到不支持的节点或规则会跳过，并在日志中给出警告。

## 4. 系统代理与 TUN 模式

**系统代理**：把 Windows 的代理设置指向 `127.0.0.1:<代理端口>`。浏览器和大多数遵守系统代理的程序会生效；游戏、命令行工具、部分应用不会走系统代理。

**TUN 模式**：创建名为 `Prism` 的虚拟网卡接管全部流量，不依赖应用是否支持代理。
- 需要**管理员权限**，两种方式任选：
  - **TUN 服务（推荐）**：在设置页「TUN 服务」点 **安装**（或开启 TUN 时按提示安装），授权一次即可。之后开启 TUN 时，内核由系统服务 `Prism TUN Service` 运行，Prism 本身保持普通权限，开机自启也能直接用 TUN。
  - **以管理员身份运行 Prism**：开启 TUN 时选择「以管理员身份重新启动」。以管理员身份运行时 TUN 不经过服务。
- Prism 内核首次使用 TUN 时会自动下载官方签名的 `wintun.dll`（校验 SHA-256）。
- Prism 内核的 TUN：
  - 通过 TLS SNI / HTTP Host 识别域名，按域名规则分流；
  - 接管发往 53 端口的 DNS 请求：启用了 DNS 模块（见 6.6）时由它应答（包括 fake-ip）；否则发给某个 DNS 服务器的查询原样转发给该服务器（绕过虚拟网卡），发给虚拟网卡自身 DNS 地址的查询转发到 `dns.default-nameserver`（默认 `223.5.5.5`、`119.29.29.29`）；
  - 其他 UDP（包括 QUIC）按规则分流，可经代理转发；
  - IPv4 和 IPv6 都接管：虚拟网卡另有 IPv6 地址 `fdfe:dcba:9876::1`，并加上 `::/1`、`8000::/1` 路由，所以 IPv6 流量也会按规则分流，不会绕过代理；内核自己的 IPv6 连接同样绑定到物理网卡。IPv6 设置失败时（例如系统禁用了 IPv6）TUN 仍以仅 IPv4 的方式运行，日志中会说明。三种内核在 Prism 中开启 TUN 时都会接管 IPv6。
- 在 Prism 里打开 TUN 时，如果订阅没有启用 `dns`，会自动加上 fake-ip 的 DNS 配置（`doh.pub`、`dns.alidns.com`），Prism 和 Mihomo 内核都会使用它。
- sing-box 内核的 TUN：订阅启用了 `dns` 时沿用它的 `nameserver`（包括 `#代理` 写法）、fake-ip 地址段和 `fake-ip-filter`；没有启用时，国内域名用 `223.5.5.5`、其他域名经代理组用 `1.1.1.1` 查询（即使订阅最后一条规则是直连）。
- 节点地址是本机（`127.0.0.1`，例如串联本机的另一个代理程序）时，Prism 内核的 TUN 也能连上（v0.1.3 及更早版本会失败）。
- 不要同时开启其他客户端（如 FlClash、Clash Verge）的 TUN，两个虚拟网卡会互相抢路由。
- 按设计，内核退出时虚拟网卡和路由会随进程一起删除，异常退出也不会残留。
- 开着 TUN 时如果以普通权限启动 Prism 且没有安装 TUN 服务（例如开机自启），内核会先不开 TUN 启动，TUN 开关随之关闭，日志中会说明原因。

**TUN 服务说明**
- 服务以 SYSTEM 身份运行，只做一件事：替 Prism 运行内核。为了安全：
  - 只运行安装时复制到 `C:\Program Files\Prism Service\` 的内核（只有管理员能修改这个目录），不会运行其他程序；
  - 只接受安装它的 Windows 用户、且是安装时登记的那个 `prism.exe` 发来的请求，网页或其他用户无法使用它；
  - 配置和数据文件（GeoSite / GeoIP、sing-box 规则集）以内容形式传给服务，配置中会写文件的路径（代理集合 / 规则集合的 `path`、sing-box 的日志和缓存文件）一律改到服务自己的目录 `C:\ProgramData\Prism Service\run\`。
- Prism 或 Mihomo / sing-box 更新后，服务里的内核副本就旧了：开启 TUN 时 Prism 会提示「更新 TUN 服务」，再授权一次即可（开机自启时不会弹授权，本次先不开 TUN）。
- 卸载：设置页 **卸载**；卸载 Prism 时也会一并卸载服务（会弹出一次管理员授权）。
- 服务运行的内核输出同样显示在「日志」页；服务自身的记录在 `C:\ProgramData\Prism Service\service.log`。

一般情况下二选一即可：日常用系统代理，需要接管所有程序时用 TUN。

### macOS 上的不同之处

- **系统代理**：设置到所有已启用的网络服务（Wi-Fi、以太网等）的 HTTP、HTTPS 和 SOCKS 代理上，退出时关闭。绕过列表沿用设置页的「系统代理绕过」，`<local>` 换成 `*.local`。
- **TUN 必须使用 TUN 服务**（macOS 上不能以管理员身份重启 Prism）。开启 TUN 时按提示安装，系统会要求输入一次 Mac 的登录密码；服务是以 root 运行的系统后台服务（LaunchDaemon `dev.prism.service`），程序放在 `/Library/Application Support/Prism Service/`，只接受当前用户的 Prism 的请求。
- TUN 使用系统的 `utunN` 虚拟网卡，地址 `172.19.0.1`，路由与 Windows 相同（`0.0.0.0/1` + `128.0.0.0/1`），内核自己的连接绑定到原来的网卡（如 `en0`）绕过隧道。
- **DNS**：Mac 的 DNS 服务器通常是路由器，不经过隧道。所以开启 TUN 期间，Prism 会把所有网络服务的 DNS 临时改成 `223.5.5.5`、`119.29.29.29`（经隧道由内核应答），关闭 TUN、停止内核或退出 Prism 时改回原来的设置；原设置备份在数据目录的 `dns-backup.json`，Prism 意外退出时下次启动会自动恢复。
- **应用分流**：程序名就是可执行文件名，没有 `.exe`，例如 `Google Chrome`、`Telegram`；完整路径形如 `/Applications/Telegram.app/Contents/MacOS/Telegram`。
- **开机自启**：写入 `~/Library/LaunchAgents/dev.prism.client.plist`。
- **一键导入**：`prism://` 和 `clash://` 链接都由 Prism 打开（装有其他 Clash 客户端时，`clash://` 可能由 macOS 选定的程序打开），没有「接管 clash:// 链接」开关。
- WebDAV 密码保存在「钥匙串」里。
- **卸载**：先在设置页 **卸载** TUN 服务（如已安装），再把 Prism 从「应用程序」拖到废纸篓；数据在 `~/Library/Application Support/dev.prism.client/`。
- **软件更新**：与 Windows 相同，在设置页检查并安装，下载后校验签名，安装完成后 Prism 自动重新打开。

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
| `find-process-mode` | 何时识别连接所属程序：`always`（每条连接，「连接」页显示程序）、`strict`（默认，只在用到进程规则时）、`off`。在 Prism 中运行时固定为 `always` |
| `tun.enable` | 启用 TUN（需管理员权限） |
| `tun.auto-route` | 自动添加路由，默认 `true` |
| `dns` | DNS 模块，见 6.6 |
| `hosts` | 静态解析，见 6.6 |
| `proxy-providers`、`rule-providers` | 见 6.7、6.8 |

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
| `client-fingerprint` | TLS 指纹。填写任意值（如 `chrome`）时，Prism 内核的 TLS 握手与 Chrome 浏览器一致（JA4 `t13d1516h2_8daaf6152771_d8a2da3f94cd`，含 X25519MLKEM768 后量子密钥交换）；服务器只支持 TLS 1.2 时自动改用普通握手。`safari`、`firefox` 等也按 Chrome 处理 |

Prism 内核会复用与服务端的 TLS 会话：新连接不需要重新握手，空闲 30 秒后关闭。

**Shadowsocks**（`type: ss`）

| 字段 | 说明 |
|---|---|
| `cipher` | Prism 内核支持 `aes-128-gcm`、`aes-256-gcm`、`chacha20-ietf-poly1305`，以及 Shadowsocks 2022：`2022-blake3-aes-128-gcm`、`2022-blake3-aes-256-gcm`、`2022-blake3-chacha20-poly1305` |
| `password` | 2022 方法的密码是 base64 编码的密钥（aes-128 为 16 字节，其余 32 字节）；多用户 / 中转服务器写成 `iPSK:uPSK`（多个 iPSK 用冒号连接，仅 AES 方法）。TCP 和 UDP 都可用 |
| `password` | 密码 |

带 `plugin`（obfs / v2ray-plugin 等）的节点会被跳过，只能在 Mihomo 内核下使用；`2022-blake3-*` 系列加密 Prism 内核不支持（节点被跳过），Mihomo / sing-box 内核可用。

**Trojan**（`type: trojan`）

| 字段 | 说明 |
|---|---|
| `password` | 密码 |
| `sni`、`skip-cert-verify`、`alpn` | 同 AnyTLS |
| `network` | 传输方式，见下面的「传输方式」 |

**VLESS**（`type: vless`）

| 字段 | 说明 |
|---|---|
| `uuid` | 用户 UUID |
| `tls` | 是否启用 TLS |
| `servername` | TLS 服务器名 |
| `skip-cert-verify`、`alpn` | 同上 |
| `flow` | `xtls-rprx-vision`（需 TLS 或 Reality）。Vision 节点的 UDP 以 XUDP 方式转发（每个 UDP 会话一条连接） |
| `reality-opts` | `public-key`、`short-id`：VLESS Reality；`servername` 为伪装的网站域名 |
| `network` | 传输方式，见下面的「传输方式」。`xtls-rprx-vision` 只能用默认的 TCP |

**VMess**（`type: vmess`）

| 字段 | 说明 |
|---|---|
| `uuid` | 用户 UUID |
| `alterId` | 填 `0`。Prism 内核只使用 AEAD 认证（当前 V2Ray / Xray / sing-box 服务端的默认要求），不使用旧式 MD5 认证 |
| `cipher` | `auto`（= `aes-128-gcm`）、`aes-128-gcm`、`chacha20-poly1305`、`none` / `zero` |
| `tls`、`servername`、`skip-cert-verify`、`alpn`、`client-fingerprint` | TLS 参数，同 VLESS |
| `network` | 传输方式，见下面的「传输方式」 |

UDP 以 VMess 自身的 UDP 请求转发（每个目标一条连接）。

**传输方式**（VLESS、Trojan、VMess 的 `network`）

| `network` | 选项 | 说明 |
|---|---|---|
| 不写或 `tcp` | | 直接在 TCP / TLS 上传输 |
| `ws` | `ws-opts: { path: /路径, headers: { Host: 域名 } }` | WebSocket |
| `ws` + `ws-opts.v2ray-http-upgrade: true` | 同上 | HTTPUpgrade：只用 WebSocket 的升级请求，之后直接传输（sing-box 中为 `httpupgrade` 传输） |
| `grpc` | `grpc-opts: { grpc-service-name: 服务名 }` | gRPC（HTTP/2，`/服务名/Tun`） |
| `h2` | `h2-opts: { host: [域名], path: /路径 }` | HTTP/2：一条 `PUT` 请求双向传输（需 TLS，ALPN 为 `h2`；sing-box 中为 `http` 传输） |
| `xhttp` | `xhttp-opts: { path: /路径, host: 域名, mode: auto, x-padding-bytes: 100-1000 }` | Xray 的 xhttp（SplitHTTP），走 HTTP/2。`mode`：`stream-one`（一条 POST 双向传输）、`stream-up`（GET 下载 + 一条流式 POST 上传）、`packet-up`（GET 下载 + 按序号的多个 POST 上传，适合 CDN）；`auto`（默认）在 REALITY 下用 stream-one，否则用 packet-up。可与 TLS、REALITY、TLS 指纹叠加。不支持 `download-settings`（分离上下行），sing-box 不支持 xhttp |
| `http`、`h2` 等 | | Prism 内核不支持，这样的节点会被跳过 |

- 开启 TLS 且没写 `alpn` 时，WebSocket / HTTPUpgrade 使用 `http/1.1`，gRPC 使用 `h2`。
- `Host` 不写时，开启 TLS 用 `servername`（或 `sni`），否则用服务器地址。
- 路径里的 `?ed=2048` 等早期数据参数会原样发送，不使用早期数据（服务端兼容）。

**SOCKS5**（`type: socks5`）、**HTTP**（`type: http`）

| 字段 | 说明 |
|---|---|
| `username`、`password` | 认证信息，可省略 |
| `tls` | 通过 TLS 连接上游代理 |
| `sni`、`skip-cert-verify` | TLS 参数 |

**Hysteria2**（`type: hysteria2`）

| 字段 | 说明 |
|---|---|
| `password` | 认证密码 |
| `sni`、`skip-cert-verify`、`alpn` | TLS 参数（ALPN 默认 `h3`） |
| `obfs`、`obfs-password` | 混淆，仅支持 `salamander` |
| `up`、`down` | 带宽提示；Prism 内核使用 QUIC 默认拥塞控制，不使用 Brutal |

**TUIC**（`type: tuic`，仅 v5）

| 字段 | 说明 |
|---|---|
| `uuid`、`password` | 用户 UUID 和密码（只有 `token` 的 v4 节点不支持，会被跳过） |
| `sni`、`skip-cert-verify`、`alpn` | TLS 参数（ALPN 默认 `h3`）；`disable-sni: true` 时不发送 SNI |
| `congestion-controller` | `cubic`（默认）、`new_reno`、`bbr` |
| `udp-relay-mode` | `native`（默认，QUIC 数据报，大包自动分片）或 `quic`（每个 UDP 包一条 QUIC 流） |

**WireGuard**（`type: wireguard`）

| 字段 | 说明 |
|---|---|
| `private-key` | 本机私钥（Base64） |
| `public-key`、`pre-shared-key` | 服务端公钥、预共享密钥（可省略） |
| `ip`、`ipv6` | 本机在隧道内的地址，如 `172.16.0.2`、`fd01::2`（可带 `/32` 等前缀长度），至少填一个 |
| `reserved` | 3 字节保留位，如 `[1, 2, 3]` 或其 Base64；Cloudflare WARP 需要 |
| `mtu` | 隧道 MTU，默认 1408 |
| `peers` | 也可以把 `server`、`port`、`public-key`、`pre-shared-key`、`reserved` 写在只含一项的 `peers` 列表里；多个 peer 不支持 |

Prism 内核在程序内实现 WireGuard 和 TCP/IP 协议栈，不创建网卡、不需要管理员权限。目标域名在本机解析后再经隧道连接；只填了 `ip` 时只能访问 IPv4 目标。AmneziaWG（`amnezia-wg-option`）不支持。

**所有节点通用：`dialer-proxy`**

```yaml
  - { name: 落地, type: ss, server: 5.6.7.8, port: 8388, cipher: aes-128-gcm, password: pw, dialer-proxy: 中转 }
```

`dialer-proxy` 写一个节点或代理组的名字：连接这个节点的服务器时先经过它（链式代理）。TUIC、Hysteria2、WireGuard 这类走 UDP 的节点需要前一跳支持 UDP。前一跳又回到自己（例如 `dialer-proxy` 指向的组当前选中了自己）时，连接会失败并在日志中提示，不会卡死。

**仅 Mihomo / sing-box 支持的**：上面标注不支持的传输方式和插件。Prism 能识别并在转换时保留它们，但 Prism 内核运行时会跳过这些节点。

### 6.4 proxy-groups（代理组）

```yaml
proxy-groups:
  - name: 组名
    type: select | url-test | fallback | load-balance | relay
    proxies: [节点或组名, ..., DIRECT, REJECT]
    url: https://www.gstatic.com/generate_204   # 健康检查地址（自动类型使用）
    interval: 300                               # 检查间隔（秒），最小 10
    tolerance: 50                               # url-test 切换阈值（毫秒）
    use: [集合名, ...]                          # 追加 proxy-providers 中的节点
    filter: '香港|HK'                           # 只要名字匹配该正则的集合节点
    exclude-filter: '过期|剩余'                  # 去掉名字匹配该正则的集合节点
```

| 类型 | 行为 |
|---|---|
| `select` | 手动选择，默认第一个 |
| `url-test` | 每隔 `interval` 秒测速，当前节点比最快节点慢超过 `tolerance` 毫秒时切换到最快的 |
| `fallback` | 使用列表中第一个可用（最近一次测速未失败）的节点 |
| `load-balance` | 在可用节点间分配；同一个目标主机始终走同一个节点 |
| `relay` | 链式代理：依次经过 `proxies` 中的每一个（第一个连第二个的服务器，以此类推），最后一个连接目标。成员可以是组（按当前选择）；除第一个外都必须是节点。`DIRECT` 成员会被忽略。新版 Mihomo 已不支持 `relay`，Prism 生成配置时会把它改写成一串用 `dialer-proxy` 相连的节点副本（最后一个副本使用组名） |

- 组可以嵌套组（`proxies` 里写其他组的名字）。
- 内置的 `DIRECT`（直连）、`REJECT`（拒绝）可直接使用；`GLOBAL` 组由内核自动生成，包含所有组和节点，供全局模式使用。
- 引用了不存在名字的成员会被删除并记录警告；成员全部无效的组会变成只含 `DIRECT`。
- `use` 的节点排在 `proxies` 之后；`filter` / `exclude-filter` 只作用于集合节点。

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
| `NETWORK` | `tcp` 或 `udp` | `NETWORK,udp,DIRECT` |
| `PROCESS-NAME` | 发起连接的程序名（不区分大小写，可省略 `.exe`） | `PROCESS-NAME,Telegram.exe,PROXY` |
| `PROCESS-PATH` | 程序完整路径（不区分大小写，`/` 与 `\` 等同） | `PROCESS-PATH,D:\Games\Steam\steam.exe,DIRECT` |
| `PROCESS-NAME-REGEX` / `PROCESS-PATH-REGEX` | 程序名 / 路径匹配正则 | `PROCESS-NAME-REGEX,^(chrome\|msedge)\.exe$,PROXY` |
| `RULE-SET` | 规则集合（rule-providers，见 6.8）中的任一条 | `RULE-SET,reject,REJECT`、`RULE-SET,telegram,PROXY,no-resolve` |
| `SRC-GEOIP` | 来源 IP 的分类（`lan` / `private` 为私有地址） | `SRC-GEOIP,lan,DIRECT` |
| `AND` / `OR` | 括号里的条件全部 / 任一满足，条件可以嵌套组合规则 | `AND,((NETWORK,udp),(DST-PORT,443)),REJECT` |
| `NOT` | 括号里的唯一条件不满足 | `NOT,((GEOSITE,cn)),PROXY` |
| `MATCH`（或 `FINAL`） | 匹配所有，放在最后 | `MATCH,PROXY` |

**no-resolve**：`GEOIP`、`IP-CIDR` 类规则遇到域名目标时，默认会先解析出 IP 再匹配（超时 3 秒）。加上 `no-resolve` 则域名目标直接跳过这条规则，避免为了匹配规则而做 DNS 查询。一般给 IP 类规则都加上 `no-resolve`，只在确实需要按 IP 分流的规则上去掉。

**GEOSITE / GEOIP 数据**：Prism 内核读取 v2ray 格式的 `geosite.dat`、`geoip.dat`（MetaCubeX/meta-rules-dat），首次使用时自动下载，设置页可手动更新。数据文件缺失时这两类规则会被整体跳过；数据文件中不存在的分类，对应规则也会被忽略。

进程规则通过系统的 TCP / UDP 连接表找出连接所属的程序：系统代理模式下识别本机通过代理端口连接的程序，TUN 模式下识别所有本机程序；局域网其他设备的连接没有程序信息。

**Prism 内核不支持的规则**（会被跳过并在日志中警告，Mihomo 内核可用）：`IP-ASN`、`SUB-RULE` 等。组合规则里只要有一个条件不支持，整条规则都会被跳过。

### 6.6 dns（DNS 模块）与 hosts

```yaml
hosts:
  router.lan: 192.168.1.1
  myserver.test: [10.0.0.2, 10.0.0.3]

dns:
  enable: true
  listen: 127.0.0.1:1053          # 可选：对外提供 DNS 服务（UDP）
  ipv6: false                     # false 时 AAAA 查询返回空
  enhanced-mode: fake-ip          # fake-ip / redir-host / normal
  fake-ip-range: 198.18.0.1/16
  fake-ip-filter:                 # 这些域名返回真实地址
    - '*.lan'
    - '+.msftconnecttest.com'
    - 'geosite:cn'
  default-nameserver:             # 只能写 IP，用来解析下面服务器的域名
    - 223.5.5.5
    - 119.29.29.29
  nameserver:
    - https://doh.pub/dns-query
    - https://dns.alidns.com/dns-query
  nameserver-policy:              # 按域名指定服务器，从上到下第一个匹配生效
    'geosite:geolocation-!cn': ['https://1.1.1.1/dns-query#PROXY']
    '+.corp.example': 'tcp://10.1.1.1'
  proxy-server-nameserver:        # 可选：专门解析代理服务器域名
    - https://doh.pub/dns-query
```

**服务器写法**：

| 写法 | 协议 |
|---|---|
| `223.5.5.5`、`udp://1.1.1.1:53` | UDP |
| `tcp://8.8.8.8` | TCP |
| `tls://dns.alidns.com` | DNS over TLS（853） |
| `https://dns.alidns.com/dns-query` | DNS over HTTPS |
| 以上任意写法后加 `#组名或节点名` | 通过该代理查询（如 `https://1.1.1.1/dns-query#PROXY`）；`#DIRECT` 表示直连 |

还支持 DNS over QUIC：`quic://dns.alidns.com`（默认端口 853），以及 DNS over HTTP/3：`https://dns.alidns.com/dns-query#h3=true` 或 `h3://cloudflare-dns.com/dns-query`；两者都复用一条 QUIC 连接，同样可以加 `#代理`（如 `quic://dns.adguard-dns.com#节点选择`）。不支持 `dhcp://`、`system`。

**fallback**（防污染）：

```yaml
dns:
  nameserver: [223.5.5.5, 119.29.29.29]
  fallback: ['https://1.1.1.1/dns-query', 'quic://dns.adguard-dns.com']
  fallback-filter:
    geoip: true          # 默认 true：主服务器返回的地址不在 geoip-code 内，就改用 fallback 的结果
    geoip-code: CN       # 默认 CN
    ipcidr: [240.0.0.0/4]       # 返回这些地址也视为被污染
    domain: ['+.google.com']    # 这些域名直接只问 fallback（也可写 geosite:分类）
```

有 `fallback` 时，A / AAAA 查询同时发给 `nameserver` 和 `fallback`：主服务器的结果非空且每个地址都通过过滤（局域网地址总是通过）就用它，不必等 fallback；否则用 fallback 的结果（fallback 失败时仍用主服务器的结果）。`nameserver-policy` 命中的域名不经过 fallback。内核自己的解析（直连目标、代理服务器域名）同样适用。`geoip` 过滤需要 `geoip.dat`。

**域名写法**（`fake-ip-filter`、`nameserver-policy` 的键）：`example.com` 仅该域名；`+.example.com` 该域名及所有子域名；`.example.com` 所有子域名；`*.example.com` 恰好一级子域名；`geosite:分类`（可写 `geosite:cn,private`）。

**工作方式**：
- 应用的查询（`listen` 端口、TUN 接管的 DNS）：先查 `hosts`；fake-ip 模式下，不在 `fake-ip-filter` 中的域名返回一个假地址（TTL 1 秒），之后连向该地址的连接会还原成域名，再按域名规则分流、由代理服务器解析；其余查询按 `nameserver-policy` / `nameserver` 转发，所有服务器同时查询、取最先返回的结果，并按 TTL 缓存。
- 内核自己的解析（直连目标、代理服务器域名、IP 类规则）只用直连的服务器：`proxy-server-nameserver`，没有则用不带 `#代理` 的 `nameserver`，再没有则用 `default-nameserver`。因此代理服务器的域名不会绕回经过代理本身来解析，也不会得到假地址。
- 重新加载配置（热重载）时 fake-ip 映射会保留。

### 6.7 proxy-providers（代理集合）

```yaml
proxy-providers:
  airport:
    type: http                        # http / file / inline
    url: https://example.com/sub?token=xxx
    path: ./providers/airport.yaml    # 缓存文件（相对数据目录 runtime\），默认 ./proxy_providers/<名字>.yaml
    interval: 3600                    # 刷新间隔（秒）
    filter: '香港|日本'                # 只保留名字匹配的节点
    exclude-filter: '剩余|过期'
    proxy: DIRECT                     # 下载经过的出站，默认直连
    health-check:
      enable: true
      url: https://www.gstatic.com/generate_204
      interval: 300
  local:
    type: file
    path: ./my-nodes.yaml
  inline-nodes:
    type: inline
    payload:
      - {name: 备用, type: ss, server: 1.2.3.4, port: 8388, cipher: aes-256-gcm, password: pw}

proxy-groups:
  - {name: PROXY, type: select, use: [airport, local], filter: '香港'}
```

- 集合内容可以是 Clash YAML（含 `proxies:`）或分享链接列表（含 Base64），格式同订阅。
- `http` 集合：缓存文件不存在或超过 `interval` 时，启动内核时下载；下载失败时继续使用旧的缓存文件（日志中提示）。内核运行期间每 `interval` 秒刷新一次，内容有变化时自动热重载，已选节点保持不变。
- 集合中的节点名与已有名字重复时，会改名为 `节点名 [集合名]`。
- 集合节点只能通过组的 `use` 使用（不会直接出现在全局模式的 GLOBAL 列表顶层）。

### 6.8 rule-providers（规则集合）

```yaml
rule-providers:
  reject:
    type: http                        # http / file / inline
    behavior: domain                  # domain / ipcidr / classical
    format: yaml                      # yaml / text
    url: https://example.com/reject.yaml
    path: ./rules/reject.yaml         # 默认 ./rule_providers/<名字>.yaml
    interval: 86400
  telegram:
    type: http
    behavior: ipcidr
    format: text
    url: https://example.com/telegram.txt
    interval: 86400
  custom:
    type: inline
    behavior: classical
    payload:
      - DOMAIN-KEYWORD,ads
      - DST-PORT,6881-6889

rules:
  - RULE-SET,reject,REJECT
  - RULE-SET,telegram,PROXY,no-resolve
  - RULE-SET,custom,REJECT
```

| behavior | 每一条的写法 |
|---|---|
| `domain` | `example.com`、`+.example.com`、`.example.com`、`*.example.com`（同 6.6 的域名写法） |
| `ipcidr` | `91.108.4.0/22`、`2001:b28:f23d::/48` |
| `classical` | 不带目标的规则：`DOMAIN-SUFFIX,google.com`、`IP-CIDR,1.0.0.0/8`、`DST-PORT,22`、`NETWORK,udp` 等（不支持其中的 `GEOSITE` / `GEOIP` / `RULE-SET`） |

- `format: yaml` 时内容为 `payload:` 列表；`format: text` 时每行一条，`#` 开头为注释；`format: mrs` 是 mihomo 的二进制规则集（如 MetaCubeX meta-rules-dat 的 `.mrs` 文件），支持 `behavior: domain` 和 `ipcidr`（mrs 本身不支持 classical），文件里的类型与 `behavior` 不一致时会跳过并警告。域名中间带通配符的条目（如 `a.*.example.com`）不支持，会计入跳过的条数。
- 下载、缓存、刷新和热重载与代理集合相同。

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
| `type` | `tcp`（默认）/ `ws` / `httpupgrade` / `grpc` |
| `path`、`host` | WebSocket / HTTPUpgrade 路径和 Host |
| `serviceName` | gRPC 服务名 |
| `flow`、`pbk`、`sid` | Vision / Reality 参数 |

**VMess**（v2rayN 格式）

```
vmess://BASE64({"ps":"节点名","add":"服务器","port":"443","id":"UUID","aid":"0","scy":"auto","net":"ws","host":"域名","path":"/路径","tls":"tls","sni":"域名"})
```

`net` 为 `tcp` / `ws` / `httpupgrade` / `grpc`（gRPC 时 `path` 是服务名）；`fp`、`alpn` 为 TLS 指纹和 ALPN。

**Hysteria2**

```
hysteria2://密码@服务器:端口?sni=域名&obfs=salamander&obfs-password=混淆密码&insecure=1#节点名
hy2://...（同上）
```

**TUIC**

```
tuic://UUID:密码@服务器:端口?sni=域名&alpn=h3&congestion_control=bbr&udp_relay_mode=native&allow_insecure=1#节点名
```

**WireGuard**（v2rayN 格式）

```
wireguard://私钥@服务器:端口?publickey=服务端公钥&presharedkey=预共享密钥&address=172.16.0.2/32,fd01::2/128&reserved=1,2,3&mtu=1280#节点名
wg://...（同上）
```

私钥、公钥中的 `+`、`/`、`=` 需 URL 编码。

**SOCKS5**

```
socks5://用户名:密码@服务器:端口#节点名
```

以上节点类型和 WS / HTTPUpgrade / gRPC / HTTP/2 传输三个内核都可用。VLESS / Trojan 链接里 `type=h2`（或 `type=http`）是 HTTP/2 传输，`type=xhttp&path=/路径&host=域名&mode=auto` 是 xhttp（Prism 内核和 Mihomo 可用）；`ss://` 链接可以直接写 2022 方法，如 `ss://2022-blake3-aes-128-gcm:密钥（base64，需 URL 编码）@服务器:端口#名称`。其他传输（`kcp`、`quic` 等）只能在 Mihomo / sing-box 内核下使用。

## 8. sing-box JSON 与格式转换

### 8.1 使用 sing-box 内核时

- **sing-box JSON 订阅**原样使用，但 `inbounds` 会被替换为 Prism 的混合入站（开启 TUN 时再加 TUN 入站），并强制设置 Clash API 和日志级别。
- **Clash YAML / 分享链接订阅**会转换为 sing-box 配置（要求 sing-box 1.12 及以上）：

| Clash 规则 | sing-box 路由规则 |
|---|---|
| `DOMAIN` / `DOMAIN-SUFFIX` / `DOMAIN-KEYWORD` / `DOMAIN-REGEX` | `domain` / `domain_suffix` / `domain_keyword` / `domain_regex` |
| `IP-CIDR` / `IP-CIDR6` | `ip_cidr` |
| `SRC-IP-CIDR` | `source_ip_cidr` |
| `DST-PORT` / `SRC-PORT` | `port` / `port_range`、`source_port` / `source_port_range` |
| `NETWORK` | `network` |
| `PROCESS-NAME` / `PROCESS-PATH` | `process_name` / `process_path` |
| `GEOIP,private` | `ip_is_private` |
| `SRC-GEOIP,xx` | 规则集 `geoip-xx` 加 `rule_set_ip_cidr_match_source` |
| `AND` / `OR` | `type: logical`，`mode: and` / `or` |
| `NOT` | 条件加 `invert: true` |
| `GEOIP,xx` | 规则集 `geoip-xx` |
| `GEOSITE,xx` | 规则集 `geosite-xx` |
| 目标 `REJECT` | `action: reject` |
| `MATCH` | `route.final` |

  其他类型的规则（包括 `RULE-SET`）在转换时会被丢弃；`proxy-providers` 也不会转换（组里只保留 `proxies` 中写明的成员）。代理组 `select` 转为 `selector`，其余自动类型都转为 `urltest`。
  WireGuard 节点转为 sing-box 的 `endpoints`；`dialer-proxy` 转为 `detour`；`relay` 组转为一串经 `detour` 相连的节点副本（最后一个副本使用组名），第一跳可以是组，后面的必须是节点。

- **规则集**：GEOSITE / GEOIP 对应的 `.srs` 规则集由 Prism 下载到 `runtime/rule-set/`（优先经运行中的代理，其次 jsdelivr 镜像，最后 GitHub），再以本地规则集交给 sing-box。这样 sing-box 启动时不依赖网络，节点不可用也能启动。超过 7 天的规则集会在后台刷新；实在下载不到的规则集会连同使用它的规则一起跳过，并在日志中提示。原生 sing-box 订阅里的远程规则集也按同样方式处理。
- 全局 / 直连模式通过 sing-box 的 `clash_mode`（`Global` / `Direct`）实现。

### 8.2 sing-box JSON 订阅用于 Prism / Mihomo 内核时

sing-box 配置会转换为 Clash YAML：

- 出站：`anytls`、`shadowsocks`、`trojan`、`vless`、`vmess`、`hysteria2`、`tuic`、`socks`、`http` 转为节点（`detour` 转为 `dialer-proxy`），只有一个 peer 的 WireGuard `endpoints` 也转为节点；`selector` / `urltest` 转为代理组；`direct` 视为 `DIRECT`，`block` 视为 `REJECT`。
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
| `PUT /configs` | 热重载配置文件（或 `{"path":"..."}` / `{"payload":"..."}`），监听端口不中断 |
| `GET /proxies`、`GET /proxies/{名称}` | 节点和组 |
| `PUT /proxies/{组名}` | `{"name":"节点名"}` 选择节点 |
| `GET /proxies/{名称}/delay?url=&timeout=` | 测速 |
| `GET /group/{组名}/delay?url=&timeout=` | 整组测速 |
| `GET /connections`、`DELETE /connections`、`DELETE /connections/{id}` | 连接列表与关闭 |
| `GET /rules` | 已加载的规则 |
| `GET /traffic` | 每秒一行的实时速度 |
| `GET /providers/proxies`、`GET /providers/proxies/{名称}` | 代理集合 |
| `PUT /providers/proxies/{名称}` | 立即更新集合（内容有变化时热重载） |
| `GET /providers/proxies/{名称}/healthcheck` | 集合内节点测速 |
| `GET /providers/rules`、`PUT /providers/rules/{名称}` | 规则集合与更新 |

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
| `runtime\proxy_providers\`、`runtime\rule_providers\` | 代理集合、规则集合的缓存（默认位置） |
| `cores\` | 下载的 `mihomo.exe`、`sing-box.exe` |

想恢复初始状态：退出 Prism 后删除整个数据目录即可。

设置环境变量 `PRISM_DATA_DIR` 可以让 Prism 使用另一个数据目录（例如放在 U 盘上便携使用）。

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
确认已安装 TUN 服务（设置页显示「已安装，运行中」）或以管理员身份运行，并关闭其他客户端的 TUN 模式。设置页显示「已安装，未运行」时点 **重新安装 / 更新内核**。Prism 内核需要 `wintun.dll`，首次使用会自动下载；下载失败时可手动从 [wintun.net](https://www.wintun.net/) 下载，把 `wintun/bin/amd64/wintun.dll` 放到数据目录的 `runtime\` 下。

**应用分流不生效**
确认处于**规则模式**；在「连接」页看该程序的连接是否出现、「程序」列是否识别正确。没有出现说明该程序没走系统代理，请开启 TUN 模式。按程序名匹配时，名字需与「程序」列一致（如 `Telegram.exe`）。

**客户端或内核异常退出时**
- **只有客户端退出、内核还在**（客户端闪退或被强制结束）：内核继续运行，规则和「出口异常时拒绝」继续生效。TUN 服务模式下内核本来就独立于客户端运行。
- **内核也退出了，用的是系统代理**：系统代理仍指向已经关闭的端口，走系统代理的程序连不上网，不会变成直连。不走系统代理的程序本来就不受规则约束。
- **内核也退出了，用的是 TUN**：虚拟网卡随内核消失，流量会回到系统默认路由，也就是**直连**，规则不再起作用。不想发生这种情况，请开启下面的**断网保护**。

**断网保护**（设置页 → TUN 服务，仅 Windows，需要安装 TUN 服务，默认关闭）
- 开启 TUN 时，只允许流量经 TUN、本机和局域网通过（另外放行内核自身和 DHCP）。内核崩溃后继续拦截，流量不会变成直连。
- 拦截生效时窗口顶部会提示，可以 **重启内核** 或 **解除拦截**。
- 拦截规则由 TUN 服务持有，不写入系统防火墙配置：解除拦截、停止内核、退出 Prism、卸载 TUN 服务、服务崩溃或重启电脑后都会消失，不会把电脑一直锁在断网状态。重启电脑后要再次启动内核才会恢复保护。
- 生效期间，绕过 TUN 直接走物理网卡的程序（如部分远程控制软件、其他 VPN）也会被拦截；通过远程桌面控制这台电脑时请谨慎开启。
- 局域网地址始终放行，所以局域网流量不受保护。全局 / 直连模式下内核仍在运行，直连流量由内核发出，不算绕过，也不会被拦截。

**点机场的「一键导入」没有反应，或打开了别的客户端**
`clash://` 链接由最后登记它的客户端打开。在设置页打开 **接管 clash:// 链接**；页面上会显示当前由哪个程序打开。如果面板提供 `prism://` 链接或「复制订阅地址」，也可以直接用。

**订阅没有自动更新**
在订阅卡片上查看间隔（「不自动更新」表示间隔为 0，在 **编辑** 中修改），日志中会记录每次自动更新的结果和失败原因。Prism 需要在运行（可以只在托盘）才会自动更新。

**开着 TUN，代理网站打不开，直连网站正常**
v0.1.3 及更早的 Prism 内核有这个问题：订阅规则里把 `198.18.0.0/15`（fake-ip 地址段）设为直连时（很多规则集把它算作内网地址），经 TUN 的 HTTPS 连接会被误判为直连。新版已修复；也可以暂时改用 Mihomo / sing-box 内核。

**检查更新失败**
多半是无法访问 GitHub。先启动内核（检查会自动经过代理）再点 **检查更新**。

**内核下载失败**
GitHub 在部分网络下访问较慢。先用 Prism 内核连上代理，再到设置页下载 Mihomo / sing-box，下载会自动经过代理。


## 运行记录、节点合并与连接筛选

- 概览页显示当前内核本次持续运行时长；重新启动内核后重新计时。
- 日志页的“安全事件”保存最近 2000 条运行、断网保护和拒绝事件，并可按类型筛选。记录位于应用数据目录的 `security-events.json`，关闭应用后保留，清空普通内核日志不会删除它们。拒绝记录依赖内核输出日志，关闭日志或提高日志等级可能使部分事件不可见。
- 合并订阅只向节点选择与自动选择入口加入节点，不向地区或业务组兜底添加。支持带符号的“节点选择”“手动选择”“自动选择”“自动测速”“自动优选”，以及 Proxy / Proxy Select / Node Select / Auto / Auto Select / URL Test 等常见名称；无法识别入口时会在日志中警告，可在本地配置或覆写中调整组名。原订阅的规则保留。
- 在订阅页点击“添加单个节点”，输入一个分享链接或只包含一个节点的 Clash / sing-box 配置，可另填名称。节点独立保存在本地，自动合并到当前订阅，不受远程更新覆盖；没有订阅时自动启用它。依赖其他节点或代理组的节点需要通过完整配置导入。
- 连接页支持关键词、TCP / UDP、出口筛选，以及开始时间、主机、程序、流量和速度的升降序排列。地图和导出使用相同的筛选结果；鼠标停在列表上时暂时保持顺序，便于操作连接。
- TUN 模式下拦截 TCP / UDP 53 端口的 DNS；Prism DNS 模块不可用时拒绝查询，不直接转发到客户端指定服务器。Prism 的 DNS 上游域名必须配置显式的 IP 引导服务器 `default-nameserver`，缺失时不会调用系统 DNS。引导查询仍发送至明确配置的服务器；应用自行使用的 DoH / DoT 则按普通连接分流。


## Prism DNS 与 IPv6 保护

设置页的“Prism DNS 与 IPv6 保护”提供订阅 DNS 覆盖、DNS 经代理、代理出口、DoH 上游、代理服务器域名引导 DoH、fake-IP、AAAA 查询和客户端 IPv6 目标连接阻断。

新安装或没有 DNS 保护设置的配置默认开启 DNS 覆盖和“DNS 经代理查询”，使用经所选代理访问的 DoH；已经明确保存的关闭状态仍保留。建议配合 TUN，指定节点选择组或实际节点，启用 fake-IP；需要 IPv4-only 客户端连接时，关闭 AAAA 并启用 IPv6 目标阻断。保存后重启运行中的内核。当前这些设置仅作用于 Prism，其他内核保留原配置。

Windows Prism TUN 自动路由启用时，还会独立安装 DNS 防绕过规则：普通程序的 IPv4/IPv6 TCP/UDP 53 查询只允许经本次 TUN 接口，阻断直接发往物理网卡和局域网路由器的查询。内核自身保留必要的上游 DNS 连接，最终使用代理还是直连由 DNS 配置决定；此规则不受 DNS 覆盖开关或互联网断网保护开关影响。安装规则失败时 TUN 启动失败，不静默退化。规则随 TUN/内核退出撤销，热重载和上联网卡切换不会移除规则。这不表示系统所有 HTTPS/DoT 查询都能按 DNS 内容识别或拦截。

Windows TUN 会先启用网卡会话，并等待配置的 IPv4/IPv6 地址完成重复地址检测、进入可用状态后才报告启动完成。地址冲突或等待超时会使启动失败，避免网卡看似已启动但 DNS 仍不可达。

普通目标域名与代理服务器域名使用不同的解析通道和缓存：普通目标遵守 DNS 上游/策略，代理服务器域名使用独立直连引导 DNS，避免循环依赖。覆盖模式清除订阅中的 nameserver-policy 和 fallback，防止未知直连查询；“真实 IP”页面明确配置的 DNS 例外仍然保留。DNS 出口切换为 DIRECT、REJECT、不可用节点或链式组时，代理保护模式拒绝查询，不隐式退回直连。

引导 DoH 仍是直连，只解析建立代理连接所需的服务器域名；请使用 IP 地址的 HTTPS DoH，避免其自身需要额外 DNS。直连 DNS 模式的普通 DoH 上游也需要 IP 地址。浏览器自带 DoH 是普通 HTTPS 连接，按原有分流规则处理，不会被 53 端口 DNS 劫持开关自动接管。

关闭 AAAA 只控制解析应答，不代表系统 IPv6 被关闭。IPv6 阻断保留 TUN 的 IPv6 路由并拒绝客户端 IPv6 地址目标（包括规则、全局和直连模式），防止移除路由后绕行；开启阻断时如果 IPv6 TUN 路由安装失败，内核启动失败。它不会修改系统网卡设置，也不控制代理服务器在远端如何解析域名。若 DNS 测试仍显示异常，应对照解析器是否属于本地 ISP、浏览器安全 DNS、自定义真实 IP 例外和当前流量规则，不能仅根据解析器地区判断泄露。


## Tailscale 组网

Tailscale 页面通过已安装的官方客户端工作，提供官方安装/注册入口、登录授权、连接与断开、设备列表、IP/MagicDNS 展示和单设备连通测试。没有账号时先安装官方客户端并注册，在浏览器完成授权；其他设备加入同一 tailnet 后即可互访。Prism 不保存 Tailscale 密码或授权密钥，应用不会自行注册账号、加入网络或安装软件。

“Prism TUN 组网直连”默认关闭。开启后保留操作系统的 100.64.0.0/10 和 fd7a:115c:a1e0::/48 组网路由，避免把这些连接固定到物理上联网卡；ts.net 域名由 Tailscale 的 100.100.100.100 解析，不使用 fake-IP。此例外在 Prism 规则/全局/直连模式下生效，并明确豁免客户端 IPv6 阻断。该模式需要官方客户端已连接，不支持将 Tailscale 出口节点与 Prism 的全局 TUN 叠加作为可靠出口；首次使用请先仅配置设备互联。

Windows 的组网直连可以与断网保护共同启用，需要更新 TUN 服务。保护服务独立查询 SCM 的 Tailscale 服务进程，核对 LocalSystem 身份、官方 Program Files 安装路径、文件签名与 Tailscale Inc. 发行者；签名验证使用本机缓存的证书链和吊销信息，不为验证开放额外联网通道。验证失败保持拦截。

守护进程仅获准 UDP 传输和 TCP 443 控制/DERP 连接，规则同时匹配程序 AppID 和 SYSTEM 身份，不放行其他程序的互联网连接。组网设备连接仅允许真实在线 Tailscale 虚拟接口的 LUID 与其设备清单内具体地址（含 MagicDNS 地址）组合，不开放整个组网网段。100.64.0.0/10 和 fd7a:115c:a1e0::/48 的兜底阻断优先于既有局域网/ULA 放行，防止网卡断开后退回物理网络。内核创建组网 socket 时也会确认并绑定有效 Tailscale 接口；路由不匹配则拒绝。

服务在后台每约 2 秒核验一次，不依赖 Prism 页面或内核进程是否存活。Tailscale 断开、接口失效、服务进程更换或验证失败时撤销组网与守护进程例外，保持基础互联网阻断；外部断开会有状态查询和轮询的短暂检测窗口。Prism 的断开按钮会先撤销授权，再请求官方客户端断开。WFP 更新错误会明确显示并重试，不会关闭基础保护。

为了让尚未上线的官方守护进程能够登录/重连，用户点击连接时可获得最多 60 秒的短期守护进程授权；该阶段不开放组网接口或设备流量，超时自动撤销，断开按钮可立即取消。在线状态验证通过后转为正常的接口受限规则。未在一分钟内完成网页授权时可以再次点击连接。

验证期间服务持有禁止写入/删除的程序文件句柄，避免白名单生效时二进制被替换；需要更新官方 Tailscale 时可先在 Prism 点击断开以释放这些句柄。其他内核可使用官方客户端，但此组网路由兼容开关和受控共存目前只作用于 Windows 上的 Prism。

## 内核自动恢复

设置页提供“内核自动恢复”，默认开启。它只监护由 Prism 启动且仍期望运行的内核；手动停止、安装更新和退出应用会取消运行意图。后台每约 10 秒观察内核状态和 /connections 数据，不依赖连接页面是否打开。

- 内核状态连续两次确认退出时尝试重启。
- 每约 30 秒经本机代理端口探测联网，无论连接 ID、连接流量计数、总流量是否变化；连接重试或背景流量不会推迟检测。无变化阈值（默认 120 秒）用于附加诊断提示，不作为周期探测的前提。
- 开启 TUN 时，代理探测成功后还会使用系统 DNS、系统路由检查联网，禁用环境代理；代理端口正常而 TUN/系统通路失败也会触发故障确认。检测不会修改路由或放宽断网保护。仅代理模式不检查系统直连路径。
- 连续三轮未能通过通路检测才因网络无响应重启；每条通路会尝试配置的探测地址，任一地址有效响应即视为该通路正常。探测间隔至少 30 秒，HTTP 5xx 和代理认证错误不算健康。单次失败不重启；任一整轮检测正常即清除失败计数，正常空闲不会被当成故障。
- 启动后宽限 30 秒；自动重启至少间隔 60 秒，10 分钟内最多 3 次，达到限制后进入冷却，之后再尝试。设置页显示阶段、原因和本次自动重启次数，日志页安全事件中保存“自动恢复”记录。
- 持久化故障确认、通路类型、冷却、重启结果和联网恢复记录；安全事件总量最多 2000 条，其中为最近的运行/保护记录预留最多 500 条，避免拒绝日志刷屏覆盖故障经过。

重启操作与手动启动/停止、配置重载串行执行，停止请求会使旧的恢复请求失效。自动恢复不会弹出管理员授权、自动安装服务、切换节点或关闭断网保护。上游节点失效、外网断开或探测站点同时受限时，重启也可能无法恢复；请结合日志排查，不能将自动重启视为联网保证。
