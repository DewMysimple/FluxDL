import { memo, useState } from "react";
import { useTaskIds } from "../stores/selectors";
import { Folder, History, Search, Trash2, X } from "lucide-react";
import { api } from "../lib/api";
import { type DownloadTask } from "../lib/types";
import { useApp } from "../stores/app";
import { IconButton, Button, Empty } from "../components/ui";
import { Modal } from "../components/overlays";
import { useAction } from "../hooks/useAction";
import { TaskRow } from "../features/downloads/TaskRow";

export function HistoryPage({ onTask }: { onTask: (taskId: string) => void }) {
  const [query, setQuery] = useState("");
  const tasks = useTaskIds("history");
  const visible = useTaskIds("history", query);
  const [remove, setRemove] = useState<DownloadTask | null>(null);
  const { busy, run } = useAction();
  return (
    <>
      <div className="page-heading">
        <div>
          <span className="eyebrow">SAVED FOR LATER</span>
          <h1>下载历史</h1>
          <p>已经抵达的精彩，都有迹可循。</p>
        </div>
        <span className="quiet-caption">{tasks.length} 个已完成</span>
      </div>
      <div className="list-toolbar">
        <label className="search-field">
          <Search size={16} />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="搜索标题或来源…"
            aria-label="搜索下载历史"
          />
          {query && (
            <IconButton label="清除搜索" onClick={() => setQuery("")}>
              <X size={14} />
            </IconButton>
          )}
        </label>
        <span className="toolbar-caption">最近完成优先</span>
      </div>
      {visible.length ? (
        <div className="history-list">
          {visible.map((id) => (
            <HistoryRecord
              key={id}
              taskId={id}
              onTask={onTask}
              onRemove={setRemove}
            />
          ))}
        </div>
      ) : (
        <Empty
          icon={<History size={30} strokeWidth={1.3} />}
          title={query ? "没有找到相关下载" : "让喜欢的内容留下来"}
          text={
            query
              ? "试试其他标题或来源关键词。"
              : "成功完成的下载会自动保存为历史，关闭应用后也不会丢失。"
          }
        />
      )}
      {remove && (
        <Modal title="删除这条历史记录？" onClose={() => setRemove(null)}>
          <p className="modal-description">
            仅删除「{remove.media.title}」的记录，本地文件会保留。
          </p>
          <div className="modal-actions">
            <Button onClick={() => setRemove(null)}>取消</Button>
            <Button
              className="danger-button"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  await api.remove(remove.id);
                  setRemove(null);
                })
              }
            >
              删除记录
            </Button>
          </div>
        </Modal>
      )}
    </>
  );
}

const HistoryRecord = memo(function HistoryRecord({
  taskId,
  onTask,
  onRemove,
}: {
  taskId: string;
  onTask: (taskId: string) => void;
  onRemove: (task: DownloadTask) => void;
}) {
  const task = useApp((s) => s.tasksById[taskId]);
  if (!task) return null;
  return (
    <div className="history-record">
      <TaskRow taskId={taskId} onSelect={onTask} history />
      <div className="history-extra">
        <span title={task.finalPath || task.outputDir}>
          <Folder size={12} />
          {task.finalPath || task.outputDir}
        </span>
        <button onClick={() => onRemove(task)}>
          <Trash2 size={13} />
          删除记录
        </button>
      </div>
    </div>
  );
});
