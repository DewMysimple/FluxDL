import { useTaskIds } from "../stores/selectors";
import {
  ArrowDownToLine,
  ArrowRight,
  Clock3,
  ShieldCheck,
  Terminal,
  Zap,
} from "lucide-react";
import { api } from "../lib/api";
import { type MediaInfo } from "../lib/types";
import { useApp } from "../stores/app";
import { Button } from "../components/ui";
import { CaptureInput } from "../features/capture/CaptureInput";
import { TaskRow } from "../features/downloads/TaskRow";
import { type Page } from "../app/navigation";

export function Home({
  onMedia,
  onTask,
  go,
}: {
  onMedia: (media: MediaInfo) => void;
  onTask: (taskId: string) => void;
  go: (page: Page) => void;
}) {
  const engines = useApp((s) => s.engines);
  const active = useTaskIds("active");
  const recent = useTaskIds("completed").slice(0, 3);
  return (
    <>
      <div className="page-heading">
        <div>
          <span className="eyebrow">YOUR MEDIA, YOUR SPACE</span>
          <h1>
            下载工作台<span className="heading-dot">.</span>
          </h1>
          <p>收藏值得留下的内容，从一个链接开始。</p>
        </div>
        <Button
          className="quiet"
          onClick={() => void api.quick().catch(useApp.getState().notify)}
        >
          <Zap size={15} />
          快速捕获<span className="desktop-shortcut">独立浮窗</span>
        </Button>
      </div>
      <CaptureInput onMedia={onMedia} />
      {!engines.ytdlp.available && (
        <button className="setup-banner" onClick={() => go("settings")}>
          <span className="setup-icon">
            <Terminal size={18} />
          </span>
          <span>
            <strong>连接您的下载引擎</strong>
            <small>自动检测 PATH，或选择本地 yt-dlp，即可开始。</small>
          </span>
          <span className="setup-link">
            前往设置
            <ArrowRight size={15} />
          </span>
        </button>
      )}
      <section className="task-section">
        <div className="section-heading">
          <h2>
            当前任务
            {active.length > 0 && (
              <span className="count">{active.length}</span>
            )}
          </h2>
          <button className="text-button" onClick={() => go("queue")}>
            任务中心
            <ArrowRight size={14} />
          </button>
        </div>
        {active.length ? (
          <div className="task-list">
            {active.slice(0, 3).map((id) => (
              <TaskRow key={id} taskId={id} onSelect={onTask} />
            ))}
          </div>
        ) : (
          <div className="resting-state">
            <span className="resting-art">
              <ArrowDownToLine size={28} strokeWidth={1.25} />
              <i />
              <i />
            </span>
            <div>
              <strong>队列很轻，准备好出发</strong>
              <p>在上方粘贴链接，解析后选择喜欢的格式。</p>
            </div>
            <span className="resting-label">READY WHEN YOU ARE</span>
          </div>
        )}
      </section>
      <section className="task-section recent-section">
        <div className="section-heading">
          <h2>最近完成</h2>
          <button className="text-button" onClick={() => go("history")}>
            查看历史
            <ArrowRight size={14} />
          </button>
        </div>
        {recent.length ? (
          <div className="task-list">
            {recent.map((id) => (
              <TaskRow key={id} taskId={id} onSelect={onTask} history />
            ))}
          </div>
        ) : (
          <div className="history-placeholder">
            <Clock3 size={18} />
            <span>完成的下载会留在这里，随时再次找到。</span>
          </div>
        )}
      </section>
      <div className="workspace-note">
        <ShieldCheck size={14} />
        <span>本地优先。无账号，无遥测，文件始终属于您。</span>
      </div>
    </>
  );
}
