import { seedApp } from "./fixtures";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import App from "../App";
import { api } from "../lib/api";
import { unknownErrorMessage } from "../lib/errors";
import {
  installMatchMedia,
  mediaFixture,
  snapshotFixture,
  taskFixture,
} from "./fixtures";

vi.mock("../lib/api", async (importOriginal) => {
  const original = await importOriginal<typeof import("../lib/api")>();
  return {
    ...original,
    api: {
      ...original.api,
      snapshot: vi.fn(),
      probe: vi.fn(),
      enqueue: vi.fn(),
      cancel: vi.fn(),
      settings: vi.fn(),
    },
  };
});

beforeEach(() => {
  vi.clearAllMocks();
  installMatchMedia();
  const snapshot = snapshotFixture([
    taskFixture("running", { status: "downloading" }),
  ]);
  vi.mocked(api.snapshot).mockResolvedValue(snapshot);
  seedApp(snapshot, false);
});
afterEach(cleanup);

describe("user-observable IPC failures", () => {
  it.each([
    {
      label: "structured DTO",
      value: {
        version: 1,
        code: "PROCESS_CANCEL_FAILED",
        phase: "process_cleanup",
        message: "停止任务失败，请重试",
        details: "private-cookie=not-for-product-ui",
      },
      message: "停止任务失败，请重试",
    },
    {
      label: "legacy string",
      value: "旧版后端取消失败",
      message: "旧版后端取消失败",
    },
    {
      label: "native Error",
      value: new Error("原生调用失败"),
      message: "原生调用失败",
    },
    {
      label: "malformed DTO",
      value: {
        version: 2,
        code: "UNKNOWN",
        phase: "cancel",
        message: "unsupported",
        details: "private-cookie=not-for-product-ui",
      },
      message: unknownErrorMessage,
    },
  ])(
    "shows $label through the existing toast without exposing diagnostic fields",
    async ({ value, message }) => {
      vi.mocked(api.cancel).mockRejectedValue(value);
      render(<App />);
      fireEvent.click(screen.getByRole("button", { name: "取消下载" }));
      const alert = await screen.findByRole("alert");
      expect(alert.textContent).toContain(message);
      expect(alert.textContent).not.toContain("private-cookie");
      expect(alert.textContent).not.toContain("[object Object]");
    },
  );

  it("shows a probe failure and lets the user try parsing again", async () => {
    vi.mocked(api.probe).mockRejectedValue(
      new Error("媒体需要登录，请使用公开链接"),
    );
    render(<App />);
    fireEvent.change(screen.getByRole("textbox", { name: "媒体链接" }), {
      target: { value: "https://example.org/private.mp4" },
    });
    fireEvent.click(screen.getByRole("button", { name: /解析链接/ }));
    expect(
      await screen.findByText("媒体需要登录，请使用公开链接"),
    ).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: /解析链接/ }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
    expect(api.probe).toHaveBeenCalledWith("https://example.org/private.mp4");
  });

  it("does not claim successful queueing when the backend rejects a download", async () => {
    vi.mocked(api.probe).mockResolvedValue(mediaFixture());
    vi.mocked(api.enqueue).mockRejectedValue(
      new Error("保存目录不可写，请选择其他目录"),
    );
    render(<App />);
    fireEvent.change(screen.getByRole("textbox", { name: "媒体链接" }), {
      target: { value: "https://example.org/authorized-test.mp4" },
    });
    fireEvent.click(screen.getByRole("button", { name: /解析链接/ }));
    fireEvent.click(
      await screen.findByRole("button", { name: "加入下载队列" }),
    );
    expect(
      await screen.findByText("保存目录不可写，请选择其他目录"),
    ).toBeTruthy();
    expect(screen.queryByText("已加入队列")).toBeNull();
    expect(
      (
        screen.getByRole("button", {
          name: "加入下载队列",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false);
  });

  it("keeps a running task visible and restores its action after cancellation fails", async () => {
    vi.mocked(api.cancel).mockRejectedValue(
      new Error("无法停止下载进程，请重试"),
    );
    render(<App />);
    const cancel = screen.getByRole("button", {
      name: "取消下载",
    }) as HTMLButtonElement;
    fireEvent.click(cancel);
    expect(await screen.findByText("无法停止下载进程，请重试")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "查看任务：媒体 running" }),
    ).toBeTruthy();
    await waitFor(() => expect(cancel.disabled).toBe(false));
    expect(api.cancel).toHaveBeenCalledWith("running");
  });

  it("preserves an unsaved settings draft when saving fails", async () => {
    vi.mocked(api.settings).mockRejectedValue(
      new Error("设置未保存：磁盘写入失败"),
    );
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: /^设置$/ }));
    const input = screen.getByRole("textbox", {
      name: "默认保存目录",
    }) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "C:\\Chosen folder" } });
    fireEvent.click(screen.getAllByRole("button", { name: "保存更改" })[0]);
    expect(await screen.findByText("设置未保存：磁盘写入失败")).toBeTruthy();
    expect(input.value).toBe("C:\\Chosen folder");
    expect(screen.getByText("有尚未保存的更改")).toBeTruthy();
  });
});
