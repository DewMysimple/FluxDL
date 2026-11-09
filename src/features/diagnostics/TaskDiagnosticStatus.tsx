import { useEffect, useState } from "react";
import { isDesktop } from "../../lib/api";
import { statusLabels as taskLabels } from "../../lib/utils";
import { diagnosticsApi } from "./diagnostics-api";
import type { DiagnosticEvent, Health } from "./types";
export function observedStatus(
  events: DiagnosticEvent[],
  sessionId?: string,
): string | null {
  const allBackend = events.filter((e) => e.source === "rust");
  const current = allBackend.filter((e) => e.session_id === sessionId);
  const backend = current.length ? current : allBackend;
  const latest = backend.find(
    (e) =>
      e.phase !== "download_total" &&
      e.phase !== "enqueue_to_progress" &&
      e.status !== "unobserved" &&
      e.status !== "notapplicable",
  );
  if (!latest) return null;
  if (latest.phase === "download_first_output")
    return "引擎已有输出，等待下载前阶段或首次有效进度";
  if (
    latest.phase === "download_before_dl" ||
    latest.phase === "download_before_dl_to_progress"
  )
    return latest.status === "ok" &&
      latest.phase === "download_before_dl_to_progress"
      ? "下载中"
      : "已观察下载前标记，等待首次有效进度";
  if (latest.phase === "download_process_exit")
    return "引擎已退出，正在核验文件与保存最终状态";
  if (latest.phase === "enqueue_trusted_validate")
    return "正在校验后端可信下载选项";
  if (latest.phase === "preview_cache_decision")
    return latest.fields.cache_outcome === "hit"
      ? "可信预览已验证，正在创建任务"
      : "正在安全复核媒体信息";
  if (latest.phase === "persistence_issue")
    return "状态保存出现异常，请查看诊断中心";
  if (latest.phase === "download_process_issue")
    return "下载进程出现异常，请查看诊断中心";
  if (
    sessionId &&
    latest.session_id !== sessionId &&
    latest.phase !== "download_terminal"
  )
    return "上一会话已结束；缺少后续阶段证据";
  if (latest.phase === "download_terminal")
    return latest.status === "started"
      ? "进程已结束，保存最终状态中"
      : latest.status === "failed"
        ? "失败"
        : latest.status === "cancelled"
          ? "已取消"
          : "完成";
  if (latest.phase === "download_postprocess") return "后处理中";
  if (
    latest.phase === "download_active" ||
    (latest.phase === "download_first_progress" && latest.status === "ok")
  )
    return "下载中";
  if (latest.phase === "download_first_progress" && latest.status === "started")
    return "进程运行，等待首次有效进度";
  if (latest.phase === "download_engine_spawn")
    return latest.status === "started"
      ? "启动下载引擎中"
      : latest.status === "failed"
        ? "下载引擎启动失败"
        : "进程已启动";
  if (latest.phase === "queue_wait") return "等待串行队列";
  if (latest.phase === "enqueue_persist") return "保存入队状态中";
  if (latest.phase === "enqueue_db_queue") return "等待SQLite写入队列";
  if (latest.phase === "enqueue_db_write") return "执行SQLite写入事务";
  if (latest.phase === "enqueue_metadata_recheck") return "复核媒体信息中";
  if (latest.phase === "enqueue_validate") return "校验参数中";
  return null;
}

export const LONG_WAIT_SECONDS = 30;
const requests = new Map<
  string,
  {
    at: number;
    value: Promise<[Awaited<ReturnType<typeof diagnosticsApi.query>>, Health]>;
  }
>();
function read(taskId?: string, traceId?: string) {
  const key = taskId ?? traceId ?? "";
  const prior = requests.get(key);
  if (prior && performance.now() - prior.at < 800) return prior.value;
  if (requests.size >= 32) requests.delete(requests.keys().next().value!);
  const value = Promise.all([
    diagnosticsApi.query({ taskId, traceId, limit: 100 }),
    diagnosticsApi.health(),
  ]);
  requests.set(key, { at: performance.now(), value });
  return value;
}
export function waitingMilliseconds(
  events: DiagnosticEvent[],
  health: Health,
): number | null {
  if (health.elapsedSinceSessionStartMs == null) return null;
  const finished = new Set(
    events.filter((e) => e.status !== "started").map((e) => e.span_id),
  );
  const open = events.filter(
    (e) =>
      e.source === "rust" &&
      e.session_id === health.sessionId &&
      e.status === "started" &&
      !finished.has(e.span_id),
  );
  const start =
    open.find((e) => e.phase === "download_first_progress") ??
    open.find(
      (e) => !["download_total", "enqueue_to_progress"].includes(e.phase),
    );
  if (!start) return null;
  const elapsed =
    health.elapsedSinceSessionStartMs - start.elapsed_since_session_start_ms;
  return elapsed >= 0 ? elapsed : null;
}
export function TaskDiagnosticStatus({
  taskId,
  traceId,
  active,
  status: taskStatus,
  compact = false,
  showBusinessState = false,
}: {
  taskId?: string;
  traceId?: string;
  active: boolean;
  status?: string;
  compact?: boolean;
  showBusinessState?: boolean;
}) {
  const [snapshot, setSnapshot] = useState<{
    key: string;
    text: string;
    waited: number | null;
    received: number;
  } | null>(null);
  const [now, setNow] = useState(() => performance.now());
  useEffect(() => {
    if (!isDesktop || !/^[0-9a-f-]{36}$/.test(taskId ?? traceId ?? "")) return;
    if (taskStatus && !["queued", "resolving"].includes(taskStatus)) return;
    let disposed = false,
      busy = false;
    const update = async () => {
      if (busy) return;
      busy = true;
      try {
        const [page, health] = await read(taskId, traceId);
        if (!disposed)
          setSnapshot({
            key: taskId ?? traceId ?? "",
            text:
              observedStatus(page.events, health.sessionId) ??
              "尚无有效阶段记录",
            waited: waitingMilliseconds(page.events, health),
            received: performance.now(),
          });
      } catch {
        if (!disposed)
          setSnapshot({
            key: taskId ?? traceId ?? "",
            text: "阶段记录暂不可用；任务控制仍可使用",
            waited: null,
            received: performance.now(),
          });
      } finally {
        busy = false;
      }
    };
    void update();
    const timer = active
      ? window.setInterval(() => {
          setNow(performance.now());
          void update();
        }, 1000)
      : undefined;
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [taskId, traceId, active, taskStatus]);
  const Tag = compact ? "span" : "p";
  const current = snapshot?.key === (taskId ?? traceId ?? "") ? snapshot : null;
  // Business state wins over stale/missing diagnostic reads.
  if (taskStatus && !["queued", "resolving"].includes(taskStatus))
    return showBusinessState ? (
      <Tag className="diagnostic-observed-status" role="status">
        {(taskLabels as Record<string, string>)[taskStatus] ?? "任务状态未知"}
      </Tag>
    ) : null;
  if (!isDesktop) return null;
  const seconds =
    current?.waited != null
      ? Math.floor(
          (current.waited + Math.max(0, now - current.received)) / 1000,
        )
      : null;
  return (
    <Tag className="diagnostic-observed-status" role="status">
      已观测阶段：{current?.text ?? "正在读取阶段记录…"}
      {active && seconds != null && (
        <>
          {" "}
          · 已等待 {seconds} 秒
          {seconds >= LONG_WAIT_SECONDS && " · 此步骤等待较久，可查看诊断"}
        </>
      )}
    </Tag>
  );
}
