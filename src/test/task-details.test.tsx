import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { TaskDetails } from "../features/downloads/TaskDetails";
import { ErrorToast } from "../components/feedback";
import { api } from "../lib/api";
import { useApp } from "../stores/app";
import type { TaskDetailsResult } from "../lib/types";
import { seedApp, snapshotFixture, taskFixture } from "./fixtures";

vi.mock("../lib/api", async (importOriginal) => {
  const original = await importOriginal<typeof import("../lib/api")>();
  return {
    ...original,
    isDesktop: true,
    api: { ...original.api, snapshot: vi.fn(), details: vi.fn() },
  };
});

beforeEach(async () => {
  vi.resetAllMocks();
  const snapshot = {
    ...snapshotFixture([taskFixture("a", { status: "downloading" })]),
    revision: 3,
  };
  seedApp(snapshot);
  vi.mocked(api.snapshot).mockResolvedValue(snapshot);
  await useApp.getState().refresh();
});
afterEach(() => {
  cleanup();
  useApp.getState().beginSync(async () => undefined)();
});

describe("on-demand task details", () => {
  it("loads the opened task and merges same-session same-revision complete details", async () => {
    vi.mocked(api.details).mockResolvedValue({
      sessionId: "fixture-session",
      revision: 3,
      task: taskFixture("a", { status: "downloading", logs: ["完整任务诊断"] }),
    });
    render(<TaskDetails taskId="a" />);
    await waitFor(() => expect(api.details).toHaveBeenCalledWith("a"));
    fireEvent.click(screen.getByText("技术诊断与日志"));
    expect(await screen.findByText("完整任务诊断")).toBeTruthy();
  });
  it("does not apply a detail response after the drawer unmounts", async () => {
    let finish: (value: TaskDetailsResult) => void = () => {
      throw new Error("Not requested");
    };
    vi.mocked(api.details).mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const view = render(<TaskDetails taskId="a" />);
    await waitFor(() => expect(api.details).toHaveBeenCalledTimes(1));
    view.unmount();
    await act(async () =>
      finish({
        sessionId: "fixture-session",
        revision: 3,
        task: taskFixture("a", { logs: ["late"] }),
      }),
    );
    expect(useApp.getState().tasksById.a.logs).toEqual([]);
  });
  it("uses full resynchronization for higher-revision details without skipping global events", async () => {
    const done = taskFixture("a", { status: "completed", fileExists: true });
    vi.mocked(api.details).mockResolvedValue({
      sessionId: "fixture-session",
      revision: 4,
      task: done,
    });
    vi.mocked(api.snapshot).mockResolvedValue({
      ...snapshotFixture([done]),
      revision: 4,
    });
    render(<TaskDetails taskId="a" />);
    await waitFor(() => expect(useApp.getState().revision).toBe(4));
    expect(api.snapshot).toHaveBeenCalledTimes(2);
    expect(
      (
        screen.getByRole("button", {
          name: /^打开文件$/,
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false);
  });
  it("reports detail IPC failures through the existing user-visible error boundary", async () => {
    vi.mocked(api.details).mockRejectedValue(
      new Error("读取任务详情失败，请重试"),
    );
    render(
      <>
        <TaskDetails taskId="a" />
        <ErrorToast />
      </>,
    );
    expect((await screen.findByRole("alert")).textContent).toContain(
      "读取任务详情失败，请重试",
    );
  });
});
