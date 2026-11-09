import { seedApp } from "./fixtures";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import App from "../App";
import { api } from "../lib/api";
import {
  installMatchMedia,
  mediaFixture,
  snapshotFixture,
  taskFixture,
} from "./fixtures";

vi.mock("../lib/api", async (importOriginal) => {
  const original = await importOriginal<typeof import("../lib/api")>();
  return { ...original, api: { ...original.api, snapshot: vi.fn() } };
});

const tasks = [
  taskFixture("failed", { status: "failed" }),
  taskFixture("queued"),
  taskFixture("older", {
    status: "completed",
    media: mediaFixture({ title: "Old mountain", source: "Archive" }),
    updatedAt: "2026-01-01T00:00:00Z",
  }),
  taskFixture("downloading", { status: "downloading" }),
  taskFixture("newer", {
    status: "completed",
    media: mediaFixture({ title: "Deep SEA", source: "Nature" }),
    updatedAt: "2026-01-03T00:00:00Z",
  }),
  taskFixture("cancelling", { status: "cancelling" }),
  taskFixture("cancelled", { status: "cancelled" }),
];
const visibleTitles = () =>
  screen
    .queryAllByRole("button", { name: /^查看任务：/ })
    .map((button) =>
      button.getAttribute("aria-label")?.replace("查看任务：", ""),
    );

beforeEach(() => {
  vi.clearAllMocks();
  installMatchMedia();
  const snapshot = snapshotFixture(tasks);
  vi.mocked(api.snapshot).mockResolvedValue(snapshot);
  seedApp(snapshot, false);
});
afterEach(cleanup);

describe("queue and history derived views", () => {
  it("filters active and waiting tasks without changing backend queue order", () => {
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: /^下载队列/ }));
    expect(visibleTitles()).toEqual([
      "媒体 queued",
      "媒体 downloading",
      "媒体 cancelling",
    ]);
    fireEvent.click(screen.getByRole("button", { name: /^等待中/ }));
    expect(visibleTitles()).toEqual(["媒体 queued"]);
    fireEvent.click(screen.getByRole("button", { name: /^全部/ }));
    expect(visibleTitles()).toEqual([
      "媒体 failed",
      "媒体 queued",
      "Old mountain",
      "媒体 downloading",
      "Deep SEA",
      "媒体 cancelling",
      "媒体 cancelled",
    ]);
  });

  it("shows only completed history ordered by updated time and searches title or source case-insensitively", () => {
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: /^下载历史$/ }));
    expect(visibleTitles()).toEqual(["Deep SEA", "Old mountain"]);
    const search = screen.getByRole("textbox", { name: "搜索下载历史" });
    fireEvent.change(search, { target: { value: "sea" } });
    expect(visibleTitles()).toEqual(["Deep SEA"]);
    fireEvent.change(search, { target: { value: "ARCHIVE" } });
    expect(visibleTitles()).toEqual(["Old mountain"]);
    fireEvent.change(search, { target: { value: "not found" } });
    expect(visibleTitles()).toEqual([]);
    expect(screen.getByText("没有找到相关下载")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "清除搜索" }));
    expect(visibleTitles()).toEqual(["Deep SEA", "Old mountain"]);
  });
});
