# IPC 0.2 实际接缝合同

状态：**阶段4固定合同，后端已实现，整体验证待根统一完成**。精确类型在 `src-tauri/src/events.rs`、`models.rs`，query注册在 `lib.rs`；前后端按同一合同接入。旧command名、camelCase任务JSON、lowercase状态和权限保持不变。

## 通道与版本

单一增量事件名为 `app-state-changed`，切换后不再常规广播旧的完整 `app-state-updated`。每次Manager运行生成固定UUID `sessionId`；会话内 `revision` 从0开始单调递增，为非负安全整数，不能wrap。每个实际捕获的可发布envelope加一，不对被合并的原始进度、数据库写入分配IPC序号。

完整 `get_app_state` 保留 `settings/engines/tasks`，增加 `sessionId/revision`。revision与状态在同一次短状态锁内克隆，涵盖已捕获但尚待锁外发送的事件。数据库mutation_seq、进程AttemptId、文件检查token与IPC revision各自独立。

## 精确JSON与TypeScript形状

变体使用内层 `change.type`，不flatten envelope。Rust Serde必须产生以下精确camelCase字段，序列化测试核对字面JSON：

```ts
type AppEvent = {
  sessionId: string;
  revision: number;
  change: AppChange;
};

type AppChange =
  | { type: "checkpoint" }
  | { type: "taskUpsert"; task: DownloadTask; index: number }
  | {
      type: "taskProgress";
      taskId: string;
      status: TaskStatus;
      progress: number | null;
      speed: number | null;
      eta: number | null;
      downloadedBytes: number;
      totalBytes: number | null;
      updatedAt: string;
      logsAppend: string[];
    }
  | { type: "taskRemoved"; taskId: string }
  | { type: "settingsUpdated"; settings: AppSettings }
  | { type: "diagnosticsUpdated"; engines: EngineDiagnostics }
  | {
      type: "fileAvailabilityUpdated";
      files: Array<{ taskId: string; fileExists: boolean }>;
    };

type AppSnapshot = {
  sessionId: string;
  revision: number;
  settings: AppSettings;
  engines: EngineDiagnostics;
  tasks: DownloadTask[];
};
```

AppSettings等沿用现有前端类型；Rust可继续使用Settings。公开DownloadTask不增加attempt/数据库版本。TaskProgress不携带title/media/formats、已有日志全集或全部历史；finalPath/error/mode/quality等语义字段改变时使用单任务TaskUpsert。

TaskUpsert.index是该次提交后在后端tasks Vec中的零起始位置。前端先移除既有ID，再插入index，其他ID和对象保留；创建插入当前位置，retry明确移到队尾。进度不移动顺序。非法index触发恢复，不猜顺序。

## 按需详情查询

新增内部command `get_task_details({taskId})`，返回 `{sessionId:string, revision:number, task:DownloadTask}`，task不为null；不存在或删除竞态返回原有Err字符串。它仅刷新该项文件信息，最后同锁flush所有pending progress/log append，再克隆task和session/revision，锁外emit，不为打开详情强刷全部历史。

前端只在详情打开时请求，只有同session且response.revision等于当前可信revision时合并单项。旧响应拒绝；更高revision或未知session请求完整恢复，不能用单项详情跳过全局sequence。

## 捕获与发送

关键ack后，在提交内存的**同一个状态锁**内捕获当时的TaskUpsert/TaskRemoved/SettingsUpdated并分配revision。不能只mark dirty ID后到emit再读最新task，否则极快retry会吞掉前一个终态。诊断改变同样在变化被接受的锁内捕获。

普通进度采用dirty task IDs和pending log appends，约250ms捕获最新runtime小包；不能先clone全量snapshot再diff。日志先脱敏并限制每行1600字符，每任务pending append保留最近100行，与task.logs上限相同。前端 `(oldLogs + logsAppend).slice(-100)` 得到一致结果。

使用简单typed outbox与发送gate：锁内按revision排入待发envelope，锁外单个publisher drain并emit，后来的publisher也先发送旧envelope。不引入通用消息总线或第二份任务真相。emit失败记录诊断，序号不复用，缺口由snapshot恢复。

捕获TaskUpsert时清该任务pending progress/log append，因为它已经包含完整有界logs；TaskRemoved同样清除。关键事件不受普通进度节流吞掉。

## 日志防重复：仅快照原子flush，不加cursor

完整snapshot的最后一个短锁严格按顺序：

1. 应用通过mutation version、finalPath和refresh token验证的文件检查；有实际变化时捕获一个紧凑FileAvailabilityUpdated批次并分配revision。
2. 将**所有**pending dirty task的最新progress/log append捕获为envelope，分配revision，清pending append和dirty标记。
3. 克隆完整状态以及此时sessionId/revision。
4. 释放锁，再发送outbox，返回该snapshot。

因此snapshot里的C/D日志已被其revision覆盖；稍后到达的对应旧append被前端丢弃。snapshot捕获之后新E才属于更高revision，不会重复C/D。没有第二套logCursor/logOffset。

文件检查等待期间的语义变化仍即时捕获。`task(id)`单项刷新改变fileExists时也要锁内捕获files批次推进revision，不可只更新缓存。5,000项可用性变化用一个紧凑files批次，不发5,000个完整task。旧异步结果仍必须通过token/version/path守卫。

## 前端恢复与进度语义

先监听 `app-state-changed`，缓冲注册及snapshot请求期间事件，再按session/revision合并：

- 丢弃旧/重复revision，防止重复append；发现缺口只启动一个恢复请求并有界缓冲，不能靠setTimeout猜顺序。
- snapshot晚返回不能覆盖同会话更高revision；应用snapshot后再合并其后连续事件。
- 未知session事件只触发权威snapshot，不能单凭事件切换会话。旧会话不能污染新会话。
- 监听/请求失败展示错误并保留最后可信状态；卸载后不应用结果并清监听。
- 未知任务ID的progress不能捏造不完整task，应重新同步；移除已不存在的ID可幂等处理。
- TaskUpsert允许失败/取消/中断后retry为Queued；TaskProgress不得将终态变回运行中。
- TaskProgress/files仅替换受影响task；taskIds及无关对象保持引用，settings/diagnostics不重建任务集合。

yt-dlp可先后下载不同音视频流，估计总大小也会修正。前端不能用max(percent)伪造总任务进度；拒绝旧session/revision后展示后端认可的当前流指标。后端先验证attempt和终态护栏，再捕获IPC事件，不让旧attempt产生新revision。

## 末帧丢失的低频恢复口

后端约每6秒发布正常有序的 `{type:"checkpoint"}` envelope，revision照常加一。健康前端只推进版本，不改变领域数据。若唯一终态事件丢失、随后没有其他业务事件，checkpoint仍会暴露revision缺口并触发完整恢复，不能永远停留在下载中。

checkpoint由轻量事件循环发出，不等待数据库或关键写入gate，窗口隐藏时仍工作。该循环也是250ms普通进度flush的唯一周期来源；进程读取循环不另开一套progress发布tick。关闭/恢复仍保证只有一个事件执行器。

## 必测

序列化字面字段、监听完成即来事件、snapshot期间事件、旧snapshot晚返回、重复/乱序/缺口、丢唯一终态后checkpoint暴露缺口、DB占用时仍有checkpoint、跨会话、retry后的旧attempt、终态后的进度、retry index/删除顺序、两个独立store收敛、隐藏重显，以及单条进度只影响相关行。

日志专项：snapshot含C/D后旧append不重复，新E正常追加；TaskUpsert清pending append；超过100行截断后与后端相同。后端专项：终态紧接retry的两个envelope均保留；snapshot revision覆盖全部待发状态；晚文件检查不制造变化；高频发布不clone完整状态。

阶段4用真实桌面trace测事件字节/频率，不能只凭小包类型宣称性能提升。阶段3持久化ack、关闭恢复、未知数据保留和缓存安全门禁继续有效。完整恢复snapshot保留，IPC性能另与full payload比较。
