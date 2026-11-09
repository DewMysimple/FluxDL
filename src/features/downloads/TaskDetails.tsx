import { useEffect, useState } from "react";
import {
  ChevronDown,
  CircleAlert,
  Copy,
  ExternalLink,
  FolderOpen,
  RefreshCw,
  Terminal,
  X,
  Activity,
} from "lucide-react";
import { api, isDesktop } from "../../lib/api";
import { bytes, date, isActive, isRetryable } from "../../lib/utils";
import { useApp } from "../../stores/app";
import { Button, MediaImage, Status, Progress } from "../../components/ui";
import { useAction } from "../../hooks/useAction";
import { qualityName } from "../../lib/presets";
import { TaskDiagnosticStatus } from "../diagnostics/TaskDiagnosticStatus";

export function TaskDetails({ taskId }: { taskId: string }) {
  const task = useApp((s) => s.tasksById[taskId]);
  const { busy, run } = useAction();
  const [copied, setCopied] = useState(false);
  const mergeDetails = useApp((s) => s.mergeDetails);
  const notify = useApp((s) => s.notify);
  useEffect(() => {
    if (!isDesktop) return;
    let disposed = false;
    void api
      .details(taskId)
      .then((result) => {
        if (!disposed) {
          if (result.task.id === taskId) mergeDetails(result);
          else notify("任务详情与请求不匹配，请重新打开详情");
        }
      })
      .catch((error) => {
        if (!disposed) notify(error);
      });
    return () => {
      disposed = true;
    };
  }, [taskId, mergeDetails, notify]);
  const copy = async (value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch (error) {
      useApp.getState().notify(error);
    }
  };
  if (!task) return null;
  return (
    <div className="task-details">
      <MediaImage media={task.media} className="detail-image" />
      <span className="eyebrow">{task.media.source}</span>
      <h2>{task.media.title}</h2>
      <Status task={task} />
      <TaskDiagnosticStatus taskId={task.id} active={isActive(task.status)} />
      <Progress task={task} />
      <dl className="details-list">
        <div>
          <dt>下载方式</dt>
          <dd>
            {task.mode === "audio" ? "MP3 音频" : qualityName(task.quality)}
          </dd>
        </div>
        <div>
          <dt>已下载</dt>
          <dd>
            {bytes(task.downloadedBytes)}
            {task.totalBytes ? ` / ${bytes(task.totalBytes)}` : ""}
          </dd>
        </div>
        {task.speed != null && (
          <div>
            <dt>速度</dt>
            <dd>{bytes(task.speed)}/s</dd>
          </div>
        )}
        <div>
          <dt>创建时间</dt>
          <dd>{date(task.createdAt)}</dd>
        </div>
        <div>
          <dt>更新时间</dt>
          <dd>{date(task.updatedAt)}</dd>
        </div>
        <div className="vertical">
          <dt>保存位置</dt>
          <dd>{task.finalPath || task.outputDir}</dd>
        </div>
      </dl>
      {task.error && (
        <div className="notice danger">
          <CircleAlert size={17} />
          <span>{task.error}</span>
        </div>
      )}
      {task.status === "completed" && !task.fileExists && (
        <div className="notice warning">
          <CircleAlert size={17} />
          <span>本地文件不存在，可能已被移动或删除。下载历史仍然保留。</span>
        </div>
      )}
      <div className="detail-actions">
        <Button
          disabled={!isDesktop}
          onClick={() => void run(() => api.locateDiagnostics(task.id))}
        >
          <Activity size={15} />
          查看任务诊断
        </Button>
        {task.status === "completed" && (
          <>
            <Button
              disabled={!task.fileExists || busy}
              className="primary"
              onClick={() => void run(() => api.open(task.id))}
            >
              <ExternalLink size={16} />
              打开文件
            </Button>
            <Button
              disabled={!task.fileExists || busy}
              onClick={() => void run(() => api.reveal(task.id))}
            >
              <FolderOpen size={16} />
              所在目录
            </Button>
          </>
        )}
        {isActive(task.status) && (
          <Button
            disabled={busy || task.status === "cancelling"}
            onClick={() => void run(() => api.cancel(task.id))}
          >
            <X size={16} />
            取消任务
          </Button>
        )}
        {isRetryable(task.status) && (
          <Button
            className="primary"
            disabled={busy}
            onClick={() => void run(() => api.retry(task.id))}
          >
            <RefreshCw size={16} />
            重新下载
          </Button>
        )}
        <Button onClick={() => void copy(task.media.url)}>
          <Copy size={15} />
          {copied ? "已复制" : "复制来源链接"}
        </Button>
      </div>
      <details className="logs">
        <summary>
          <Terminal size={15} />
          技术诊断与日志
          <ChevronDown size={15} />
        </summary>
        <div className="log-copy">
          <Button onClick={() => void copy(task.logs.join("\n"))}>
            <Copy size={13} />
            复制日志
          </Button>
        </div>
        <pre>{task.logs.length ? task.logs.join("\n") : "暂无日志"}</pre>
      </details>
    </div>
  );
}
