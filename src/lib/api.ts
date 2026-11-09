import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  AppSnapshot,
  AppEvent,
  DownloadTask,
  EngineDiagnostics,
  EnqueueInput,
  MediaInfo,
  Settings,
  TaskDetailsResult,
} from "./types";
export const isDesktop = "__TAURI_INTERNALS__" in window;
const missingEngine = {
  path: null,
  version: null,
  source: "none",
  available: false,
  message: "请在桌面应用中检测本地引擎",
};
export const previewSnapshot: AppSnapshot = {
  sessionId: "browser-preview",
  revision: 0,
  settings: {
    downloadDir: "",
    enginePath: "",
    ffmpegPath: "",
    defaultMode: "video",
    quality: "1080",
    theme: "dark",
    density: "comfortable",
    reducedMotion: false,
    closeBehavior: "tray",
    completionAction: "none",
  },
  engines: {
    ytdlp: missingEngine,
    ffmpeg: missingEngine,
    ffprobe: missingEngine,
    runtime: missingEngine,
    warnings: [],
  },
  tasks: [],
};
async function call<T>(
  name: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!isDesktop)
    throw new Error(
      "当前为浏览器预览。请运行 FluxDL 桌面应用以使用本地引擎、下载和系统功能。",
    );
  return invoke<T>(name, { ...args, errorFormat: "structured" });
}
export const api = {
  snapshot: () =>
    isDesktop
      ? call<AppSnapshot>("get_app_state")
      : Promise.resolve(previewSnapshot),
  detect: () => call<EngineDiagnostics>("detect_engines"),
  probe: (url: string) => call<MediaInfo>("probe_media", { url }),
  enqueue: (input: EnqueueInput) =>
    call<DownloadTask>("enqueue_download", { input }),
  cancel: (taskId: string) => call<void>("cancel_task", { taskId }),
  retry: (taskId: string) => call<void>("retry_task", { taskId }),
  remove: (taskId: string) => call<void>("remove_history_record", { taskId }),
  settings: (settings: Settings) =>
    call<Settings>("update_settings", { settings }),
  open: (taskId: string) => call<void>("open_download_file", { taskId }),
  reveal: (taskId: string) => call<void>("reveal_in_folder", { taskId }),
  details: (taskId: string) =>
    call<TaskDetailsResult>("get_task_details", { taskId }),
  quick: () => call<void>("show_quick_capture"),
  workspace: () => call<void>("show_workspace"),
  hideQuick: () => call<void>("hide_quick_capture"),
  resizeQuick: (height: number) => call<void>("resize_quick", { height }),
  exit: () => call<void>("request_exit"),
  help: (topic: "ejs" | "ytdlp" | "ffmpeg") =>
    call<void>("open_help", { topic }),
  window: (action: "close" | "hide" | "minimize" | "maximize") =>
    call<void>("window_action", { action }),
  selectPath: async (directory: boolean, title: string) => {
    if (!isDesktop) throw new Error("原生路径选择器需要在桌面应用中使用");
    const path = await open({
      directory,
      multiple: false,
      title,
      ...(!directory
        ? { filters: [{ name: "可执行程序", extensions: ["exe"] }] }
        : {}),
    });
    return typeof path === "string" ? path : null;
  },
  onChange: (fn: (event: AppEvent) => void) =>
    listen<AppEvent>("app-state-changed", (e) => fn(e.payload)),
  onClose: (fn: () => void) => listen("workspace-close-requested", fn),
  onShutdownFailed: (fn: (message: string) => void) =>
    listen<string>("shutdown-failed", (e) => fn(e.payload)),
};
