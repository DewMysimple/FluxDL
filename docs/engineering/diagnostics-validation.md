# 可观测性迭代验收

基线main/07343339797825e832a34488d621c15f0136f491，2026-11-09 Windows11 x64、Node24.16.0、Rust1.97.0 MSVC、WebView2。实现以本轮源码为准，不沿用旧测试结论。任务书归档在prompt/FluxDL_Codex_Observability_Diagnostics_Iteration.md。

## 已执行验证

| 命令 / 范围 | 实际结果 |
| --- | --- |
| npm run typecheck / lint / format:check | 通过 |
| npm test | 97通过，1项既有性能benchmark默认跳过 |
| npm run check:rust / format:rust / lint:rust | 通过，Clippy --all-targets -D warnings |
| npm run test:rust | 101通过，1项worker fixture按原规则ignored；main/doc tests无失败 |
| scripts/test-queue.ps1 | 显式worker fixture通过，4.83秒；不写默认dev-logs |
| npm run test:diagnostics | 24项相关unit通过，加5次/组生产管线fixture、异常场景、30,000事件查询测试通过 |
| npm run benchmark:diagnostics | 20次/组，实际n/p50/p95/max及writer开关对照；细节见performance-analysis.md |
| npm run test:release | 34通过，0失败，不改发布策略 |
| npm run build / npx tauri build --debug --no-bundle | 真实构建成功，非静态页面冒充桌面 |
| scripts/diagnostics-smoke.mjs 本地真实yt-dlp | 19项通过、0 JS错误；包含5次真实原创HTTP媒体下载、UI关联与独立ZIP CRC检查 |
| scripts/desktop-smoke.mjs | 原有Quick Capture、串行队列、取消/重试/HTTP失败、主题、文件状态、历史、真实重启和强退恢复通过 |
| scripts/native-desktop-smoke.mjs | 14通过、0失败、1未测，脚本以partial/exit1如实报告；具体见下文 |
| npm run test:styles | 12项既有组件CSS等值通过；新增诊断导航不在旧CSS基线内，明确从两张比较图DOM移除，新增诊断UI由实际桌面深浅主题单独验收 |

本轮正式release构建与最终Git/产物状态在收尾发布后补记；上表不把此前release-output的旧包当成本轮程序。

## 逐项功能验收

| 要求 | 当前证据 |
| --- | --- |
| 四视图、真实统计、空状态 | DiagnosticsPage + 前端组件测试；真实WebView2导航切换、深浅主题；浏览器无伪造采样 |
| 任务→Trace→完整阶段 | UI首次解析/入队有related_trace_id，frontend计时与Rust事件可找到，单次attempt一致；真实命令查询核对复核/DB/queue/spawn/progress/terminal |
| 时间/等级/来源/任务/Trace/关键词 | Filter严格验证及分页单测、桌面实际事件搜索；trace只作UUID过滤，不转文件路径 |
| 性能瀑布、慢Trace排序 | Rust单调时点与开始/结束，前端时钟独立；后端20,000窗口计算最慢100 Trace，不累加嵌套跨度；unit验证排序和nearest-rank |
| 实际日志目录、自动报告 | 正常debug真实创建工程根dev-logs；运行中、主动refresh和退出保留、重新启动跨session查询均实际验证 |
| 导出及定位 | 真实Rust原生save dialog由明确用户操作触发；本轮自有helper确认，生成6条目ZIP，Python独立CRC/JSON/虚构敏感值检查；Explorer目录打开及本次ZIP选中均由系统Shell窗口实际核对 |
| 双窗与权限 | 重启后两个WebView可查旧Trace；Quick对目录/导出commands被拒，原capabilities完全不变 |
| 真实状态与缺失进度 | TaskDiagnosticStatus只从事件描述阶段；fixture无进度时不填耗时，后处理没有信号是不适用；取消/完成与旧attempt准入沿用原99项Rust及worker保护 |

## 可靠性、安全与查询优化

并发8线程写入、完整JSONL、重启session隔离、10MiB/5文件/7天策略（测试用小阈值和过期时间）、未知文件保留、损坏JSON、未来schema、重复event_id、截断尾行均有测试。Windows实际FileShare.None占用报告、只读属性、目录被文件阻挡及junction逃逸都通过；磁盘不足用Write故障注入，未物理填满用户磁盘。生产Manager在日志根不可写时仍完成真实fixture下载。

普通写队列有界，关键保留队列满时从有界内存尽力补写；600条错误压力测试全部可读取，受限次数仍保留提示。不能承诺磁盘永久失败或异常断电零丢失。默认不记录每帧进度。所有磁盘写在独立线程；查询spawn_blocking，不持任务状态锁。

30,000条合成事件只存ignored独立fixture目录，用于查询压力，不参与真实解析统计或默认dev-logs。测量发现重复读取约626–644ms，才加入最多20,000条/5秒TTL的已净化查询缓存。命中、过期、内存generation/文件变化、8并发读、持读锁仍能发事件、导出强制重读均测试。冷读仍有磁盘开销，不把缓存命中速度当下载速度。

伪Cookie/token/sig URL、代理密码、Windows用户名/路径、标题等字符串在入库、查询、Markdown和ZIP均做回归。新事件白名单拒绝任意对象/未知字段；原始stderr不落盘，普通引擎技术日志也仅固定摘要，错误保留code/kind与退出码。旧业务数据库不清空、不收集、不随诊断重写；历史本机原文不是可自动分享的资料。实际目录路径只在本机状态栏可查看，未写入报告/环境JSON或事件。

测试发现早期Manager::init(None)会让unit/fixture写默认debug目录，已改为显式disabled/fixture诊断服务；这批本轮生成的旧测试日志按严格路径和文件名验证后保留到ignored artifacts，未删除用户文件。最终默认dev-logs由真实桌面采样重新生成。

## 平台边界与未验证项

原生托盘补测覆盖真实注册菜单/callback、最小化恢复、Quick开关、后台、正在运行引擎的退出与真实句柄回收；14项通过。设置页原生目录按钮的UIAutomation控件不可用，1项未测，因此脚本partial/exit1；另一路真实目录picker Cancel已通过，不能替代未命中的设置入口。该限制与本轮新增诊断导出save dialog通过是不同范围。托盘callback不等于物理鼠标命中。

Windows10、真实多屏/125%及150%系统DPI、完整NSIS安装卸载、发行EXE直接运行、外网YouTube/站点测量、远程CI未新增验证。真实媒体联调使用用户已有yt-dlp 2026.07.04与原创loopback HTTP素材，不静默下载引擎、不改证书/代理、不以一次网络结果声称加速。正式路径由Release编译分支的Tauri app_log_dir取得，不能用debug路径测试假称正式运行已验证。

后续复现：Windows隔离测试系统执行README和docs/diagnostics.md命令；性能前后保持同媒体/引擎/网络/缓存条件。源码/文档可提交，dev-logs、所有真实报告/事件、数据库、媒体、诊断ZIP和发行物不得推送。
