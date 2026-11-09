import { useEffect, useState } from "react";
import { isDesktop } from "../../lib/api";
import { diagnosticsApi } from "./diagnostics-api";
import type { DiagnosticEvent } from "./types";
import { latestThroughput, TaskThroughput } from "./TaskThroughput";
export function observedStatus(
  events: DiagnosticEvent[],
  sessionId?: string,
): string | null {
  const backend = events.filter((e) => e.source === "rust");
  const latest = backend.find(
    (e) =>
      e.phase !== "download_total" &&
      e.phase !== "download_throughput" &&
      e.phase !== "enqueue_to_progress" &&
      e.status !== "unobserved" &&
      e.status !== "notapplicable",
  );
  if (!latest) return null;
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
export function TaskDiagnosticStatus({
  taskId,
  traceId,
  active,
}: {
  taskId?: string;
  traceId?: string;
  active: boolean;
}) {
  const target = taskId ?? traceId ?? "";
  const [observation, setObservation] = useState<{
    target: string;
    events: DiagnosticEvent[];
    sessionId: string;
  }>();
  useEffect(() => {
    if (!isDesktop || !/^[0-9a-f-]{36}$/.test(taskId ?? traceId ?? "")) return;
    let disposed = false;
    let busy = false;
    const update = async () => {
      if (busy) return;
      busy = true;
      try {
        const [page, health] = await Promise.all([
          diagnosticsApi.query({
            taskId,
            traceId,
            stageView: true,
            limit: 100,
          }),
          diagnosticsApi.health(),
        ]);
        if (!disposed)
          setObservation({
            target: taskId ?? traceId ?? "",
            events: page.events,
            sessionId: health.sessionId,
          });
      } catch {
        /* Diagnostic failure must not disrupt task controls. */
      } finally {
        busy = false;
      }
    };
    void update();
    const timer = active
      ? window.setInterval(() => void update(), 1000)
      : undefined;
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [taskId, traceId, active]);
  const current = observation?.target === target ? observation : undefined;
  const status = observedStatus(current?.events ?? [], current?.sessionId);
  return (
    <>
      {status && (
        <p className="diagnostic-observed-status" role="status">
          已观测阶段：{status}
        </p>
      )}
      {taskId && (
        <TaskThroughput
          event={latestThroughput(current?.events ?? [])}
          sessionId={current?.sessionId}
        />
      )}
    </>
  );
}
