import type { TaskStatus } from "./types";
export const statusLabels: Record<TaskStatus, string> = {
  queued: "等待中",
  resolving: "正在准备",
  downloading: "下载中",
  postprocessing: "正在合并 / 处理",
  completed: "已完成",
  failed: "下载失败",
  cancelling: "取消中",
  cancelled: "已取消",
  interrupted: "上次未完成",
};
export const isActive = (s: TaskStatus) =>
  [
    "queued",
    "resolving",
    "downloading",
    "postprocessing",
    "cancelling",
  ].includes(s);
export const isRetryable = (s: TaskStatus) =>
  ["failed", "cancelled", "interrupted"].includes(s);
export function bytes(n: number | null | undefined) {
  if (n == null || !Number.isFinite(n)) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < 4) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(i ? 1 : 0)} ${units[i]}`;
}
export function duration(n: number | null | undefined) {
  if (n == null || !Number.isFinite(n)) return "时长未知";
  const s = Math.max(0, Math.round(n));
  return s >= 3600
    ? `${Math.floor(s / 3600)}:${String(Math.floor(s / 60) % 60).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`
    : `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}
export function validateUrl(value: string): string | null {
  if (!value.trim()) return "请先粘贴一个媒体链接";
  try {
    const u = new URL(value.trim());
    if (!["https:", "http:"].includes(u.protocol) || !u.hostname)
      return "请输入有效的 http 或 https 链接";
    if (u.username || u.password) return "链接中不能包含用户名或密码";
    return null;
  } catch {
    return "请输入完整有效的媒体链接";
  }
}
export { errorText, isStructuredIpcError } from "./errors";
export function date(value: string) {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime())
    ? value
    : parsed.toLocaleString("zh-CN", {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
      });
}
