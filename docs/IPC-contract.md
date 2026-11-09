# Internal IPC contract

All JSON uses camelCase, status uses lowercase (postprocessing). See `src-tauri/src/models.rs` as canonical model.

Commands (Tauri invoke args camelCase):
- get_app_state() => AppSnapshot {settings, engines, tasks}
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

Events: `app-state-updated` => full AppSnapshot (throttled); `workspace-close-requested` => null; `shutdown-failed` => readable error string (frontend clears its exiting state). Settings/detection changes emit snapshots too. Frontend re-reads snapshot on focus. Browser preview shows empty state and explicit desktop-only message; never mock real downloads.

Settings accepted enums: defaultMode video|audio, quality best|2160|1440|1080|720|480, theme dark|light|system, density comfortable|compact, closeBehavior tray|exit, completionAction none|folder. enginePath/ffmpegPath paths (ffmpeg may be directory). Defaults defined in model.

Canonical command registrations are in `src-tauri/src/lib.rs`; frontend wrappers and listeners are in `src/lib/api.ts`. `src-tauri/src/models.rs` defines the backend schema and `src/lib/types.ts` mirrors it. Check these sources when changing this contract.
