# 原生交互补充验证

状态：**阶段 4 已实际运行，结果为 partial，不能视为全项通过**。控制器 `scripts/native-desktop-smoke.mjs` 独立启动隔离 debug 实例，通过 `scripts/native-smoke.ps1` 使用 Windows 自带的 Win32、UIAutomation 和 Windows PowerShell 5.1 取证，不增加依赖。此次没有操作用户已有实例；实测范围及未测项见文末。

## 隔离与前置条件

- 由独立原生控制器或主桌面 smoke 创建显式 debug 实例：EXE 必须是本项目 `src-tauri/target/debug/fluxdl.exe`，`FLUXDL_TEST_DATA_DIR` 和 WebView profile 均是该次 `artifacts/` 下的隔离目录。
- `windows.rs` 只有在 `debug_assertions` 且存在 `FLUXDL_TEST_DATA_DIR` 时，托盘 tooltip 才为 `FluxDL · 测试实例 <自身pid>`。普通 debug/release 保持原 tooltip。
- 脚本要求明确传入该 PID 和实际测试目录，校验 EXE 绝对路径、进程启动时间以及数据库位于 `artifacts/`。每次动作再次校验 PID/启动时间/路径，防止 PID 复用。按 EXE 路径排除 release，不硬编码某次用户进程 PID；记录其他既存 FluxDL 进程前后仍存活的证据，不停止它们。
- 精确 PID tooltip 可访问时，它证明显式 debug 测试 gate 生效。UIA 不暴露 tooltip 时，控制器须在测试目录写入下面的 `native-smoke-target.json`，声明它启动的进程确实使用了该 gate 和隔离 WebView profile；脚本核对各字段及精确启动时间。该证明只能由拥有本次启动信息的控制器生成，不能由脚本凭猜测补齐。脚本不读取进程环境内存。
- 测试实例无未保存草稿、`closeBehavior=tray`。控制器可先创建慢速原创媒体下载，供退出时验证引擎确实停止；脚本自身不修改设置或生成下载任务。仅打开选择目录对话框后取消，不选目录、不读写剪贴板、不打开播放器或 Explorer。

控制器证明格式（`processStartTimeUtc` 使用同一进程 `Get-Process.StartTime.ToUniversalTime().ToString('o')`，保留完整精度）：

```json
{
  "schemaVersion": 1,
  "processId": 12345,
  "processStartTimeUtc": "2026-11-09T00:00:00.1234567Z",
  "debugExecutable": "E:\\Software Development\\FluxDL\\src-tauri\\target\\debug\\fluxdl.exe",
  "testDataDir": "E:\\Software Development\\FluxDL\\artifacts\\native-example",
  "testGate": true
}
```

阶段 4 整编后，主测试控制器可在拥有的 debug 实例仍运行时执行：

```powershell
powershell.exe -NoProfile -NonInteractive -File .\scripts\native-smoke.ps1 `
  -TargetProcessId <自有debug进程ID> `
  -TestDataDir '<本次artifacts\e2e-data\runId>' -ProbeOnly
```

ProbeOnly 只读检查，输出 JSON，不操作窗口或写报告。确认隔离条件及唯一可用托盘目标后才运行交互：

```powershell
powershell.exe -NoProfile -NonInteractive -File .\scripts\native-smoke.ps1 `
  -TargetProcessId <自有debug进程ID> `
  -TestDataDir '<本次artifacts\e2e-data\runId>' -AllowExit
```

`AllowExit` 明确允许最后通过目标托盘菜单安全退出测试实例；不提供时该项标记未测。**不执行进程终止、taskkill 或模糊名称匹配**。报告保存在所传隔离目录的 `native-smoke.json`。

若 WebView 的 HTML 控件或托盘 UIA 不可用，可单独验证原生 picker：由拥有该 debug 实例的 CDP 控制器调用 `plugin:dialog|open`，参数 `options` 为 `{ directory: true, multiple: false, title: 'FluxDL Native Smoke <实际pid>' }`，保留 Promise；然后运行：

```powershell
powershell.exe -NoProfile -NonInteractive -File .\scripts\native-smoke.ps1 `
  -TargetProcessId <自有debug进程ID> `
  -TestDataDir '<本次artifacts\e2e-data\runId>' -CancelDialogOnly
```

此模式只取消同 PID、精确唯一标题的原生对话框，不触碰托盘，不要求托盘 marker 可访问。CDP 控制器还须断言原 Promise 返回 `null`，两者合在一起才证明原生选择器取消链路通过；单独脚本报告只能证明对应原生窗口已关闭。不能用它替代完整托盘验收。

## 计划记录的原生行为

1. 从本进程 Workspace 的 UIAutomation 树点击设置和目录选择器，查找同 PID、正确标题和 `#32770` 类的原生目录窗口，点击其 IDCANCEL（或仅对该原生窗口发 WM_CLOSE）。确认对话框消失且设置草稿值未变化。
2. 最小化目标 Workspace，通过下面的物理命中或已注册回调路径打开真实托盘菜单，在同 PID 的原生 `#32768` 菜单中选择“打开工作台”，确认窗口恢复。
3. 在真实托盘菜单选择“快速捕获”，确认独立原生窗口可见；原生 WM_CLOSE 后隐藏，进程继续存在。
4. Workspace 原生关闭进入后台，再通过左键路径恢复窗口。活动下载出现关闭提示时，仅调用自有窗口内精确“在后台继续”按钮；UIA 不可访问则这一项标记未测。
5. 真实托盘菜单“退出 FluxDL（停止下载）”使目标进程正常退出；没有强杀回退。退出前若发现唯一存活的直接子进程 `yt-dlp.exe`，用 `OpenProcess(SYNCHRONIZE)` 固定内核句柄，复核父 PID/创建时间并确认句柄未触发；退出后等待同一句柄触发，单独记录 `engineStopped`。没有可确认的运行引擎时，该项为未测，应用退出本身仍独立记录。

## 托盘的两条原生路径

- `physicalPointerHit`：系统 SendInput 发送前，同时要求 tooltip 完全匹配、可见矩形和 UIAutomation hit-test 对应同一标记。操作后恢复游标。
- `registeredTrayCallback`：仅在锁定 `tray-icon 0.25.1` 时启用后备。枚举目标 PID 的 `tray_icon_app` 隐藏窗口，对 `uID=1..16` 只读调用 `Shell_NotifyIconGetRect(HWND,uID)`，要求恰好一个 `S_OK` 且有非空矩形的已注册图标；发送前重新检查。只向这个自有 HWND 投递该版本的回调消息 `6002`，`wParam=uID`，`lParam` 只能是 `WM_LBUTTONUP=514` 或 `WM_RBUTTONUP=517`。它进入真实 tray-icon 事件/原生菜单链路，但**不能证明物理指针命中**。报告保留实际使用的路径。

后备依据是本机 Cargo 锁定源码 `tray-icon-0.25.1/src/platform_impl/windows/mod.rs`：`WM_USER_TRAYICON=6002`；注册 `(HWND,uID)`、未使用 GUID 或 `NIM_SETVERSION`；右键分支调用 `TrackPopupMenu`，左键分支产生应用订阅事件。不会发送该 crate 的 `6003/6004/6006` 等含 Rust 指针的私有消息。版本升级必须重新审查，否则自动禁用后备。

菜单只搜索目标 PID 的 `#32768` 窗口及精确菜单项，普通控件只搜索自有顶层窗口内部。菜单 UIA 不可访问时标记未测，并尝试向已确认属于目标 PID 的 tray 菜单 owner 发送公开 `WM_CANCELMODE` 结束该菜单；不猜 command ID、不发全局按键。

## 无法安全操作时

没有测试 gate 证明、找不到唯一可验证图标、原生菜单不属于该 PID、WebView 控件未暴露 Invoke/ValuePattern 时，记录 `unmeasured` 或整体阻断。不猜坐标、不点击普通 FluxDL 图标、不展开其他系统界面。物理命中不可用时只能采用上述受限回调后备，不能据此宣称物理命中通过。

取证只记录自有 PID、顶层窗口标题/类、目标按钮操作和断言结果；失败时也不导出整个桌面或文件对话框目录树/文件列表，不保留路径编辑框内容。

历史只读探测（阶段 4 整编前）：Windows PowerShell 可加载 UIAutomationClient，能找到 Shell_TrayWnd。当时 UIA 仅暴露 4 个结构性后代（`#32770`、`TrayNotifyWnd`、`ReBarWindow32`、`MSTaskSwWClass`），未发现可访问的 FluxDL/隐藏图标按钮；当时的 EXE 尚无 PID tooltip，因此没有尝试原生交互。整编后的自有实例须重新探测，不能沿用历史结果判断当前可用性。

脚本的 `passed` 只来自实际完成的原生断言；`unmeasured` 不算通过。退出码 0 表示已执行检查未发现失败，仍须查看未测项；1 为断言/执行失败，2 为安全门或能力限制导致整体未执行。失败时保留测试进程供排查，不自动结束它。

参考：[UIAutomation 测试模型](https://learn.microsoft.com/en-us/dotnet/framework/ui-automation/using-ui-automation-for-automated-testing)、[PostMessage](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-postmessagew)、[Shell_NotifyIconGetRect](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyicongetrect)、[WM_CANCELMODE](https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-cancelmode)、[进程同步句柄](https://learn.microsoft.com/en-us/windows/win32/procthread/synchronizing-with-processes)。

## 阶段 4 实测记录

2026-11-09 在本机 Windows 11 x64 / Windows PowerShell 5.1 / WebView2 上实际运行独立控制器，runId 为 `2026-11-09T06-02-08-961Z-a4eed156`，使用当时含阶段 4 未提交修改的 debug EXE。命令入口如下，引擎路径由调用者明确提供：

```powershell
node scripts/native-desktop-smoke.mjs C:\path\to\yt-dlp.exe
```

控制器结果为 **13 passed / 0 failed / 1 unmeasured，partial**，前端运行时异常为 0。脱敏摘要见 [native-smoke-stage4.json](native-smoke-stage4.json)，仅保存结果、计数和原始证据 SHA256，不包含本机工具、数据或输出绝对路径。原始报告及两阶段 helper JSON/stdout/stderr 留在本次 `artifacts/native-smoke/<runId>/`，不纳入版本控制。

| 实测范围 | 结果与证据边界 |
| --- | --- |
| 启动与隔离 | 9322 启动前空闲；连接前复核监听者在自有 debug PID 的进程祖先链内。精确 .NET 启动时间及独立 SQLite/WebView profile 证明通过。 |
| 原生目录对话框 | CDP 调用真实 dialog 插件打开精确标题、同 PID 的 `#32770` 窗口；helper 点击 IDCANCEL，真实插件 Promise 返回 `null`。未选择目录、未读写剪贴板。 |
| 真实下载 | 原创本机 HTTP 媒体经实际 yt-dlp 下载；`native-slow` 为每 80ms 发送 4096B，进入 downloading 并实测 downloadedBytes=1024 后开始托盘测试。 |
| 托盘与原生菜单 | 三次右键及一次左键均通过 `registeredTrayCallback`；已注册自有图标经 `Shell_NotifyIconGetRect` 确认，实际原生菜单完成工作台恢复、快速捕获显示/关闭、后台隐藏/恢复。未证明物理指针命中。 |
| 原生退出与进程停止 | 实际托盘退出菜单使自有应用正常退出，exitCode=0。退出前固定唯一存活 yt-dlp 子进程的 SYNCHRONIZE 内核句柄，退出后同一句柄触发，`engineStopped=passed`。 |
| 用户实例及清理 | 1 个既有 FluxDL 实例 PID/启动时间前后一致；测试未使用强制终止，自有应用已退出、9322 无监听、HTTP 服务器已关闭。 |
| 未测项 | helper 从 WebView 设置按钮打开目录选择器并检查草稿值不变的路径，因所需 scoped UIA 控件不可访问或被禁用而 unmeasured。独立原生 dialog 取消通过不能替代这条 UI 入口断言。 |

两阶段 helper 分别为目录取消 **1 passed**、完整托盘 **5 passed / 1 unmeasured**；控制器额外校验隔离、插件 Promise、真实下载和退出证据后得到上述 13/0/1。计数包含分层断言，不代表 13 种独立交互。此次仅覆盖当前本机能力，不证明多显示器、高 DPI、物理托盘点击或所有平台行为。此前只读探测保留为历史背景，不作为此次交互结论。

原始控制器报告 SHA256：`7c0d8ffbf679e120439eb931eaad079a0e2dcfa3552afe05b62095cde2fd70cb`。本次运行后仅补充摘要在早期能力限制时保留已记录 tray route 的取证回退，随后语法与 ESLint 复核；没有为该报告展示改动重复执行原生交互。

## 阶段 6 最终复测

2026-11-09 在根任务完成新 debug 的 15 项真实桌面回归后，明确授权以相同控制器再次运行。此次使用含修复后进度模板的新 debug，runId 为 `2026-11-09T06-45-23-602Z-20f2a6d9`。结果独立保存在 [native-smoke-final.json](native-smoke-final.json)，阶段 4 摘要及原始 run 目录均保留，没有覆盖其历史结论。

| 运行 | 结果 | 错误与清理 | 证据 |
| --- | --- | --- | --- |
| 阶段 6 最终原生复测 | **13 passed / 0 failed / 1 unmeasured，partial** | errors=0、runtimeErrors=0；appExitCode=0；engineStopped=passed；forcedOwnedTermination=false；mediaServerClosed=true；1 个既有实例身份前后一致 | 原始报告 SHA256 `5739eabb5d3875e93e7bb5362c86d5353021564aa1c6c10c1433c7ea4eab4679` |

本次再次实证：精确自有 PID/启动时间及 CDP 监听祖先链通过；原生 IDCANCEL 使真实 dialog 插件 Promise 返回 `null`；实际 yt-dlp 下载进入 downloading，开始托盘测试前 downloadedBytes=1024。原生托盘工作台恢复、Quick Capture 显示/关闭、后台隐藏/恢复、真实菜单退出通过；仍仅使用 `registeredTrayCallback`，未证明物理指针命中。退出前存活引擎的固定内核句柄在退出后触发，证明该次运行的引擎已经停止。

唯一未测项仍是 WebView 设置入口的 scoped UIA 控件不可访问或被禁用，不能将独立 dialog 取消结果替代为这条 UI 入口通过。最终只读核对自有实例已退出、9322 无监听、用户既有实例仍以原启动时间运行。原始本机取证留在对应 `artifacts/native-smoke/<runId>/`；发布摘要只含结果、计数、限制与证据散列，不包含本机绝对路径。

## 页脚版本修复后的最终复测

根任务修正工作台页脚遗留版本文本并重新构建 debug 后，于 2026-11-09 明确授权再次执行同一原生流程，增加一项真实 DOM 版本元数据抽检；runId 为 `2026-11-09T06-49-11-558Z-1cb1e8b5`。控制器从 `package.json` 读取期望版本，实际观测如下；设置页仅临时打开读取关于文本，随后恢复原导航页，没有改动设置草稿。

| 观测项 | 期望与实际 |
| --- | --- |
| 完整 package 版本 | `0.2.0` |
| 工作台真实页脚 | `v0.2`，与 package major.minor 一致 |
| 设置关于页真实文本 | `FluxDL 0.2.0 · MIT License`，与完整 package 版本一致 |
| 全流程结果 | **14 passed / 0 failed / 1 unmeasured，partial**；errors=0、runtimeErrors=0 |

独立摘要为 [native-smoke-final-version.json](native-smoke-final-version.json)，此前 stage4 和 final 两份 13/0/1 摘要已核对散列并保留。原生对话框取消、真实 downloading/bytes>0、已注册托盘回调与菜单、退出前后固定引擎内核句柄验证均再次通过；唯一 UIA 设置入口未测项及不能证明物理指针命中的边界不变。自有应用正常退出 exitCode=0，引擎已停止，无强制终止，HTTP 服务器关闭、9322 无监听，1 个用户既有实例仍保持原 PID/启动时间。

本次原始报告 SHA256：`dede48a96a76bd6a184a8325f5a0ca91dc65cb21800cbe872ef0d5ea68b7b6c8`；path-free 摘要 SHA256：`40868df7770411777017dc37447d0a1476d3e566a7574e4527f918da661f538c`。此记录仅针对上述新 debug 实际运行，不替代安装器安装/卸载或其他硬件环境验收。
