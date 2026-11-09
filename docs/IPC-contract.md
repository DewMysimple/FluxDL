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
