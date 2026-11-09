# FluxDL 0.1.0 验证记录

此页保留MVP当时的实际结果。0.2.0工程优化的当前命令、增量数据/IPC、性能和验证边界见[本轮验证报告](engineering/0.2.0-validation-report.md)，不能用本页的旧测试数量或“未配置ESLint”判断当前工程。

验证日期：2026-11-09（Asia/Shanghai）。主机：Windows 11 x64 / 10.0.26200；Node 24.16.0、npm 12.0.1、Rust 1.97.0 MSVC、Visual Studio 2026 C++ 工具链、Edge WebView2。使用用户原有 yt-dlp 2026.07.04、FFmpeg/FFprobe n8.0.1；未下载或捆绑媒体引擎。

以下结果来自 MVP 开发期间的实际验证，执行时工程尚未首次提交，没有可引用的 Git commit。本轮准备首次提交仅核对原始报告、导出脱敏快照并整理文档，未重新执行下列测试或构建。可提交的 JSON 快照与本机原始报告保持字段、数值、真实 URL、runId 和通过值不变，仅将项目根目录及工具绝对路径替换为明确占位符；映射、原始文件摘要和核对方式见[快照说明](validation/README.md)。

## 已执行的检查

| 命令 | 实际结果 |
|---|---|
| `npm run typecheck` | 通过，TypeScript strict |
| `npm test` | 2个文件、7项通过 |
| `npm run build` | 通过，生产静态资源构建成功 |
| `npm run check:rust` | 通过 |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib` | 18项通过、1项按设计ignored |
| `.\scripts\test-queue.ps1` | ignored项显式运行通过，验证生产worker生命周期 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml --check` | 通过 |
| `npx tauri build --debug --no-bundle` | 通过，实际桌面EXE生成并启动 |
| `npm run desktop:build` | Windows x64 NSIS安装包构建成功 |
| `python scripts/collect-licenses.py` | 277个Windows依赖条目及原始许可证收集成功 |

未配置 ESLint，因此没有声称其通过。GitHub Actions 文件已提供；本轮首次提交的 push 将触发 CI，但本文整理时尚未取得远程执行结果，本机历史验证不能视为远程 CI 已通过。NSIS 安装向导未在用户系统中执行完整安装/卸载循环，避免更改用户现有安装状态。产物未代码签名。

## 真实公开媒体联调

通过生产 `Manager`，不是假引擎。来源为 Blender Foundation 的 Big Buck Bunny 公开预告片，CC BY 3.0（来源及许可见 THIRD_PARTY_NOTICES.md）。

| 流程 | 结果 |
|---|---|
| OGG公开源 → MP3 | 两个文件各919,772字节，FFprobe确认音轨32.997秒；28.75秒完成联调 |
| M4V公开源 → 视频文件 | 两个文件各3,889,885字节，FFprobe确认H264/AAC，33.023秒；26.38秒完成联调 |
| 串行与实时进度 | 两次均maxRunning=1，观察到真实下载字节进度与后处理状态 |
| 重启 | 设置、历史、最终路径和真实文件可用状态保留 |

可追溯的脱敏快照：[音频联调](validation/engine-audio-smoke.json)、[视频联调](validation/engine-video-smoke.json)。本机原始报告分别为 `artifacts/engine-smoke-report.json` 与 `artifacts/video-e2e/engine-smoke-report.json`；整个 `artifacts/` 属于忽略的本机产物，不提交原始报告。第一次视频请求发生远端 TLS EOF，应用返回可读错误；当时重新运行后通过，没有关闭TLS证书校验。

## 实际双窗口桌面回归

`node scripts/desktop-smoke.mjs <本地yt-dlp.exe>` 使用真实 Tauri EXE / 两个 WebView2 窗口 / 真实本地 yt-dlp / 原创 FFmpeg 测试图与正弦音轨 / HTTP 测试服务器。所有任务经实际 IPC 与生产队列执行，不使用Mock下载或伪造进度。

已通过的断言：

1. 手动配置现有引擎后，后端检测真实路径与版本。
2. 快速捕获与工作台是不同原生窗口；实际输入、解析、加入队列按钮产生真实输出文件。
3. 同时加入两个任务，第二个保持等待；可以取消等待任务。
4. 主窗口关闭时显示后台继续选择；隐藏工作台后真实字节数继续变化。
5. 运行中取消到达终态；重试取消任务可以完成。
6. 实际 HTTP 404 产生可读失败，恢复服务器后重试成功。
7. 暗/亮主题及紧凑密度在两个原生窗口同步。
8. 移走真实文件后，快照显示不存在、打开失败；仅删除历史不删除被移走的文件。
9. 真正停止并重启应用，设置与历史保留。
10. 强制终止本次测试创建的应用进程树后重启，在途任务标为已中断，不自动恢复。
11. 两个前端窗口未捕获到JavaScript运行异常。

脱敏快照：[桌面回归报告](validation/desktop-smoke.json)，本机原始报告为忽略的 `artifacts/desktop-smoke.json`。原始桌面截图在本机 `artifacts/`，提交的精选截图在 `docs/`。浏览器预览还检查过1120px和760px宽度、四个页面、深浅主题及Quick Capture空状态，无横向溢出。Quick Capture 632×687解析态已检查输入与媒体标题无遮挡，只有结果区域滚动。

Rust 自动化还覆盖：URL/预设/格式参数、程序扩展拒绝、未知总大小/分辨率、播放列表、缺FFmpeg、日志脱敏、输出路径越界、数据库损坏、重启中断、历史删除不删文件、Windows Job父子进程清理。

## 验证边界

- 原生文件/目录选择、文件关联播放器与资源管理器入口已实现；默认播放器、原生选择器及托盘菜单的逐项人工点击仍可按README体验路径复核。桌面自动化已验证关闭后的后台继续和重新显示窗口。
- 系统缩放125%/150%、跨物理显示器移动未做完整人工验收；EXE清单已确认嵌入PerMonitorV2和longPathAware。
- 没有授权账号、Cookies或私有站点测试；YouTube EJS完整性不能由版本检测保证，按实际站点警告诊断。
- Windows以外平台未验收；大量历史性能未基准，SQLite同步事务取舍见 `docs/persistence.md`。
- 不支持整播放列表、暂停、DRM绕过、自动安装更新引擎或全依赖绿色单文件；这些属于本版明确范围边界。

安装包、程序EXE、许可证及SHA-256汇总由 `npm run release:stage` 复制到项目 `release/`，不附带测试媒体或用户数据库。

## 最终发布产物

- `release/FluxDL_0.1.0_x64-setup.exe`：NSIS，2,584,038字节，构建成功，未执行安装向导。
- `release/FluxDL.exe`：程序，8,486,912字节，已实际启动，原生窗口标题FluxDL、句柄非零、Responding=true；确认没有活动任务后结束此次启动检查。
- `release/SHA256SUMS.txt`：两个EXE的SHA-256；发布文件和构建原件逐字节一致。
- `release/licenses/`、`LICENSE`、`THIRD_PARTY_NOTICES.md`、`使用说明.txt`：随交付附带。

最终源码和EXE已包含Quick Capture滚动修复。最终桌面回归报告runId为 `2026-11-09T04-00-43-540Z`，11项通过、errors为空。应用无遗留测试进程运行。
