import { useTaskIds, type TaskListing } from "../stores/selectors";
import { useState } from "react";
import { ListFilter, ListVideo } from "lucide-react";
import { Empty } from "../components/ui";
import { TaskRow } from "../features/downloads/TaskRow";

export function Queue({ onTask }: { onTask: (taskId: string) => void }) {
  const [filter, setFilter] = useState<TaskListing>("active");
  const tasks = useTaskIds("all");
  const active = useTaskIds("active");
  const queued = useTaskIds("queued");
  const visible = useTaskIds(filter);
  return (
    <>
      <div className="page-heading">
        <div>
          <span className="eyebrow">ONE THING AT A TIME</span>
          <h1>下载队列</h1>
          <p>每一份期待，都在有序抵达。</p>
        </div>
        <span className="local-badge">
          <span />
          串行下载 · 并发 1
        </span>
      </div>
      <div className="list-toolbar">
        <div className="tabs">
          {[
            { value: "active" as const, label: "进行中", count: active.length },
            {
              value: "queued" as const,
              label: "等待中",
              count: queued.length,
            },
            { value: "all" as const, label: "全部", count: tasks.length },
          ].map((f) => (
            <button
              className={filter === f.value ? "selected" : ""}
              key={f.value}
              onClick={() => setFilter(f.value)}
            >
              {f.label}
              <span>{f.count}</span>
            </button>
          ))}
        </div>
        <span className="toolbar-caption">
          <ListFilter size={14} />
          按添加顺序
        </span>
      </div>
      {visible.length ? (
        <div className="task-list">
          {visible.map((id) => (
            <TaskRow key={id} taskId={id} onSelect={onTask} />
          ))}
        </div>
      ) : (
        <Empty
          icon={<ListVideo size={30} strokeWidth={1.3} />}
          title={filter === "queued" ? "没有等待中的任务" : "这里暂时很安静"}
          text="从工作台解析链接并加入队列，下载会自动依次开始。"
        />
      )}
    </>
  );
}
