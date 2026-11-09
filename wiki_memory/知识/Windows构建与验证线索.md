---
type: knowledge
status: active
updated: 2026-11-09
topic: windows-build-validation
sources:
  - "docs/build-and-release.md"
  - "docs/engineering/release-workflow-validation.md"
  - "src-tauri/build.rs"
  - "src-tauri/windows.manifest"
  - "src-tauri/Cargo.toml"
  - "scripts/desktop-smoke.mjs"
  - "scripts/test-queue.ps1"
  - "docs/validation.md"
  - "docs/engineering/0.2.0-validation-report.md"
  - "scripts/native-desktop-smoke.mjs"
---

# Windows 构建与验证线索

## Windows 清单

当前 `build.rs` 使用 Tauri `new_without_app_manifest()`，保留图标/版本资源，再通过链接器为所有 Windows 最终目标统一嵌入 `windows.manifest`。历史上缺 Common-Controls v6 会令测试/示例启动失败，重复 MANIFEST1 则令主程序链接失败；不能只给主 EXE 补清单。清单同时设置 asInvoker、DPI 和长路径。

该取舍由 Agent 在构建修复时采用，已用于现有本机通过的示例、Rust 测试和 NSIS。来源：`build.rs`、`windows.manifest`、`docs/validation.md`。修改清单方案时按实际受影响目标构建验证，不只看 cargo check。

## 测试入口和边界

`scripts/test-queue.ps1` 编译确定性假引擎并显式运行默认 ignored 的 worker 生命周期测试；普通 Rust 单测通过不代表该项已执行。假引擎不替代真实媒体测试。

`scripts/desktop-smoke.mjs`生成原创测试图/音轨，经本地HTTP、真实yt-dlp、实际Tauri两窗口和IPC回归。它需要debug EXE、FFmpeg、显式yt-dlp路径；仅强制终止自己创建的进程树模拟异常退出。显式测试数据/profile与单实例gate可和已有用户release共存，不要求也不能停止用户实例；诊断端口被占先拒绝。隔离数据在忽略的artifacts，release不包含调试入口。可选seed、audio、error协议开关见0.2工程报告。

`native-desktop-smoke.mjs`自行启动/复核唯一隔离PID、EXE和精确创建时间，验证目录Cancel、注册托盘callback的真实菜单链及固定活引擎句柄退出；14通过/1未测的版本复测还读取实际页脚和About。UIAutomation不可访问入口及物理指针命中不得冒称通过。当前stage只更新release-output，不再写旧release；不能为发布结束用户运行实例。

公开媒体联调用 `src-tauri/examples/engine_smoke.rs`，报告数字只证明当时来源与环境。站点格式、网络、EJS 和外部引擎变化后，需要针对新任务核实，不能把旧报告当作现时通过。

## 正式产物工作流

完整正式命令为npm run release:build，复用确认过的Tauri输入为release:stage，独立验收为release:verify。最终仅release-output/FluxDL/和FluxDL.zip；成功收据、版本/源指纹、完整SHA256/ZIP CRC、发布锁/整体替换/回滚及保备份要求见docs/build-and-release.md。clean默认只预览，保留缓存和用户数据。

Windows现有Vite监听暂存目录曾使完整候选重命名EPERM；加watch ignored后同候选立即可移，原Vite PID未变，重建发布通过。不要用杀开发服务器/用户程序或忽略验证来解决。目录输出与工作区事务必须排除Vite监听。

## 证据与未核实项

2026-11-09，本轮0.2实际执行前端90项、Rust80项+独立fixture、三档基准、CSS12等值、真实桌面15项及原生版本14通过/1未测；当时的完整证据/构建见原工程报告；后续发布任务在干净bd5449c实际通过34项发布、90前端/80Rust+fixture、NSIS/独立verify、489文件Python和Windows解压，以及隔离debug开发启动（不代表发行EXE直接运行）。最新指纹与验收见docs/engineering/release-workflow-validation.md/json，docs/validation.md只保留0.1历史。Rust最低1.97；远程Actions、真实DPI/多屏和安装卸载仍需独立证据。
