# 诊断与可观测性设计

实现入口：`src-tauri/src/diagnostics/`（event、writer、report、export、redact、服务/Trace），`diagnostic_commands.rs`，`features/diagnostics/`。复用原错误分类及脱敏门面，没有引入第二套文本文件日志、远程SDK或新依赖。诊断与原任务SQLite/IPC session/revision独立。

## 单一事件来源

Rust事件schema_version=1；JSONL使用snake_case，查询参数/页面DTO使用camelCase。`event.rs::Event`和前端`types.ts`定义：event/session/trace/span UUID、可空task/parent、UTC人类时刻、session单调毫秒、level、source、event、phase、status、可空duration，以及仅related_trace_id、attempt_id、error_code、error_kind的fields。

source只有rust/frontend，event为stage_started/stage_finished/fault_observed；phase、status、level、error_code、error_kind均为枚举。未知字段拒绝反序列化，ID必须规范小写UUID，时间/浮点数有边界。fault_observed用于持久化/回收异常时点，duration为null，不伪装成跨度，也不覆盖业务终态；同Trace同类故障合并。没有任意message/context对象，也没有接受原始异常字符串的写入接口。UUID不是权限凭据；重复操作ID在有界最近请求窗口内被拒绝。前端只能报告已被本会话后端观测到的解析/入队操作，最多600秒、一次完成测量；相同阶段重复提交被合并。

每次解析一个Trace，每次入队一个独立Trace，related_trace_id关联之前解析；用户选择格式时间从未被作为跨度计入。重试维持业务task_id，建立新的Trace/attempt_id并关联前次Trace。前端performance.now()和Rust Instant各自测量，不能跨时钟相减。阶段共享state，重复start/end不会覆盖已结束阶段；业务attempt/stream/terminal准入检查仍先于进度埋点。终态诊断在实际SQLite确认后结束；无有效进度的成功进程仍标“未观测到首次进度”。

## 实际阶段边界

| 需求阶段 | 实际phase | 代码与边界 |
| --- | --- | --- |
| P01 | parse_ui_request | api.ts，发起invoke到响应返回；独立前端时钟，不宣称包含浏览器绘制 |
| P02 | parse_ipc_dispatch | Manager::probe_with_trace，command委派至配置快照就绪；不估计IPC传输时间 |
| P03 | parse_validate | engine::probe_traced，URL/引擎/参数准备 |
| P04 | parse_engine_spawn | process/capture.rs，实际Command.spawn前后 |
| P05 | parse_metadata_extract | spawn成功至退出、管道读取及JSON解码完成；包含子进程/网络，不伪造网络细分 |
| P06 | parse_response | media_from_json结果封装；parse_total包含整个Rust解析 |
| P07 | enqueue_validate | 入队基本模式/预设校验；后续格式/目录/参数仍按原逻辑校验，失败进入本Trace |
| P08 | enqueue_metadata_recheck | enqueue_owned的真实再次probe，保留全部安全校验 |
| P09 | enqueue_persist | 任务构建到durable ack，含关键gate；enqueue_db_queue是bounded channel/owner排队，enqueue_db_write是repository.apply事务（含SQLite锁等待） |
| P10 | queue_wait | 新任务ack至claim状态ack，独立串行自然等待 |
| P11 | download_engine_spawn | process/download.rs的真实spawn，不是handle创建 |
| P12 | download_first_progress | spawn成功至通过attempt/stream/状态准入的typed下载进度；不是首字节 |
| P13 | download_active | 首次有效进度至后处理信号或owner回收，记录范围含协议/进程收尾开销 |
| P14 | download_postprocess | 实际__FLUX_POST__信号至进程及reader回收；没有信号是不适用 |
| P15 | download_terminal | owner已回收/文件校验结果至最终持久化确认 |

enqueue_to_progress从入队请求开始，download_total从claim确认开始；二者与子阶段存在嵌套，不能把所有表格行简单相加。UI瀑布只对齐本进程Rust单调时点，前端测量单列。缺失start/end、旧session未闭合跨度、无进度均保留未知状态，不补0。

## 写入、保留与失败

Debug默认由编译期CARGO_MANIFEST_DIR父目录定位`dev-logs/`；允许仅debug的FLUXDL_DIAGNOSTICS_DIR明确覆盖。Release编译分支只调用Tauri `app.path().app_log_dir()`，不可回退源码/EXE/发行目录。具体系统语义参见[Tauri路径API](https://docs.rs/tauri/latest/tauri/path/struct.PathResolver.html#method.app_log_dir)。两窗口共享Rust服务，不争抢文件。

普通队列1024、关键错误/终态保留队列256，生产者try_send，不等待文件I/O。内存环2000事件，任务/操作关联也有上限。关键队列满时记录受限计数，writer从有界内存补写；不能承诺断电、磁盘永久失败或超过所有界限仍零丢失。队列背压与文件失败会显示一次持续诊断状态，下载依旧按原生命周期运行。普通任务技术日志也改为固定安全引擎摘要；错误保留稳定类别/真实退出码，不保存原始stderr/媒体文件名。旧业务数据库不清空、不改变路径，现有旧原文的展示仍经脱敏；诊断导出完全不收集数据库。

独立标准线程写JSONL，文件最大默认10MiB，最多5个已知session轮转文件，7天过期。文件名含毫秒时间、session UUID、轮转序号；单行完整JSON，尾行不完整可跳过。只清理严格命名的普通文件，检查每层目录/链接/Windows reparse point；未知文件和导出ZIP不自动删除。根目录/写入失败不传播到下载。报告临时文件写完后替换，失败清理自有临时文件；原报告保留。每2秒有新事件时自动刷新，启动及正常退出也刷新，UI可主动请求。报告包含生成时间、明确样本不足、慢Trace及实际源码/日志文件入口；引擎版本只记录规范化数值核心和必要固定配置概要。报告提示损坏、不兼容、截断和入队受限计数。

## 查询、界面与导出

读取最多最近5文件、每文件10MiB级，内存分析窗口最多20,000事件；分页默认100，上限200，cursor/关键词/时间/UUID有界。全部磁盘查询在spawn_blocking执行，不持任务状态锁。内存与磁盘按event_id合并；同session按单调时点排列。损坏行、未来schema、非法字段不会透传UI/ZIP。窗口截断会明示，超出窗口的Trace不能当成完整历史。当前实现不提供无限历史全文索引。测得30,000事件重复扫描约626–644ms后，增加单个受限历史查询缓存（最多20,000条、5秒TTL）：每次命中仍检查目录边界与文件名/长度/修改时刻，新内存事件generation立即失效；并发读合并，只克隆返回页。该缓存锁不被事件生产者或任务状态获取。导出强制丢弃缓存并重新解码，不能靠命中绕过白名单或私有数据检查。

Workspace侧栏底部入口，四Tab：概览、性能、事件、异常；真实统计nearest-rank p50/p95，只汇总成功且有耗时样本，标注n、最慢Trace和数据区间。失败/取消/未知不混入成功分位数。事件100行分页，停在历史页不自动翻动；首页每2秒查询，新日志记录不依赖页面是否打开。任务详情/Quick Capture可发受控定位事件；双窗查询相同历史。任务准备文字来自实际阶段事件，不用定时器伪造连接/首字节状态。

仅Workspace可打开可信日志目录、导出、定位最近导出。原capabilities未扩大。导出通过Rust原生保存对话框（用户选择路径），前端不能传任意目的文件；拒绝覆盖、拒绝路径祖先链接，create_new。界面先显示六个将包含的文件与本地隐私提示，再打开保存对话框。ZIP32 STORE不引入依赖，固定条目名，每条32MiB/包64MiB上限，独立Python CRC校验。environment仅系统主版本/架构、应用和安全引擎版本、固定配置摘要；Windows RtlGetVersion只取主版本，不采集用户/设备名/环境变量。privacy-manifest列排除类别，不复制SQLite、下载文件或原始stderr。生成后可Explorer定位，绝无自动上传。

未实现可选debug/trace级别开关，因此没有隐含超高频记录模式。默认只记录上述阶段和错误，下载每帧进度不进JSONL。
