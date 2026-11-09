import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  Settings,
  StructuredIpcError,
  TaskDetailsResult,
} from "../lib/types";
const ipc = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: ipc.invoke }));
let api: (typeof import("../lib/api"))["api"];
const previous = Object.getOwnPropertyDescriptor(window, "__TAURI_INTERNALS__");
beforeEach(async () => {
  vi.resetModules();
  ipc.invoke.mockReset();
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
  api = (await import("../lib/api")).api;
});
afterEach(() => {
  if (previous) Object.defineProperty(window, "__TAURI_INTERNALS__", previous);
  else Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
});

describe("real centralized invoke wrapper negotiation", () => {
  it("requests structured errors for both argument-free and argument-bearing commands", async () => {
    ipc.invoke.mockResolvedValue(undefined);
    await api.detect();
    await api.probe("https://example.org/video.mp4");
    await api.cancel("task-1");
    expect(ipc.invoke.mock.calls).toEqual([
      ["detect_engines", { errorFormat: "structured" }],
      [
        "probe_media",
        { url: "https://example.org/video.mp4", errorFormat: "structured" },
      ],
      ["cancel_task", { taskId: "task-1", errorFormat: "structured" }],
    ]);
  });
  it("preserves success payloads and nested settings arguments", async () => {
    const settings: Settings = {
      downloadDir: "C:\\Downloads",
      enginePath: "",
      ffmpegPath: "",
      defaultMode: "video",
      quality: "720",
      theme: "dark",
      density: "comfortable",
      reducedMotion: false,
      closeBehavior: "tray",
      completionAction: "none",
    };
    ipc.invoke.mockResolvedValue(settings);
    expect(await api.settings(settings)).toBe(settings);
    expect(ipc.invoke).toHaveBeenCalledWith("update_settings", {
      settings,
      errorFormat: "structured",
    });
    const { taskFixture } = await import("./fixtures");
    const details: TaskDetailsResult = {
      sessionId: "test-session",
      revision: 3,
      task: taskFixture("task-1"),
    };
    ipc.invoke.mockResolvedValue(details);
    expect(await api.details("task-1")).toBe(details);
    expect(ipc.invoke).toHaveBeenLastCalledWith("get_task_details", {
      taskId: "task-1",
      errorFormat: "structured",
    });
  });
  it("preserves structured and legacy rejection values for the shared formatter", async () => {
    const dto: StructuredIpcError = {
      version: 1,
      code: "NETWORK",
      phase: "probe",
      message: "请检查网络",
      details: "diagnostic",
    };
    ipc.invoke.mockRejectedValueOnce(dto).mockRejectedValueOnce("旧后端错误");
    await expect(api.probe("https://example.org/video.mp4")).rejects.toBe(dto);
    await expect(api.probe("https://example.org/video.mp4")).rejects.toBe(
      "旧后端错误",
    );
  });
  it("retains the browser-only preview guard without invoking native commands", async () => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    vi.resetModules();
    const browser = (await import("../lib/api")).api;
    await expect(browser.detect()).rejects.toThrow("当前为浏览器预览");
    expect(ipc.invoke).not.toHaveBeenCalled();
  });
});
