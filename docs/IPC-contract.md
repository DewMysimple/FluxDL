# Internal IPC contract

All JSON uses camelCase, status uses lowercase (postprocessing). See `src-tauri/src/models.rs` as canonical model.

Commands (Tauri invoke args camelCase):

- get_app_state() => AppSnapshot {sessionId, revision, settings, engines, tasks}
- get_task_details({taskId}) => {sessionId, revision, task} (non-null task; missing/deleted returns an error)
- detect_engines() => EngineDiagnostics
- probe_media({url}) => MediaInfo
- enqueue_download({input: EnqueueInput}) => DownloadTask
- list_tasks() => DownloadTask[]
- get_settings() => Settings
- list_history({query?}) => DownloadTask[] (completed only, newest first)
- cancel_task({taskId}) / retry_task({taskId}) => void
- remove_history_record({taskId}) => void (completed only)
- update_settings({settings}) => Settings
- open_download_file({taskId}) / reveal_in_folder({taskId}) => void
- show_quick_capture() / show_workspace() / hide_quick_capture() => void
- resize_quick({height}) => void (finite logical height, bounded by current monitor and 260–720)
- request_exit() => void (cancel running processes, exit)
- open_help({topic: ejs|ytdlp|ffmpeg}) => void (fixed official URLs only)
- window_action({action: minimize|maximize|close|hide}) => void

所有应用command额外接受可选`errorFormat: "structured"`；省略时继续拒绝String，成功返回值不变。新前端由`api.call`统一发送该参数，也能显示旧后端返回的String或原生Error。插件原生对话框使用插件本身的合同，不附加应用错误参数。

结构化失败为`{version:1,code,phase,message,details?}`，details为空时省略。code来自明确操作边界的稳定枚举（例如INVALID_INPUT、PROCESS_READ、PROCESS_EXIT、PERSISTENCE_REJECTED、PERSISTENCE_UNCONFIRMED），phase为validate/startup/detect/probe/download/postprocess/persist/window/shutdown之一。不能从中文提示反推类别。Process非零退出诊断保留真实退出码；Writer被拒绝与ack不确定持续区分，传播时不降成通用String再分类。

`message/details`均经统一脱敏和长度限制，UI Toast只显示message；任务日志/安全技术详情仍可用于排障。Task.error和`shutdown-failed`事件继续为安全可读String，避免破坏旧数据/监听者。完整未知对象与畸形DTO有可读fallback，不显示`[object Object]`。实现为Rust `errors.rs` / `diagnostics.rs`、前端`lib/errors.ts`；协商真实桌面测试记录在工程验证报告。

Events: `app-state-changed` => {sessionId, revision, change}; change.type is checkpoint, taskUpsert, taskProgress, taskRemoved, settingsUpdated, diagnosticsUpdated, or fileAvailabilityUpdated. TaskUpsert includes the committed task and its authoritative index; progress contains runtime fields and bounded logsAppend. Normal operations no longer broadcast legacy full `app-state-updated`. Precise JSON and atomic snapshot/log rules are in [IPC 0.2 contract](engineering/ipc-plan.md).

`workspace-close-requested` => null; `shutdown-failed` => readable error string (frontend clears its exiting state). Frontend re-reads the full versioned snapshot on focus or detected gaps. A six-second checkpoint exposes a lost final event even when idle. Browser preview shows empty state and explicit desktop-only message; never mock real downloads.

Settings accepted enums: defaultMode video|audio, quality best|2160|1440|1080|720|480, theme dark|light|system, density comfortable|compact, closeBehavior tray|exit, completionAction none|folder. enginePath/ffmpegPath paths (ffmpeg may be directory). Defaults defined in model.

Canonical command registrations are in `src-tauri/src/lib.rs`, implementations in `commands.rs`; frontend wrappers and listeners are in `src/lib/api.ts`. `src-tauri/src/models.rs` defines the backend schema and `src/lib/types.ts` mirrors it. Check these sources when changing this contract.

## 本地诊断协议（2026-11-09）

probe_media额外接受可选traceId，enqueue_download接受可选traceId/relatedTraceId；原成功类型和errorFormat协商不变。前端生成独立UUID，后端严格验证、拒绝最近窗口内重复操作ID；ID不作权限凭据。前端record_frontend_diagnostic只接收已观测操作的两种UI阶段和有限durationMs/success，不能提交任意日志文本。

diagnostic_commands.rs注册get_diagnostics_health、query_diagnostic_events(filter?)、get_diagnostics_overview(filter?)、refresh_diagnostic_report、open_diagnostics_directory、export_diagnostics_bundle(filter?)、reveal_diagnostics_bundle、show_task_diagnostics(taskId)。查询filter包含sessionId/traceId/taskId、level/source/phase、search、fromUtc/toUtc、cursor/limit，默认100/上限200，未知字段拒绝。Overview提供统计、最近错误和最多100个慢Trace（当前最多20,000事件分析窗口）。get_diagnostic_trace由query的受限traceId过滤实现，不提供任意文件读取。

日志文件Event采用snake_case schema_version=1，fields为白名单；错误保留稳定error_code和可选源操作error_kind，不包含原始stderr。普通状态IPC的session/revision与诊断session完全独立；原AppEvent、tombstone、快照原子日志flush及6秒checkpoint不变。

系统目录/导出/定位只允许workspace窗口；导出目的地由Rust原生save dialog选择，无前端任意路径参数，拒绝覆盖。诊断查询两窗可用，capabilities未扩大。diagnostics-locate-task只发给workspace，diagnostics-changed是独立低频提示，不占任务revision。UI每2秒读真实诊断，历史页不自动翻动。完整schema与阶段语义见[设计](engineering/observability-design.md)。

## 0.2.1 兼容扩展

probe_media仍返回MediaInfo，新增可选trustedPreviewRef（不透明、短期、Rust所有）。enqueue_download仍接受原EnqueueInput.media；没有/失效引用走完整提取，前端任意格式/标题不授权。引用在任务构造前移除，不进入DownloadTask/SQLite或AppEvent。EngineInfo新增beforeDlSupported（旧JSON默认false），用于离线检测后的安全标记选择；前端类型为可选，旧字段和普通IPC session/revision不变。

Event仍schema1，Fields新增可选environment_id/environment、cache_outcome/output_stream。缺字段的旧记录可读，未知字段/阶段/未来schema仍计入不可读提示。Stage新增session_environment、download_first_output、download_before_dl、download_before_dl_to_progress、download_process_exit、preview_cache_decision、enqueue_trusted_validate；download_first_progress原含义不变。

Health新增build（编译revision/dirty/app_version）及elapsedSinceSessionStartMs；Overview新增cacheStats，Statistic带sessionId/environmentId/cacheOutcome/codeRevision/engineVersions，按组计算。taskId查询将其关联Trace中的入队前事件也纳入，仍受统一时间/来源/等级/数量/cursor与20,000窗口约束。导出附加所选记录对应的会话环境，不附加其他业务任务/数据库；旧环境unknown，不回填。

configure_latency_test_mode(mode)只允许workspace，debug接受full_recheck/trusted_preview_cache，两条路径都保留安全验证；Release明确拒绝，不暴露产品设置或任意参数。其余目录/保存/定位权限和capabilities完全不变。
