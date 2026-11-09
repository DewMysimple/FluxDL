# 验证报告快照

本目录保存 2026-11-09（Asia/Shanghai）开发验证报告的路径脱敏快照。原始测试执行时工程尚未首次提交，报告没有绑定 Git commit；不要将它们解释为后续某个 revision 已重新通过验证。

本轮首次提交准备只读取并核对本机原始 JSON、导出快照及维护文档，未重新执行应用测试、下载联调或构建。首次提交的 push 将触发仓库 CI；此处不宣称远程 CI 已执行或通过，远程结果应以对应 workflow run 为准。

## 来源映射

| 本机原始报告（不提交） | 仓库内脱敏快照 | 替换的字符串数 |
| --- | --- | --- |
| `artifacts/desktop-smoke.json` | [desktop-smoke.json](desktop-smoke.json) | 3 |
| `artifacts/engine-smoke-report.json` | [engine-audio-smoke.json](engine-audio-smoke.json) | 8 |
| `artifacts/video-e2e/engine-smoke-report.json` | [engine-video-smoke.json](engine-video-smoke.json) | 8 |

`artifacts/` 整体由 `.gitignore` 忽略，原始报告仍保留在生成它们的本机工作区。快照没有增加或删除 JSON 字段，没有修改数组顺序、数值、布尔值、null、版本、任务 ID、真实 URL、runId 或检查结果。

## 脱敏规则

- 项目根目录（包括 Windows 扩展长度路径前缀）替换为 `<PROJECT_ROOT>`，保留其后的相对目录及文件名。
- yt-dlp、FFmpeg、FFprobe、JavaScript 运行时的绝对可执行路径分别替换为 `<YT_DLP_EXECUTABLE>`、`<FFMPEG_EXECUTABLE>`、`<FFPROBE_EXECUTABLE>`、`<JAVASCRIPT_RUNTIME_EXECUTABLE>`。
- 本次仅改变桌面报告的 `engine/dataDir/outputDir`，以及两份引擎报告的四个引擎 `path` 与两条任务的 `finalPath/outputDir`。快照不含用户名路径或本机工具安装位置。
- 真实公开媒体 URL、测试服务器 URL、状态及通过值完整保留。占位符是文档标记，不是可直接运行的文件路径。

导出后按 JSON 结构比较，确认字段与非路径值保留，并检查快照没有残余 Windows 绝对路径或用户主目录路径。原始文件 SHA-256 如下，便于在持有本机原始报告时核对来源；快照的字节摘要因路径替换及 JSON 格式化而不同。

| 原始报告 | SHA-256 |
| --- | --- |
| 桌面回归 | `01233edb1d969159edd0253252b2ee7cd6ed01195a60ea28d44e9384ab959f63` |
| 音频联调 | `ea38a705eba3e84e2bb5d6558f2c1c142cf854abfd6d96d4ce6e0a4ed93b7ff8` |
| 视频联调 | `f8ecb2926af72f5e456f9f1a9b19d52e327a5017265f97bd25a8df735cc1799a` |

测试范围、历史命令结果及未完成的人工验收见[验证记录](../validation.md)。
