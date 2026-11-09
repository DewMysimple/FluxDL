import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import App from "../App";
import { api, previewSnapshot } from "../lib/api";
import { useApp } from "../stores/app";
import type { AppSnapshot, DownloadTask } from "../lib/types";

vi.mock("../lib/api", async (importOriginal) => {
  const original = await importOriginal<typeof import("../lib/api")>();
  return {
    ...original,
    api: { ...original.api, snapshot: vi.fn(), probe: vi.fn() },
  };
});

const task = (status: DownloadTask["status"]): DownloadTask => ({
  id: "fixture-task",
  media: {
    url: "https://example.org/authorized-test.mp4",
    title: "测试媒体",
    mediaId: "test",
    source: "Example",
    duration: null,
    thumbnail: null,
    isPlaylist: false,
    formats: [],
    warnings: [],
  },
  mode: "video",
  quality: "1080",
  formatId: null,
  outputDir: "C:\\Downloads",
  status,
  progress: null,
  speed: null,
  eta: null,
  downloadedBytes: 512,
  totalBytes: null,
  finalPath: null,
  error: null,
  logs: [],
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:00Z",
  fileExists: false,
});
const setup = (tasks: DownloadTask[] = []) => {
  const state: AppSnapshot = {
    ...previewSnapshot,
    settings: { ...previewSnapshot.settings },
    tasks,
  };
  vi.mocked(api.snapshot).mockResolvedValue(state);
  useApp.setState({ snapshot: state, loading: false, error: null });
  return state;
};

beforeEach(() => {
  vi.clearAllMocks();
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    value: vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  });
  setup();
});
afterEach(cleanup);

describe("backend-authoritative desktop views", () => {
  it("rejects empty input before invoking the media engine", async () => {
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: /解析链接/ }));
    expect(await screen.findByText("请先粘贴一个媒体链接")).toBeTruthy();
    expect(api.probe).not.toHaveBeenCalled();
  });
  it("shows unknown progress without inventing a percentage", async () => {
    setup([task("downloading")]);
    render(<App />);
    const progress = await screen.findByRole("progressbar");
    expect(progress.getAttribute("aria-valuenow")).toBeNull();
    expect(screen.getByText("512 B")).toBeTruthy();
    expect(screen.queryByText("100%")).toBeNull();
  });
  it("disables file actions when a completed file is missing", async () => {
    setup([task("completed")]);
    render(<App />);
    fireEvent.click(
      await screen.findByRole("button", { name: "查看任务：测试媒体" }),
    );
    const openButton = screen.getByRole("button", {
      name: /^打开文件$/,
    }) as HTMLButtonElement;
    expect(openButton.disabled).toBe(true);
    expect(
      screen.getByText(
        "本地文件不存在，可能已被移动或删除。下载历史仍然保留。",
      ),
    ).toBeTruthy();
  });
  it("keeps unsaved settings edits when a progress snapshot arrives", async () => {
    const state = setup();
    render(<App />);
    await waitFor(() => expect(api.snapshot).toHaveBeenCalled());
    fireEvent.click(screen.getByRole("button", { name: /^设置$/ }));
    const input = screen.getByRole("textbox", {
      name: "默认保存目录",
    }) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "C:\\My chosen folder" } });
    await act(async () =>
      useApp
        .getState()
        .setSnapshot({
          ...state,
          settings: { ...state.settings },
          tasks: [task("downloading")],
        }),
    );
    expect(input.value).toBe("C:\\My chosen folder");
    expect(screen.getByText("有尚未保存的更改")).toBeTruthy();
  });
});
