# 可观测性现状审计

审计基线：2026-11-09，main / 07343339797825e832a34488d621c15f0136f491；工作树干净，实际 origin/main 相同。当前用户持续授权每轮修改提交、推送。

| 问题 | 当前证据与结论 |
| --- | --- |
| 解析入口 | `features/capture/CaptureInput.tsx` → `lib/api.ts` probe → `commands.rs::probe_media` → `Manager::probe` → `engine::probe` → `process::capture`。75秒超时，16MiB元数据上限，同时消费stdout/stderr。 |
| 入队复核 | `Manager::enqueue_owned`先验证模式/预设，再调用probe，核对playlist、格式ID，锁外准备目录，重建可信下载参数，SQLite确认后才发布queued。再次解析是反客户端伪造格式的安全措施，不能删除。 |
| 调度与运行 | `claim_task`先持久化Resolving及attempt，建立Control；`run_process`返回handle时实际spawn尚在独立owner内。必须在`process/download.rs::run`的真正spawn处计时，不能把handle创建当启动。 |
| 首次进度 | `engine/download.rs`解析typed fact，`transitions.rs`按attempt、stream sequence、阶段防倒退；只有被接受的download fact才可计首次有效进度。FFmpeg标记是后处理证据，不等于字节到达。 |
| 终态 | owner回收child和reader后，校验真实输出，再建立终态候选；`commit_terminal`确认SQLite后公开。取消先发信号；必须保留这些顺序。 |
| IPC | `events.rs`和stores提供session/revision、有界增量、约250ms节流、6秒checkpoint；新增诊断通知不能占用或破坏该版本空间。 |
| 既有日志 | `diagnostics.rs`仅有ANSI/Header/query等脱敏，任务100行缓存和eprintln；无JSONL、轮转、Trace、统计或报告。已有redactor保留URL路径及本地路径，因此不适合直接作为新诊断字段。 |
| 权限 | capabilities仅事件、拖动和原生open picker；无通用文件系统/shell。新导出由Rust发起可信save dialog，不向Quick Capture增加文件权限。 |
| 模式/产物 | debug_assertions与明确debug隔离目录已存在。开发日志从编译期CARGO_MANIFEST_DIR的父目录发现；正式只用app.path().app_log_dir()，无源码回退。release脚本白名单打包，新增dev-logs必须git/prettier/Vite/Tauri watcher排除。 |
| 可用测试 | manager worker fixture、engine_smoke、desktop-smoke.mjs、本地原创媒体HTTP服务、真实WebView2/CDP9322和隔离数据/profile。只管理本轮创建的进程，保留用户旧release和Vite实例。 |

实施顺序：①复用redactor模块并增加单一Rust诊断服务/类型白名单/有界writer与安全轮转测试；②沿真实上述边界加入Trace和单调span，解析与入队分trace、关联解析引用，retry独立attempt；③同一净化事件生成分页查询、报告和用户确认导出ZIP；④Workspace四视图、任务跳转和双窗定位；⑤确定性基准及真实本地yt-dlp测量；⑥仅依据测量决定优化；⑦全量回归、文档、记忆、提交推送。

风险控制：不记录媒体对象/标题/URL/命令行/stderr/路径；事件只接受枚举、UUID及有限数值。磁盘I/O独立线程，热路径只短内存操作和try_send。查询/导出重新严格解码文件，忽略损坏/未来schema并计数，不信任磁盘任意JSON。清理只针对可信根内命名严格且非链接的已知日志，保留未知文件及主动导出。

审计后继续实施；实际结果与未验证项将记入diagnostics-validation.md，不把计划写成通过。
