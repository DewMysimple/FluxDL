import { memo } from "react";
import { useApp } from "../../stores/app";
import { FolderOpen, MoreHorizontal, RefreshCw, X } from "lucide-react";
import { api } from "../../lib/api";
import { bytes, date, duration, isActive, isRetryable } from "../../lib/utils";
import { IconButton, MediaImage, Status, Progress } from "../../components/ui";
import { useAction } from "../../hooks/useAction";
import { qualityName } from "../../lib/presets";

export const TaskRow = memo(function TaskRow({
  taskId,
  onSelect,
  history = false,
}: {
  taskId: string;
  onSelect: (taskId: string) => void;
  history?: boolean;
}) {
  const { busy, run } = useAction();
  const task = useApp((s) => s.tasksById[taskId]);
  if (!task) return null;
  const act = (fn: () => Promise<unknown>) => {
    void run(fn);
  };
  return (
    <div className={`task-row ${busy ? "busy" : ""}`}>
      <button
        className="task-main"
        onClick={() => onSelect(taskId)}
        aria-label={`查看任务：${task.media.title}`}
      >
        <MediaImage media={task.media} />
        <span className="task-copy">
          <strong title={task.media.title}>{task.media.title}</strong>
          <span className="task-meta">
            <span>
              {task.mode === "audio"
                ? "音频"
                : task.formatId
                  ? "指定格式"
                  : qualityName(task.quality)}
            </span>
            <i /> <span>{task.media.source || "媒体文件"}</span>
            {history && (
              <>
                <i />
                <span>{date(task.updatedAt)}</span>
              </>
            )}
          </span>
          {task.status === "downloading" && (
            <>
              <Progress task={task} />
              <span className="task-transfer">
                {bytes(task.downloadedBytes)}
                {task.totalBytes ? ` / ${bytes(task.totalBytes)}` : ""}
                <span>
                  {task.speed != null ? `${bytes(task.speed)}/s` : "正在连接"}
                  {task.eta != null ? ` · 剩余 ${duration(task.eta)}` : ""}
                </span>
              </span>
            </>
          )}
        </span>
      </button>
      <div className="task-state">
        <Status task={task} />
        {task.status === "downloading" && task.progress != null && (
          <span className="percent">{task.progress.toFixed(1)}%</span>
        )}
        {history && !task.fileExists && (
          <small className="warning-text">本地文件不存在</small>
        )}
      </div>
      <div className="task-actions">
        {isActive(task.status) && task.status !== "cancelling" && (
          <IconButton
            label="取消下载"
            disabled={busy}
            onClick={() => act(() => api.cancel(task.id))}
          >
            <X size={16} />
          </IconButton>
        )}
        {isRetryable(task.status) && (
          <IconButton
            label="重试下载"
            disabled={busy}
            onClick={() => act(() => api.retry(task.id))}
          >
            <RefreshCw size={16} />
          </IconButton>
        )}
        {task.status === "completed" && (
          <IconButton
            label="打开所在目录"
            disabled={busy || !task.fileExists}
            onClick={() => act(() => api.reveal(task.id))}
          >
            <FolderOpen size={16} />
          </IconButton>
        )}
        <IconButton label="任务详情" onClick={() => onSelect(taskId)}>
          <MoreHorizontal size={18} />
        </IconButton>
      </div>
    </div>
  );
});
