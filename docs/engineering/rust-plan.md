# 阶段 5 Rust 渐进整理方案

状态：**阶段 5 已实施，最终集成门禁由根协调**。阶段 2、3、4 分别验收后执行。基线见 [0.2.0 审计](0.2.0-audit.md)，本轮不 commit/push、不扩大 capability、不新增产品功能。

Engine/Process/纯 transitions/错误与窗口装配由 Engine 子任务实现；Manager/Persistence 由 Queue 子任务接入。下列为已经存在的内部职责，内部 Rust 导出不作为稳定 SDK；兼容范围是既有 IPC 命令、成功响应与缺省 String 失败。

## 保留的边界

- Rust 内存任务源仍唯一权威，SQLite 负责恢复，前端只展示。默认一次一个任务；不加入并发、暂停或自动恢复。
- 原有 command 名称、成功响应、camelCase JSON、lowercase TaskStatus 字符串保持兼容。缺省失败仍 String；新前端统一请求 `errorFormat: "structured"`，失败为 `{version:1,code,phase,message,details?}`，details 缺省不序列化为 null。
- 不向公开 `DownloadTask` 强塞 attempt/数据库版本字段。阶段 3 计划的存储 `mutation_seq`、运行 attempt，以及阶段 4 的 IPC session/sequence 各自承担不同职责。
- SQLite mutation sequence 负责写入新旧和删除 tombstone；IPC sequence 只计算实际发布的协议事件，不能把被合并的每个进度写入都计为事件，否则前端会误判缺口。
- 保留参数数组、`--ignore-config`、单视频限制、FFmpeg 提前诊断、媒体扩展白名单、最终目录校验与 Windows Job。
- 保留显式 debug smoke 的用户实例隔离 gate；release 的单实例与窗口权限不变。不能为测试停止用户已有的 FluxDL 实例。

## 按职责逐次提取

| 位置 | 提取后的职责 | 明确不承担 |
| --- | --- | --- |
| `engine/mod.rs` | 保留旧函数入口的 re-export；站点引擎适配 | 队列状态、SQL、Tauri 事件 |
| `engine/arguments.rs` | 纯参数数组与格式/依赖校验 | 启动进程或读取设置文件 |
| `engine/metadata.rs`、`progress.rs`、`download.rs` | 元数据/标记协议解析，具体下载适配器输出 typed facts 与脱敏日志 | 修改 Manager 或保存任务 |
| `engine/discovery.rs` | 候选路径、版本/依赖探测 | 自动安装或更新依赖 |
| `errors.rs`、`diagnostics.rs` | 源操作错误分类、IPC 失败映射、敏感信息过滤与安全展示文本 | 从中文字符串反推类别、记录原始凭据 |
| `process/{capture,lines,download}.rs`、`process_job.rs` | 具体子进程所有权、capture/line reader、Job、退出/取消/回收 | yt-dlp 参数含义、任务终态、数据库写入 |
| `transitions.rs` | 纯状态转换、attempt/阶段守卫、进度应用结果 | 锁、进程、SQL、时钟与事件发送 |
| `commands` | 原有受控命令适配及 String 错误边界 | 业务状态规则和下载循环 |
| `lib.rs` | Tauri 装配、初始化及窗口生命周期注册 | 所有 command 实现细节 |

已按职责提取，保留必要 Engine 入口重导出；没有引入 trait 家族、通用消息总线或另一套任务仓库。`engine.rs` 已移为 `engine/mod.rs`；所有 command 实现位于 `commands.rs`。

实施顺序是 errors/diagnostics 与 Engine 纯函数、Process、transitions 接入 Manager、commands/lib/windows。Windows Job 仅保留 `process_job.rs` 一份，Manager 不持有 child/Job/readers。

## 集中状态转换与终态规则

转换函数接收现有状态、明确意图及必要上下文，返回“已变化 / 幂等无变化 / 拒绝”以及需要持久化/发布的变更；不自行执行 I/O。非法转换不分配新版本、不发伪变化事件。

| 输入 | 合法状态/结果 |
| --- | --- |
| Claim | Queued → Resolving；成功认领时建立新 AttemptId 与 Control |
| Download progress | 当前 attempt 的 Resolving/Downloading → Downloading；阶段守卫见下节 |
| Postprocess | 当前 attempt 的 Resolving/Downloading/Postprocessing → Postprocessing；清百分比、速度和 ETA |
| RequestCancel | Queued → Cancelled；运行阶段 → Cancelling；Cancelling/终态重复请求无副作用 |
| Process finished | 当前 attempt；未接受取消时由成功退出 + 有效最终文件决定 Completed，否则 Failed |
| Cancellation settled | 已接受取消且进程已停止 → Cancelled；停止/清理失败则 Failed 并保留可理解原因 |
| Retry | Failed/Cancelled/Interrupted → Queued；清运行字段和旧错误，保留任务 ID、输入选项和原创建时间，移动队尾 |
| Startup recovery | 数据库中残留 active（含 Queued/Cancelling）→ Interrupted；不自动开始下载 |

取消和完成同时发生时，以状态锁内的意图/终态决策为准：取消先被接受则按取消结算；终态 candidate 先决定则晚取消不翻转。candidate 的终态在 durable ack 后才公开，等待 ack 时保持旧公开状态。即使留下真实文件也不删除该文件。

Completed/Failed/Cancelled/Interrupted 之后的普通进度、重复退出和重复清理不再改变终态；显式 Retry 是离开可重试终态的唯一入口。取消不意味着可以立即复用仍有进程的任务，必须等待旧运行实例停止并回收。

关键终态要沿用阶段 3 的持久化确认机制，不在数据库失败时先向前端广播“已可靠完成”。删除历史仍只删记录，不删本地媒体。shutdown 要等待进程回收和 writer flush，并报告超时/失败；恢复窗口不应被误解为 worker 已恢复接单。

## Attempt、顺序与进度流

内部 attempt 为 `u64`：worker 成功 claim 的持久化 ack 后，在同一个内存提交锁中发布 Resolving 和新 attempt，再放入 Control。Manager **先验证 expected attempt、running 状态、取消及 pending terminal，再应用状态和分配持久化版本**。重试排队期间也拒绝旧 attempt，旧行、退出和清理结果不能污染重试。

同一 task ID 的重试不是同一运行实例。前端的 IPC sequence 不能替代后端 attempt 守卫：旧进程回调若先被 Manager 当成新变化接收，事件自然会获得新 sequence，前端无从判断来源已经过期。

必须区分三类情况：

1. **过期事件**：旧 attempt、旧行序号、旧 IPC sequence，或已知终态之后的普通进度。直接拒绝，不使状态、字节或百分比倒退。
2. **合法新下载流**：分离视频/音频切换，已下载文件的 filename/format 标识发生明确变化，或上一流 finished 后开始另一明确流。新 stream epoch 可从低字节/低百分比开始；这是真实引擎进度，不能用 `max(percent)` 卡在上一流的 100%。
3. **估计变化或不明回退**：total_bytes_estimate 被修正时，同样字节数的百分比可以降低；不能单凭百分比下降判定乱序。相同 epoch 的字节下降且没有明确新流/重启信号时，记录诊断并采用明确的未知/异常处理，不能伪造一个整体百分比。

Process 保留各输出流的行序号、stdout/stderr 身份、截断和读取故障；Engine 适配器输出解析好的 facts 和每路独立 stateful 脱敏日志。模板显式输出官方 `info.format_id`、`progress.filename/status/bytes/total/speed/eta`，避免输出整个 progress 字典。Tracker 使用当前身份及最多 32 个 retired 身份，拒绝 A→B 后跨输出流迟到的旧 A。明确新流重置走完整 TaskUpsert，刷新发布日志基线；普通 progress patch 不允许 Postprocessing 倒退。未知身份不能证明新流时保守拒绝回退，字节回退首条产生可见诊断，不虚构总体百分比。

Postprocessing 后同一 stream 的迟到下载进度应拒绝；只有明确的后续下载流信号才能按实际引擎行为进入另一流。阶段变更和 stream epoch 是内部元信息，不要求为此改版界面或数据库中旧字段含义。

## 进程提取的最小合同

使用具体结构和有界通道，不要求新的抽象 trait。启动描述只含已验证的 executable、args 数组、工作目录及超时/输出限额，不接受 shell 字符串。

- capture：保留退出码、stdout/stderr、各路截断标志和读取错误；独立 owner 任务不随 IPC waiter 丢弃而消失。版本/解析超时终止并等待；读取故障通过 try_join 提前结束并回收子进程，保留 ProcessRead 源分类，清理失败附加安全原因。
- 下载 runner：独立 owner 持有 child、Job、reader handles；EngineDownload 仅消费 typed 通道。Cancellation 的 flag+Notify 等待先注册再检查状态，满下游通道也不能丢取消；丢弃 handle/finish waiter 请求取消但不会 abort owner。正常退出、取消和错误分别返回真实结果。
- 停止失败时 owner 继续隔离原 child，不清 PID、不关闭事件流、不让 Manager 结算并 claim 下个任务。每轮停止有界，独立 1 秒重试 timer 和重复取消均能推进；相同原因只交付一次诊断。诊断保留为单一 pending 项，由有容量的通道成功交付后才去重，背压不阻止 kill/reap。只有真实 wait 成功才释放；此等待在系统持续拒绝清理时可持续存在，应用的 shutdown 总 deadline 仍会明确失败并恢复窗口，允许重试退出，而非伪称清理完成。
- Engine adapter 负责将行解析为进度/后处理/最终路径/脱敏日志；Manager 仅协调转换、持久化与事件，不再逐字节读管道。
- reader 的读取错误必须可见，不能 `Err(_) => break`；超过单行限制应有截断诊断，继续消费管道避免死锁。
- 退出后的管道 drain 必须有界，超限产生诊断；reader abort 后等待 handle 回收。关闭 Job、父进程退出和终态提交各自只认领一次。
- Job 创建/加入失败对存活进程仍保守拒绝；极快退出程序先确认句柄已结束。不能把清理退出码必须非零作为正确性的条件。

## 内部错误与 IPC 兼容

`AppError` 已提供稳定 `ErrorCode`、`ErrorPhase`、安全 message 与可选限长 details，均在源操作边界构造。没有泛用 `From<String>` 或按中文猜 code；Writer 保留 Rejected/Unconfirmed 分类及其 typed 源错误。Engine 非零退出在安全 details 保留 exit code/phase；友好提示只检查实际 ERROR 行或最后非空行，普通 warning 不覆盖真实错误类别。

类别覆盖输入、状态/任务不存在、关闭准入、依赖、进程启动/超时/读取/等待/清理/退出、引擎协议、输出校验、持久化拒绝/未确认、窗口和系统打开。沿用标准库 Display/Error，不建立 trait 层级。

21 个 command 均接受可选 `errorFormat`；无值返回原 String 失败，structured 返回 v1 DTO。成功值及 command 名保持不变。前端 runtime guard 同时兼容旧 String/Error，新 DTO 的 UI 只展示 message。Task.error 和 shutdown-failed 沿用 String，通过 safe_text 保留安全原因；退出失败不再统一抹成“停止下载失败”。tray、single-instance、窗口 show/hide/emit 及几何查询错误记录有界安全诊断后执行必要 fallback，不静默丢弃。

所有前端可见日志、元数据 warning、引擎检测失败和友好错误走同一过滤入口。`LogRedactor` 按 stdout/stderr 保留 Header 续行状态；超长行整体省略并保守保护后续缩进续行。旧数据库记录仅在内存展示副本过滤，原始未知/终态行不因读取被改写；正常产品路径设置继续展示。

## 脱敏分类与测试

| 输入类别 | 输出要求 |
| --- | --- |
| HTTP/HTTPS/代理 URL | 移除 username/password、query、fragment；保留有用 host/path，格式无法安全解析时保守隐藏该 URL |
| socks/socks5/http 代理认证 | 同样移除凭据；不能仅寻找 `http://` 字面量 |
| Authorization、Proxy-Authorization | 不保留 Bearer/Basic/token 内容，Header 名可保留 |
| Cookie、Set-Cookie | 整个敏感值隐藏，不逐个猜测 cookie 名 |
| 命令参数或错误中的令牌 | 不输出原始含认证参数；参数诊断只输出允许的安全阶段/选项摘要 |
| ANSI/控制字符/过长输出 | 清理控制序列并限制长度；截断后仍不能留下凭据片段 |

不要用已知失败的脱敏结果作为测试预期。先增加“秘密不得出现”的回归并在同一变更修复。大小写 Header、多行/引号/JSON 片段、多个 URL、IPv6/Unicode 路径及凭据百分号编码均应有代表样例；避免只按例子词语做字符串替换。

## 验证切片及交接

阶段 1 旧结构基线为 Rust 23 passed/1 ignored。阶段 5 第一轮接入为 68 passed/1 ignored；Queue 新 Manager/Writer 测试接入后为 70 passed/1 ignored，真实 fixture 通过。Process 相关 6 项测试已通过：读取故障不被 hanging wait/timeout 掩盖、行截断/故障可见、实际满通道取消、丢弃 finish waiter 的 owner 清理、注入一次/多次清理失败（含满通道诊断）后通过固定 OS 句柄确认仍隔离与恢复后退出。此前 all-targets strict Clippy 已通过；后续新增隔离测试及最终全量结果以根门禁记录为准，不将未执行项目写成通过。

阶段 5 必须补的行为测试：

- 状态转换表及非法边；重复 cancel/finish/cleanup 不新增终态。
- cancel 与 successful exit 两种先后次序；清理失败不伪装 Cancelled。
- retry 后旧 attempt 的日志/进度/退出被拒绝；重试加入队尾。
- 迟到 progress 不覆盖终态；新流允许 reset；估计总大小变化不被误当乱序。
- 非对象 `__FLUX_PROGRESS__null/[]/42` 必须拒绝，不能变成伪造零字节 Downloading。
- stderr/stdout 同时大输出、单行截断、读取故障、退出后继承管道不关闭、超时和进程树清理。
- Cookie/Header/多协议代理及友好错误/metadata warning 的秘密不外泄。
- 原命令名、String 错误、旧 lowercase 状态 JSON、缺 FFmpeg 和未知分辨率预设保持兼容。

每个提取切片跑相关 Rust 单测；过程完成跑 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、默认 Rust tests 和 `scripts/test-queue.ps1`。最终再由根统一 debug 桌面 smoke 与 Tauri 构建；与用户已启动的 release 实例隔离，不重跑不必要的公网下载。

最终交接保持阶段 3 writer/mutation_seq 与阶段 4 envelope 不变；attempt/stream tracker 仅后端运行时元数据。保留 smoke gate、权限、发布配置；原生 registered callback 与 physical pointer hit 的验证范围继续分别记录在 native-smoke 文档。

最终门禁：0.2.0全部80项Rust通过、1默认ignored由worker fixture单独通过（5.17s），严格all-targets Clippy/fmt通过。首轮真实桌面暴露逐字段j输出裸NA，已改有界对象一次序列化并兼容dotted/flat；相关Engine17项及真实快慢29条进度记录通过。重建后15项桌面通过，最终版本原生14通过/1未测。最终报告与产物指纹见0.2.0-validation-report.md/build-final.json，不将早期68/70/76/79项的中间结果冒作最终覆盖。
