import { useCallback, useEffect, useState } from "react";
import { version as appVersion } from "../../package.json";
import {
  ChevronRight,
  HardDrive,
  LoaderCircle,
  Maximize2,
  Minus,
  Monitor,
  Plus,
  X,
  Zap,
  Activity,
} from "lucide-react";
import { api, isDesktop } from "../lib/api";
import type { MediaInfo } from "../lib/types";
import { useApp } from "../stores/app";
import { useTaskIds } from "../stores/selectors";
import { useAppSync } from "../hooks/useAppSync";
import { Brand, Button } from "../components/ui";
import { Drawer, Modal } from "../components/overlays";
import { MediaOptions } from "../features/capture/MediaOptions";
import { TaskDetails } from "../features/downloads/TaskDetails";
import { Home } from "../pages/Home";
import { Queue } from "../pages/Queue";
import { HistoryPage } from "../pages/History";
import { SettingsPage } from "../pages/Settings";
import { DiagnosticsPage } from "../features/diagnostics/DiagnosticsPage";
import { type Page, pages } from "../app/navigation";
export function WorkspaceWindow() {
  const [page, setPage] = useState<Page>("home");
  const [media, setMedia] = useState<MediaInfo | null>(null);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [closing, setClosing] = useState(false);
  const [exiting, setExiting] = useState(false);
  const [diagnosticTask, setDiagnosticTask] = useState("");
  const engines = useApp((s) => s.engines);
  const loading = useApp((s) => s.loading);
  const notify = useApp((s) => s.notify);
  const activeCount = useTaskIds("active").length;
  const hasSelectedTask = useApp((s) => Boolean(taskId && s.tasksById[taskId]));
  const showClose = useCallback(() => setClosing(true), []);
  const shutdownFailed = useCallback(() => setExiting(false), []);
  useAppSync({ onClose: showClose, onShutdownFailed: shutdownFailed });
  const navigate = useCallback((next: Page) => {
    setPage(next);
    setMedia(null);
    setTaskId(null);
  }, []);
  useEffect(() => {
    if (!isDesktop) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void api
      .onDiagnosticsTask((id) => {
        setDiagnosticTask(id);
        setPage("diagnostics");
        setTaskId(null);
        setMedia(null);
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else stop = unlisten;
      })
      .catch(notify);
    return () => {
      disposed = true;
      stop?.();
    };
  }, [notify]);
  useEffect(() => {
    const handle = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setMedia(null);
        setTaskId(null);
      }
    };
    window.addEventListener("keydown", handle);
    return () => window.removeEventListener("keydown", handle);
  }, []);
  const windowAction = (action: "close" | "minimize" | "maximize") => {
    void api.window(action).catch(notify);
  };
  const exit = async () => {
    setExiting(true);
    try {
      await api.exit();
    } catch (error) {
      notify(error);
      setExiting(false);
    }
  };
  return (
    <>
      <header className="titlebar" data-tauri-drag-region>
        <span className="titlebar-name" data-tauri-drag-region>
          FluxDL
          <span className="titlebar-divider" />
          Workspace
        </span>
        <div className="window-controls">
          <button
            aria-label="最小化窗口"
            onClick={() => windowAction("minimize")}
          >
            <Minus size={15} />
          </button>
          <button
            aria-label="最大化或还原窗口"
            onClick={() => windowAction("maximize")}
          >
            <Maximize2 size={12} />
          </button>
          <button
            className="window-close"
            aria-label="关闭窗口"
            onClick={() => windowAction("close")}
          >
            <X size={16} />
          </button>
        </div>
      </header>
      <div className="app-body">
        <aside className="sidebar">
          <div className="sidebar-brand">
            <Brand />
            <span className="brand-tagline">留住每一份精彩</span>
          </div>
          <span className="nav-caption">你的空间</span>
          <nav>
            {pages.map((p) => (
              <button
                className={page === p.id ? "active" : ""}
                onClick={() => navigate(p.id)}
                key={p.id}
                title={p.label}
              >
                <p.icon size={18} strokeWidth={1.65} />
                <span>{p.label}</span>
                {p.id === "queue" && activeCount > 0 && (
                  <small>{activeCount}</small>
                )}
                {page === p.id && <i />}
              </button>
            ))}
          </nav>
          <div className="sidebar-bottom">
            <button
              className={`diagnostic-nav ${page === "diagnostics" ? "active" : ""}`}
              onClick={() => {
                setDiagnosticTask("");
                navigate("diagnostics");
              }}
              title="诊断中心"
            >
              <Activity size={18} />
              诊断中心
            </button>
            <button
              className="quick-launch"
              title="打开快速捕获"
              onClick={() => void api.quick().catch(notify)}
            >
              <span>
                <Zap size={17} />
              </span>
              <span>
                快速捕获<small>随手粘贴，轻松收藏</small>
              </span>
              <Plus size={15} />
            </button>
            <button
              className="engine-status"
              onClick={() => navigate("settings")}
              title="查看引擎设置"
            >
              <span
                className={`engine-dot ${engines.ytdlp.available ? "ready" : ""}`}
              />
              <span>
                {engines.ytdlp.available ? "引擎已就绪" : "引擎待配置"}
                <small>
                  {engines.ytdlp.available
                    ? `yt-dlp ${engines.ytdlp.version || ""}`
                    : "选择本地 yt-dlp"}
                </small>
              </span>
              <ChevronRight size={13} />
            </button>
            <div className="sidebar-version">
              <span>本地 · 私有 · 自由</span>
              <span>v{appVersion.split(".").slice(0, 2).join(".")}</span>
            </div>
          </div>
        </aside>
        <main className="main-content">
          <div className="page-content">
            {!isDesktop && (
              <div className="preview-banner">
                <Monitor size={14} />
                浏览器界面预览 · 下载与系统功能请在桌面应用中使用
              </div>
            )}
            {loading ? (
              <div className="loading-state">
                <LoaderCircle size={25} className="spin" />
                <span>正在连接本地工作台…</span>
              </div>
            ) : page === "home" ? (
              <Home onMedia={setMedia} onTask={setTaskId} go={navigate} />
            ) : page === "queue" ? (
              <Queue onTask={setTaskId} />
            ) : page === "history" ? (
              <HistoryPage onTask={setTaskId} />
            ) : page === "diagnostics" ? (
              <DiagnosticsPage
                key={diagnosticTask}
                initialTask={diagnosticTask}
                onTask={setTaskId}
              />
            ) : (
              <SettingsPage />
            )}
          </div>
        </main>
      </div>
      <footer className="statusbar">
        <span>
          <HardDrive size={12} />
          文件保存在您的设备上
        </span>
        <span>
          {activeCount ? `${activeCount} 个活动任务` : "一切就绪"}
          <span className="statusbar-dot" />
          FluxDL
        </span>
      </footer>
      {media && (
        <Drawer title="解析结果" onClose={() => setMedia(null)}>
          <MediaOptions key={media.url} media={media} onAdded={() => {}} />
        </Drawer>
      )}
      {taskId && hasSelectedTask && !media && (
        <Drawer title="任务详情" onClose={() => setTaskId(null)}>
          <TaskDetails taskId={taskId} />
        </Drawer>
      )}
      {closing && (
        <Modal title="下载仍在进行中" onClose={() => setClosing(false)}>
          <p className="modal-description">
            当前有 {activeCount} 个活动任务。您可以关闭窗口，让 FluxDL
            在系统托盘中继续下载。
          </p>
          <div className="close-choices">
            <Button
              className="primary"
              disabled={exiting}
              onClick={() => {
                void api
                  .window("hide")
                  .then(() => setClosing(false))
                  .catch(notify);
              }}
            >
              <Minus size={16} />
              在后台继续
            </Button>
            <Button
              className="danger-button"
              disabled={exiting}
              onClick={() => void exit()}
            >
              {exiting ? (
                <LoaderCircle size={16} className="spin" />
              ) : (
                <X size={16} />
              )}
              {exiting ? "正在安全停止…" : "停止任务并退出"}
            </Button>
            <Button disabled={exiting} onClick={() => setClosing(false)}>
              取消
            </Button>
          </div>
          <p className="modal-footnote">
            后台下载时，可通过系统托盘重新打开工作台。
          </p>
        </Modal>
      )}
    </>
  );
}
