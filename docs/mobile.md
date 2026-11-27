# 移动端已迁移

手机端已迁移至独立仓库 **[yuuolo/prism-phone](https://github.com/yuuolo/prism-phone)**。安卓原生界面、JNI 桥、构建/模拟器测试、升级发布脚本和功能规划均在该仓库维护。iOS/iPadOS 与 HarmonyOS NEXT 继续暂停。

本仓库负责桌面客户端与 Prism 共享内核，保留手机拆分前的完整开发历史。手机仓库包含标明来源提交的共享内核快照，能够独立构建；未来内核修复需按平台验证后同步。

现有安卓安装包和 `android-updates/android-update.json` 保留用于升级兼容。新手机发布脚本会向这里镜像相同签名的 APK，并维持旧路径的清单，因为已安装 Beta.1 仅接受本仓库的 APK URL。桌面稳定版的 `latest.json` 更新入口保持原有方式。

手机版本的功能范围与测试状态见 [手机仓库说明](https://github.com/yuuolo/prism-phone#readme)；后续手机问题请提交至 [手机 Issues](https://github.com/yuuolo/prism-phone/issues)。
