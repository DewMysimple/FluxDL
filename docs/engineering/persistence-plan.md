# 阶段 3 持久化实施检查点

状态：**方案已准备，尚未实施**。阶段 2 通过并获根任务授权后才修改生产代码。本轮不 commit/push。当前生产持久化仍是 v1 文档表和全量 `save`，测量依据见 [重构前基线](0.2.0-performance-baseline.md)；本页不代表新 API 已存在或验证通过。

## 选定的最小边界

- 沿用 rusqlite、现有两张文档表、DELETE journal、FULL synchronous 和 busy timeout，不预先改变 durability。
- 增加一个具体 repository 和一个专用标准线程 writer，不引入 trait 家族、通用事务框架或第二个运行时。连接在该线程内创建、使用和关闭。
- `Manager::Inner` 不再拥有 Connection/Persistence。任务状态锁只做验证、内存修改、版本分配及克隆单项变更；锁内没有 SQL、序列化、文件检查或阻塞等待。
- 一个独立的 `critical_gate` 串行化关键写入的准备/ack/提交，但不占用主状态锁。快照和普通进度仍能读取/修改其他任务。取消进程的信号不等待该 gate 或数据库。
- 保留现有公开任务 JSON 与 commands 的 `Result<T, String>`。存储序号、待确认变更、attempt 均为内部信息；持久化故障通过现有可读诊断与任务错误呈现。

## 拟实现的具体 API

```rust
// 均为内部类型；字段名在实际实现中遵循仓库风格。
type MutationSeq = i64;
type AttemptId = u64;

enum Mutation {
    UpsertTask { seq: MutationSeq, task: DownloadTask },
    DeleteTask { seq: MutationSeq, task_id: String },
    UpsertSettings { seq: MutationSeq, settings: Settings },
}

enum ApplyOutcome { Applied, AlreadyCurrent, Stale }
struct WriteReceipt { outcomes: Vec<ApplyOutcome> }
struct Loaded {
    settings: Settings,
    tasks: Vec<DownloadTask>,
    max_mutation_seq: MutationSeq,
    diagnostics: Vec<String>,
}

// Repository 的同步方法只允许 writer 线程调用；基准也可直接测量它。
Repository::open(data_dir) -> Result<(Repository, Loaded), PersistenceError>;
Repository::apply(&mut self, changes: &[Mutation]) -> Result<WriteReceipt, PersistenceError>;

// Writer 持有有界 Tokio mpsc Sender，标准线程使用 blocking_recv。
PersistenceWriter::open(data_dir).await -> Result<(PersistenceWriter, Loaded), PersistenceError>;
PersistenceWriter::apply(changes: Vec<Mutation>).await -> Result<WriteReceipt, PersistenceError>;
PersistenceWriter::flush().await -> Result<(), PersistenceError>;
PersistenceWriter::close().await -> Result<(), PersistenceError>;
```

`apply` 中全部变更使用一个事务，成功 COMMIT 后才发送 oneshot ack；失败返回明确错误，不发送成功 ack。`flush` 是排队屏障，确认此前已经提交的请求完成，不能替 Manager 提交尚在内存中的脏进度。线程退出或通道断开同样返回故障。未获 ack 的操作不能被当作失败回滚已知数据库之外的不可逆事实，也不能被当作成功。

不设置会任意丢弃已提交写入请求的普通 API timeout；SQLite 本身保留 busy timeout。关闭操作有整体 deadline，但不能通过取消等待 future 来假定 writer 请求已取消，见下文关闭协议。

## v1 → v2 迁移与原始数据

首先读取 `user_version` 和 sqlite_master/表结构，再作任何写操作。空新库在事务内建立 v2；版本 0 但具有可识别 v1 两表时按 legacy v1 迁移；版本 1 增补下面的版本表；版本 2 直接读取。负值、未来版本、缺失/不兼容的必需列均返回可读兼容错误，不能降级版本或新建空库。

保留 `app_settings(id, document)`、`tasks(id, document)` 的现有记录、附加列和未知表。只新增：

```sql
CREATE TABLE persistence_versions (
  kind TEXT NOT NULL,
  record_id TEXT NOT NULL,
  revision INTEGER NOT NULL,
  deleted INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (kind, record_id)
);
```

旧 settings/task 各建立 revision=0 的版本行；与 `PRAGMA user_version=2` 一同在迁移事务提交，失败完整回滚。当前查询按 ID 和启动全量读取，主键已经覆盖，不为尚未存在的分页查询增加索引。

- 正常任务 upsert 只更新该 ID 的 `document`；未知 JSON 属性递归保留。`media` 中的未知属性、格式条目中的未知属性也保留，格式按 `formatId` 对应，不能仅重新序列化 DTO 后覆盖全文。
- 设置采用相同的已知字段合并策略，未知字符串设置不在加载时擅自改为默认值。
- 设置 JSON 若无法安全解码，返回兼容性错误并保留原文，不将默认设置写回覆盖它；这不是物理损坏恢复的条件。
- 一行坏 JSON、缺失不可恢复必需字段、未知状态或 SQL ID/JSON ID 不一致时，保留原行原 `document`，继续加载可理解行并给出不含原始私人链接的诊断。该不兼容行不能被全量保存删除，也不能被同 ID 的自动写入覆盖。
- 不把未知状态强行解释成 Completed/Queued/Interrupted；阶段 3 可将该行作为保留的不可操作记录，而不改变公开 TaskStatus。将来若引入 Unknown 状态也必须保留原始值。
- `Option` 等真正可缺省的旧字段可用显式兼容解码；不能随意给 ID、来源 URL 或创建时间编造值。测试须列出接受与保留而不解释的边界。
- 仅 SQLite quick_check 报告损坏（包括该检测返回 SQLITE_CORRUPT/NOTADB）可进入原有“字节备份后恢复”分支。其他查询失败、serde 语义失败、未来 schema、权限、busy、磁盘空间不足不能触发该分支。备份失败时停止，不覆盖原文件。

加载后仅对可理解且处于 active 的旧任务批量更新为 Interrupted；仍不自动恢复 queued。不能再次在加载末尾全表 `save`。未变化的终态记录与未知行不重写，旧排序 `createdAt`、ID 次序保持。

## 序号、删除及进度合并

主状态锁内分配全局 `mutation_seq`，起点高于数据库所有版本行（包括 tombstone）的最大 revision；检查 i64 溢出，不能 wrap。锁外发送可能乱序，所以 repository 在同一事务中检查记录版本，低版本直接报告 Stale，不能覆盖新版本。相同 revision 的同一请求可幂等重试；同 revision 却有不同内容是冲突错误，不能假装 Applied。

删除时事务内只 DELETE 指定任务，并将版本表对应行更新为 `deleted=1`。保留该 tombstone，防止较旧 upsert 重新插入；不删除本地媒体。仅内存过滤旧事件不足以保证这一点。

进度先写运行时状态，每两秒从脏任务集合克隆最新一项变更，释放主锁后交 writer；不等待每个原始 stdout 行完成磁盘写入。通道拥堵时保留最新脏进度，下个时机重试，不无限堆积所有采样点。终态候选产生时停收该 attempt 的普通进度；关键变更不受普通节流吞掉。

`AttemptId` 在 worker 成功认领 queued → resolving 时递增并放进 Control。所有进程回调先检查 attempt，再分配 mutation 序号。retry 保留 ID、重新入队尾，旧 attempt 回调无效。IPC 的 session/sequence 只统计实际发布事件，与持久化序号分开，避免因进度合并产生假事件缺口；衔接见 [Rust 方案](rust-plan.md)。

## Manager 的关键 ack 与可见状态

每项关键操作采用“准备候选 → 释放主锁 → writer ack → 锁内条件提交 → 锁外发事件”。候选带预期状态版本和 attempt；若条件失效，不把旧候选写回内存。准备中的任务通过内部标记阻止同一任务被普通进度继续修改；不持有主状态锁等待 gate、写入或 ack。

| 操作 | 磁盘前的运行时行为 | 获得 ack 后 | 失败时 |
| --- | --- | --- | --- |
| create | 准备新任务，尚不可调度、不发布新任务 | 加入内存 FIFO、通知 worker | 返回错误，不发布未接受的任务 |
| claim | 保留 queued 候选，尚未启动进程 | 提交 Resolving、建立 attempt/Control、启动 | 不启动进程，显示诊断 |
| retry | 保留原失败/取消/中断记录 | 提交 Queued，清运行字段并移队尾 | 原公开记录保持，不伪装已排队 |
| delete | 保留原历史记录，不做文件操作 | 从内存和展示删除 | 保留记录并返回错误 |
| settings | 保留原公开设置 | 提交设置并重新探测 | 原设置保持并返回错误 |
| cancel | 主锁内先接受取消意图，锁外立刻 signal/notify 真进程；随后才等待关键 gate/DB | 提交 Cancelling/Cancelled | 进程仍停止；不能撤销取消意图，明确报告状态落库失败，由 worker 收敛终态 |
| terminal | 进程已结束，准备唯一确定终态，保留 pending finalization | 提交 Completed/Failed/Cancelled，才发送终态事件 | 保留候选并重试；不发布 Completed，不开始下个下载，给出持久化故障 |

终态准备在主锁内建立唯一 finalization 标记，这与原实现先取得状态锁并决定终态的竞争边界一致：此前已接受取消则结算 Cancelled；之后才到的取消不能将已决定的成功翻成取消，也不重复清理。快照在终态 ack 前保留原非终态和清晰的“正在保存最终状态/持久化失败”错误；不能仅在事件侧隐藏 Completed、却让 `get_app_state` 先读到它。

writer 的终态失败有界重试，仍失败则保留 pending 候选和诊断、阻止 worker 继续认领，不靠下次其他任务全量保存偶然修复。用户再次退出/后续恢复尝试会重试该候选。绝不能只把错误塞进随后可能被引擎探测覆盖的 warnings，然后返回持久化成功。

若 ack 丢失或关闭 deadline 到期，请求仍可能已提交：保留候选与序号，等待确定回执或按相同序号核对/重试。不能立即“回滚内存并忘记请求”，造成晚提交后的幽灵任务。ack 的 Stale 不等于候选已成功，必须根据其预期版本放弃或重建。

## 关闭、故障恢复与文件检查

关闭先停止接受新任务并建立一次总 deadline，立即 signal 活动进程；queued 取消合成一个事务，而不是每条任务重写一遍。等待真实进程结束、pending 终态确认、最新脏进度提交和 writer flush 均计算在同一 deadline 内。关键队列与 writer 排空之前不退出应用。

普通运行标志与最终 worker-stop 标志分开：优先在 flush 成功后再要求空闲 worker 结束。任一步失败，返回具体错误并恢复接受任务及唯一 worker，保留未确认终态与诊断。deadline 只停止本次退出尝试，不取消已经在 writer 内的 COMMIT；未决回执在后台收敛后恢复可用，不能让旧的关闭尝试晚到时再退出或关闭新 worker。恢复时确认原 worker 是否结束，必要时只启动一个替代 worker，避免“shutting_down 清了但调度器已死”。

`snapshot` 从短状态锁读取 DTO，常规进度广播不再逐个 stat 历史文件。文件可用性采用有界缓存，在历史列表/详情请求、窗口重新聚焦、终态完成时按需刷新；`task(id)` 只检查该项。文件真正打开时仍作现有 canonicalize/目录与扩展名验证，不依赖缓存授权。此项在阶段 3/4 接缝按旧 IPC 能力保持兼容，不先扩大权限。

## 实施顺序与门禁

1. 先落 repository 的版本读取、事务迁移、增量 SQL、未知数据保留，补临时数据库测试；暂不接 Manager。
2. 加 owned-thread writer、ack/flush 和失败测试；连接不跨运行时任务共享。
3. 逐项替换 Manager create/settings/delete/retry，再 claim/cancel/terminal/progress/shutdown，保留原 commands 与事件入口。每一步都移除对应的锁内 DB 路径，不能留下半套双写。
4. 更新同一个 `engineering_bench` 数据配方，保留 before JSON，输出 after；复测 100/1000/5000，不能以架构描述替代数字。

必须补：v1/v0迁移及失败回滚、未来版本不改文件、未知行原文保留、嵌套未知字段保留、ID 不一致保护、多次增量与终态恢复、旧版本/重复版本、删除后旧 upsert、进度晚于终态、事务失败 ack、线程断开、flush、DB busy 仍及时杀进程、snapshot 不暴露未确认 Completed、取消/完成两种顺序、关闭失败后继续 enqueue 与单一 worker。

测试只用临时数据库和确定性进程。阶段门禁使用 Rust tests/fmt/Clippy、worker fixture 与同配方性能测量；完整两窗口 smoke 由根统一执行。本页仅设计核对，没有新增运行结果。
