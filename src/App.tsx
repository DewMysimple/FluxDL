import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import {
  ArrowDown,
  ArrowDownToLine,
  ArrowRight,
  AudioLines,
  Check,
  CheckCheck,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Clipboard,
  Clock3,
  Copy,
  Download,
  ExternalLink,
  FileVideo,
  Folder,
  FolderOpen,
  HardDrive,
  History,
  LayoutDashboard,
  Link2,
  ListFilter,
  ListVideo,
  LoaderCircle,
  Maximize2,
  Minus,
  Monitor,
  Moon,
  MoreHorizontal,
  Plus,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
  Sparkles,
  SquareArrowOutUpRight,
  Sun,
  Terminal,
  Trash2,
  Video,
  X,
  Zap,
} from "lucide-react";
import { api, isDesktop } from "./lib/api";
import type {
  DownloadTask,
  EngineInfo,
  MediaInfo,
  Settings,
} from "./lib/types";
import {
  bytes,
  date,
  duration,
  errorText,
  isActive,
  isRetryable,
  statusLabels,
  validateUrl,
} from "./lib/utils";
import { useApp } from "./stores/app";

type Page = "home" | "queue" | "history" | "settings";
const pages = [
  { id: "home" as const, label: "工作台", icon: LayoutDashboard },
  { id: "queue" as const, label: "下载队列", icon: ListVideo },
  { id: "history" as const, label: "下载历史", icon: History },
  { id: "settings" as const, label: "设置", icon: Settings2 },
];
const qualityOptions = [
  { value: "best", label: "最佳可用画质" },
  { value: "2160", label: "最高 4K · 2160p" },
  { value: "1440", label: "最高 2K · 1440p" },
  { value: "1080", label: "最高 1080p" },
  { value: "720", label: "最高 720p" },
  { value: "480", label: "最高 480p" },
];
const qualityName = (value: string) =>
  qualityOptions.find((q) => q.value === value)?.label ?? value;

function Brand({ small = false }: { small?: boolean }) {
  return (
    <div className={`brand ${small ? "small" : ""}`}>
      <span className="brand-mark">
        <ArrowDown size={small ? 18 : 23} strokeWidth={2.5} />
        <span />
      </span>
      <span>
        FluxDL<span className="brand-dot">.</span>
      </span>
    </div>
  );
}
function IconButton({
  label,
  children,
  onClick,
  disabled = false,
  className = "",
}: {
  label: string;
  children: ReactNode;
  onClick: () => void;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <button
      type="button"
      className={`icon-button ${className}`}
      aria-label={label}
      title={label}
      onClick={onClick}
      disabled={disabled}
    >
      {children}
    </button>
  );
}
function Button({
  children,
  onClick,
  className = "",
  disabled = false,
  type = "button",
}: {
  children: ReactNode;
  onClick?: () => void;
  className?: string;
  disabled?: boolean;
  type?: "button" | "submit";
}) {
  return (
    <button
      type={type}
      onClick={onClick}
      className={`button ${className}`}
      disabled={disabled}
    >
      {children}
    </button>
  );
}
function Empty({
  icon,
  title,
  text,
  children,
}: {
  icon: ReactNode;
  title: string;
  text: string;
  children?: ReactNode;
}) {
  return (
    <div className="empty">
      <span className="empty-icon">{icon}</span>
      <h3>{title}</h3>
      <p>{text}</p>
      {children}
    </div>
  );
}
function MediaImage({
  media,
  className = "",
}: {
  media: MediaInfo;
  className?: string;
}) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [media.thumbnail]);
  const valid = media.thumbnail && /^https?:\/\//i.test(media.thumbnail);
  return (
    <div className={`media-image ${className}`}>
      {valid && !failed ? (
        <img
          src={media.thumbnail!}
          alt=""
          referrerPolicy="no-referrer"
          onError={() => setFailed(true)}
        />
      ) : (
        <FileVideo size={26} strokeWidth={1.4} />
      )}
    </div>
  );
}
function Status({ task }: { task: DownloadTask }) {
  return (
    <span className={`status status-${task.status}`}>
      <span />
      {statusLabels[task.status]}
    </span>
  );
}
function Progress({ task }: { task: DownloadTask }) {
  if (task.status !== "downloading") return null;
  return (
    <div
      className={`progress-track ${task.progress == null ? "indeterminate" : ""}`}
      aria-label="下载进度"
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={task.progress ?? undefined}
    >
      <span
        style={
          task.progress == null
            ? undefined
            : { width: `${Math.min(100, Math.max(0, task.progress))}%` }
        }
      />
    </div>
  );
}

function useAction() {
  const [busy, setBusy] = useState(false);
  const notify = useApp((s) => s.notify);
  const refresh = useApp((s) => s.refresh);
  const run = async (action: () => Promise<unknown>, shouldRefresh = true) => {
    if (busy) return;
    setBusy(true);
    try {
      await action();
      if (shouldRefresh) await refresh();
    } catch (error) {
      notify(error);
    } finally {
      setBusy(false);
    }
  };
  return { busy, run };
}
function TaskRow({
  task,
  onSelect,
  history = false,
}: {
  task: DownloadTask;
  onSelect: (task: DownloadTask) => void;
  history?: boolean;
}) {
  const { busy, run } = useAction();
  const act = (fn: () => Promise<unknown>) => {
    void run(fn);
  };
  return (
    <div className={`task-row ${busy ? "busy" : ""}`}>
      <button
        className="task-main"
        onClick={() => onSelect(task)}
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
        <IconButton label="任务详情" onClick={() => onSelect(task)}>
          <MoreHorizontal size={18} />
        </IconButton>
      </div>
    </div>
  );
}

function CaptureInput({
  onMedia,
  quick = false,
}: {
  onMedia: (media: MediaInfo) => void;
  quick?: boolean;
}) {
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const settings = useApp((s) => s.snapshot.settings);
  const parse = async (e?: FormEvent) => {
    e?.preventDefault();
    if (busy) return;
    const invalid = validateUrl(url);
    if (invalid) {
      setError(invalid);
      input.current?.focus();
      return;
    }
    setError(null);
    setBusy(true);
    try {
      onMedia(await api.probe(url.trim()));
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };
  const paste = async () => {
    try {
      setUrl(await navigator.clipboard.readText());
      setError(null);
      input.current?.focus();
    } catch {
      setError("无法读取剪贴板，请使用 Ctrl + V 粘贴链接");
    }
  };
  return (
    <div className={`capture-panel ${quick ? "quick-input" : ""}`}>
      <form noValidate onSubmit={parse}>
        <span className="input-link-icon">
          <Link2 size={21} />
        </span>
        <input
          ref={input}
          autoFocus={quick}
          value={url}
          onChange={(e) => {
            setUrl(e.target.value);
            setError(null);
          }}
          placeholder="粘贴视频或音频链接…"
          aria-label="媒体链接"
          type="url"
          required={false}
          disabled={busy}
        />
        <IconButton
          label="从剪贴板粘贴"
          onClick={() => void paste()}
          disabled={busy}
        >
          <Clipboard size={17} />
        </IconButton>
        <Button type="submit" className="primary parse-button" disabled={busy}>
          {busy ? (
            <LoaderCircle size={16} className="spin" />
          ) : (
            <Sparkles size={16} />
          )}
          {busy ? "正在解析" : "解析链接"}
          {!busy && <span className="keycap">↵</span>}
        </Button>
      </form>
      <div className="capture-foot">
        <span>
          <span className="tiny-dot" />
          {settings.defaultMode === "audio"
            ? "仅音频 · MP3"
            : `视频 · ${qualityName(settings.quality)}`}
          <span className="subtle-separator">/</span>保存到本地
        </span>
        <span className="paste-hint">
          <kbd>Ctrl</kbd> + <kbd>V</kbd> 快速粘贴
        </span>
      </div>
      {error && (
        <div className="inline-error" role="alert">
          <CircleAlert size={16} />
          <span>{error}</span>
        </div>
      )}
      {busy && (
        <div className="probe-loading">
          <span />
          <span />
          <span />
        </div>
      )}
    </div>
  );
}

function MediaOptions({
  media,
  onAdded,
}: {
  media: MediaInfo;
  onAdded: () => void;
}) {
  const snapshot = useApp((s) => s.snapshot);
  const notify = useApp((s) => s.notify);
  const [mode, setMode] = useState(snapshot.settings.defaultMode);
  const [quality, setQuality] = useState(snapshot.settings.quality);
  const [format, setFormat] = useState("");
  const [directory, setDirectory] = useState(snapshot.settings.downloadDir);
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false);
  const [added, setAdded] = useState(false);
  const ffmpeg =
    snapshot.engines.ffmpeg.available && snapshot.engines.ffprobe.available;
  const selected = media.formats.find((f) => f.formatId === format);
  const needsFfmpeg = mode === "audio" || !!selected?.requiresFfmpeg;
  const blocked =
    media.isPlaylist || !media.formats.length || (needsFfmpeg && !ffmpeg);
  const formats = media.formats.filter((f) =>
    mode === "video" ? f.hasVideo : f.hasAudio && !f.hasVideo,
  );
  const enqueue = async () => {
    if (busy || blocked) return;
    setBusy(true);
    try {
      await api.enqueue({
        media,
        mode,
        quality,
        formatId: format || null,
        outputDir: directory || null,
      });
      await useApp.getState().refresh();
      setAdded(true);
      onAdded();
    } catch (error) {
      notify(error);
    } finally {
      setBusy(false);
    }
  };
  const choose = async () => {
    try {
      const path = await api.selectPath(true, "选择保存目录");
      if (path) setDirectory(path);
    } catch (error) {
      notify(error);
    }
  };
  return (
    <div className="media-options">
      <div className="media-preview">
        <MediaImage media={media} />
        <span className="duration-chip">{duration(media.duration)}</span>
      </div>
      <div className="media-heading">
        <span className="eyebrow">{media.source || "媒体信息"}</span>
        <h2>{media.title}</h2>
        <span className="muted small-text">
          {media.formats.length} 个可用格式 · {duration(media.duration)}
        </span>
      </div>
      {media.isPlaylist && (
        <div className="notice warning">
          <CircleAlert size={17} />
          <span>当前 MVP 不执行整列表批量下载，请使用单个视频的链接。</span>
        </div>
      )}
      {media.warnings.map((warning, i) => (
        <div className="notice warning" key={i}>
          <CircleAlert size={16} />
          <span>{warning}</span>
        </div>
      ))}
      <div className="segment mode-segment">
        <button
          className={mode === "video" ? "selected" : ""}
          onClick={() => {
            setMode("video");
            setFormat("");
          }}
        >
          <Video size={16} />
          视频
        </button>
        <button
          className={mode === "audio" ? "selected" : ""}
          onClick={() => {
            setMode("audio");
            setFormat("");
          }}
        >
          <AudioLines size={16} />
          仅音频
        </button>
      </div>
      {mode === "video" ? (
        <label className="field">
          <span>画质预设</span>
          <select
            value={quality}
            onChange={(e) => setQuality(e.target.value)}
            disabled={!!format}
          >
            {qualityOptions.map((q) => (
              <option value={q.value} key={q.value}>
                {q.label}
              </option>
            ))}
          </select>
          <small>优先 MP4；根据来源选择实际可用的画质。</small>
        </label>
      ) : (
        <div className="audio-note">
          <AudioLines size={19} />
          <span>
            <strong>MP3 音频</strong>
            <small>由 FFmpeg 提取并转换音轨</small>
          </span>
        </div>
      )}
      {!ffmpeg && (
        <div className="notice warning">
          <CircleAlert size={17} />
          <span>
            {mode === "audio"
              ? "FFmpeg / FFprobe 未就绪，MP3 转换暂不可用。请前往设置配置。"
              : "FFmpeg / FFprobe 未就绪，将选择包含音轨的直接视频格式，画质可能受限。"}
          </span>
        </div>
      )}
      <button
        className="disclosure"
        onClick={() => setAdvanced(!advanced)}
        aria-expanded={advanced}
      >
        <span>高级格式选择</span>
        <ChevronDown size={15} className={advanced ? "rotate" : ""} />
      </button>
      {advanced && (
        <label className="field">
          <span>真实可用格式</span>
          <select value={format} onChange={(e) => setFormat(e.target.value)}>
            <option value="">自动选择（推荐）</option>
            {formats.map((f) => (
              <option
                key={f.formatId}
                value={f.formatId}
                disabled={f.requiresFfmpeg && !ffmpeg}
              >
                {f.label} · {f.ext}
                {f.filesize ? ` · ${bytes(f.filesize)}` : ""}
                {f.requiresFfmpeg ? " · 需要 FFmpeg" : ""}
              </option>
            ))}
          </select>
        </label>
      )}
      <div className="save-location">
        <Folder size={17} />
        <span>
          <small>保存到</small>
          <strong title={directory}>{directory || "系统默认下载目录"}</strong>
        </span>
        <IconButton label="更改此任务保存目录" onClick={() => void choose()}>
          <MoreHorizontal size={18} />
        </IconButton>
      </div>
      <Button
        className={`primary full-width ${added ? "success-button" : ""}`}
        onClick={() => void enqueue()}
        disabled={busy || blocked || added}
      >
        {busy ? (
          <LoaderCircle size={17} className="spin" />
        ) : added ? (
          <CheckCheck size={18} />
        ) : (
          <Plus size={18} />
        )}
        {added ? "已加入队列" : busy ? "正在加入" : "加入下载队列"}
      </Button>
      <p className="legal-note">
        <ShieldCheck size={12} />
        请仅下载您拥有权利或获得授权的内容
      </p>
    </div>
  );
}

function Drawer({
  title,
  children,
  onClose,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
}) {
  const element = useRef<HTMLElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    element.current?.focus();
    return () => previous?.focus();
  }, []);
  return (
    <>
      <div className="drawer-scrim" onClick={onClose} />
      <aside
        ref={element}
        className="drawer"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
          if (e.key === "Tab") {
            const items = element.current?.querySelectorAll<HTMLElement>(
              'button:not(:disabled), input, select, a[href], [tabindex="0"]',
            );
            if (!items?.length) return;
            const first = items[0],
              last = items[items.length - 1];
            if (
              e.shiftKey &&
              (document.activeElement === first ||
                document.activeElement === element.current)
            ) {
              e.preventDefault();
              last.focus();
            } else if (!e.shiftKey && document.activeElement === last) {
              e.preventDefault();
              first.focus();
            }
          }
        }}
      >
        <div className="drawer-bar">
          <span>{title}</span>
          <IconButton label="关闭详情" onClick={onClose}>
            <X size={18} />
          </IconButton>
        </div>
        <div className="drawer-body">{children}</div>
      </aside>
    </>
  );
}
function TaskDetails({ task }: { task: DownloadTask }) {
  const { busy, run } = useAction();
  const [copied, setCopied] = useState(false);
  const copy = async (value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch (error) {
      useApp.getState().notify(error);
    }
  };
  return (
    <div className="task-details">
      <MediaImage media={task.media} className="detail-image" />
      <span className="eyebrow">{task.media.source}</span>
      <h2>{task.media.title}</h2>
      <Status task={task} />
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

function Home({
  onMedia,
  onTask,
  go,
}: {
  onMedia: (media: MediaInfo) => void;
  onTask: (task: DownloadTask) => void;
  go: (page: Page) => void;
}) {
  const { tasks, engines } = useApp((s) => s.snapshot);
  const active = tasks.filter((t) => isActive(t.status));
  const recent = tasks.filter((t) => t.status === "completed").slice(0, 3);
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
            {active.slice(0, 3).map((t) => (
              <TaskRow key={t.id} task={t} onSelect={onTask} />
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
            {recent.map((t) => (
              <TaskRow key={t.id} task={t} onSelect={onTask} history />
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

function Queue({ onTask }: { onTask: (task: DownloadTask) => void }) {
  const tasks = useApp((s) => s.snapshot.tasks);
  const [filter, setFilter] = useState("active");
  const active = tasks.filter((t) => isActive(t.status));
  const visible =
    filter === "active"
      ? active
      : filter === "queued"
        ? tasks.filter((t) => t.status === "queued")
        : tasks;
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
            { value: "active", label: "进行中", count: active.length },
            {
              value: "queued",
              label: "等待中",
              count: tasks.filter((t) => t.status === "queued").length,
            },
            { value: "all", label: "全部", count: tasks.length },
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
          {visible.map((t) => (
            <TaskRow key={t.id} task={t} onSelect={onTask} />
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
function HistoryPage({ onTask }: { onTask: (task: DownloadTask) => void }) {
  const tasks = useApp((s) => s.snapshot.tasks)
    .filter((t) => t.status === "completed")
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
  const [query, setQuery] = useState("");
  const [remove, setRemove] = useState<DownloadTask | null>(null);
  const { busy, run } = useAction();
  const visible = tasks.filter((t) =>
    `${t.media.title} ${t.media.source}`
      .toLocaleLowerCase()
      .includes(query.toLocaleLowerCase()),
  );
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
          {visible.map((t) => (
            <div className="history-record" key={t.id}>
              <TaskRow task={t} onSelect={onTask} history />
              <div className="history-extra">
                <span title={t.finalPath || t.outputDir}>
                  <Folder size={12} />
                  {t.finalPath || t.outputDir}
                </span>
                <button onClick={() => setRemove(t)}>
                  <Trash2 size={13} />
                  删除记录
                </button>
              </div>
            </div>
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

function SettingRow({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <div className="setting-row">
      <div>
        <strong>{title}</strong>
        {description && <p>{description}</p>}
      </div>
      <div className="setting-control">{children}</div>
    </div>
  );
}
function EngineRow({ title, info }: { title: string; info: EngineInfo }) {
  return (
    <div className="engine-row">
      <span className={`engine-symbol ${info.available ? "available" : ""}`}>
        {info.available ? <Check size={14} /> : <Minus size={14} />}
      </span>
      <div>
        <strong>
          {title}
          <span>
            {info.available
              ? info.version?.split("\n")[0] || "已检测到"
              : "未就绪"}
          </span>
        </strong>
        {info.path && <small title={info.path}>{info.path}</small>}
        <p>
          {info.message ||
            (info.available ? `来源：${info.source}` : "未检测到可执行程序")}
        </p>
      </div>
    </div>
  );
}
function SettingsPage() {
  const snapshot = useApp((s) => s.snapshot);
  const [draft, setDraft] = useState<Settings>(snapshot.settings);
  const [saved, setSaved] = useState(false);
  const [diagnosing, setDiagnosing] = useState(false);
  const { busy, run } = useAction();
  const set = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    setDraft((v) => ({ ...v, [key]: value }));
    setSaved(false);
  };
  const settingsKey = JSON.stringify(snapshot.settings);
  useEffect(() => {
    setDraft(JSON.parse(settingsKey) as Settings);
  }, [settingsKey]);
  const dirty = JSON.stringify(draft) !== JSON.stringify(snapshot.settings);
  const select = async (
    key: "downloadDir" | "enginePath" | "ffmpegPath",
    directory: boolean,
  ) => {
    try {
      const path = await api.selectPath(
        directory,
        key === "downloadDir"
          ? "选择默认保存目录"
          : key === "enginePath"
            ? "选择 yt-dlp.exe"
            : "选择 FFmpeg 所在目录",
      );
      if (path) set(key, path);
    } catch (error) {
      useApp.getState().notify(error);
    }
  };
  const save = () =>
    run(async () => {
      await api.settings(draft);
      setSaved(true);
    });
  const diagnose = async () => {
    setDiagnosing(true);
    try {
      await api.detect();
      await useApp.getState().refresh();
    } catch (error) {
      useApp.getState().notify(error);
    } finally {
      setDiagnosing(false);
    }
  };
  return (
    <>
      <div className="page-heading settings-heading">
        <div>
          <span className="eyebrow">MAKE IT YOURS</span>
          <h1>设置</h1>
          <p>让 FluxDL 以您习惯的方式工作。</p>
        </div>
        <Button
          className={dirty ? "primary" : ""}
          disabled={!dirty || busy}
          onClick={() => void save()}
        >
          {busy ? (
            <LoaderCircle size={15} className="spin" />
          ) : saved ? (
            <Check size={15} />
          ) : (
            <CheckCheck size={15} />
          )}
          {busy
            ? "正在保存"
            : saved
              ? "已保存"
              : dirty
                ? "保存更改"
                : "设置已同步"}
        </Button>
      </div>
      <div className="settings-stack">
        <section>
          <h2 className="settings-section-label">
            <Folder size={15} />
            常规
          </h2>
          <div className="settings-card">
            <SettingRow
              title="默认保存目录"
              description="文件下载完成后保存在这里"
            >
              <div className="path-control">
                <input
                  aria-label="默认保存目录"
                  value={draft.downloadDir}
                  onChange={(e) => set("downloadDir", e.target.value)}
                  placeholder="系统下载目录"
                  title={draft.downloadDir}
                />
                <IconButton
                  label="选择默认保存目录"
                  onClick={() => void select("downloadDir", true)}
                >
                  <FolderOpen size={17} />
                </IconButton>
              </div>
            </SettingRow>
            <SettingRow title="下载完成后">
              <select
                aria-label="下载完成后"
                value={draft.completionAction}
                onChange={(e) => set("completionAction", e.target.value)}
              >
                <option value="none">保留在任务列表</option>
                <option value="folder">打开所在目录</option>
              </select>
            </SettingRow>
            <SettingRow
              title="关闭主窗口"
              description="活动下载期间始终询问您如何处理"
            >
              <select
                aria-label="关闭主窗口"
                value={draft.closeBehavior}
                onChange={(e) => set("closeBehavior", e.target.value)}
              >
                <option value="tray">最小化到系统托盘</option>
                <option value="exit">退出 FluxDL</option>
              </select>
            </SettingRow>
          </div>
        </section>
        <section>
          <h2 className="settings-section-label">
            <Download size={15} />
            下载偏好
          </h2>
          <div className="settings-card">
            <SettingRow title="默认下载方式">
              <select
                aria-label="默认下载方式"
                value={draft.defaultMode}
                onChange={(e) => set("defaultMode", e.target.value)}
              >
                <option value="video">视频</option>
                <option value="audio">仅音频 · MP3</option>
              </select>
            </SettingRow>
            <SettingRow title="视频画质预设">
              <select
                aria-label="视频画质预设"
                value={draft.quality}
                onChange={(e) => set("quality", e.target.value)}
              >
                {qualityOptions.map((q) => (
                  <option value={q.value} key={q.value}>
                    {q.label}
                  </option>
                ))}
              </select>
            </SettingRow>
            <SettingRow
              title="同时下载任务数"
              description="串行处理，保持队列稳定"
            >
              <span className="fixed-value">1 个任务</span>
            </SettingRow>
          </div>
        </section>
        <section>
          <h2 className="settings-section-label">
            <Sun size={15} />
            外观
          </h2>
          <div className="settings-card">
            <SettingRow title="主题">
              <div className="theme-picker">
                {[
                  { value: "dark", label: "深色", icon: Moon },
                  { value: "light", label: "浅色", icon: Sun },
                  { value: "system", label: "系统", icon: Monitor },
                ].map((t) => (
                  <button
                    title={t.label}
                    aria-label={`${t.label}主题`}
                    aria-pressed={draft.theme === t.value}
                    className={draft.theme === t.value ? "selected" : ""}
                    onClick={() => set("theme", t.value)}
                    key={t.value}
                  >
                    <t.icon size={15} />
                    {t.label}
                  </button>
                ))}
              </div>
            </SettingRow>
            <SettingRow title="任务列表密度">
              <select
                aria-label="任务列表密度"
                value={draft.density}
                onChange={(e) => set("density", e.target.value)}
              >
                <option value="comfortable">舒适</option>
                <option value="compact">紧凑</option>
              </select>
            </SettingRow>
            <SettingRow
              title="减少动画"
              description="始终尊重系统的减少动态效果偏好"
            >
              <button
                className={`switch ${draft.reducedMotion ? "on" : ""}`}
                role="switch"
                aria-checked={draft.reducedMotion}
                aria-label="减少动画"
                onClick={() => set("reducedMotion", !draft.reducedMotion)}
              >
                <span />
              </button>
            </SettingRow>
          </div>
        </section>
        <section id="engine-settings">
          <div className="settings-section-heading">
            <h2 className="settings-section-label">
              <Terminal size={15} />
              本地引擎
            </h2>
            <button
              className="text-button"
              onClick={() => void diagnose()}
              disabled={diagnosing || dirty}
            >
              {diagnosing ? (
                <LoaderCircle size={14} className="spin" />
              ) : (
                <RefreshCw size={14} />
              )}
              {diagnosing ? "检测中" : "重新检测"}
            </button>
          </div>
          <div className="settings-card">
            <SettingRow
              title="yt-dlp 路径"
              description="用户指定路径 → 内置引擎 → PATH"
            >
              <div className="path-control">
                <input
                  aria-label="yt-dlp 路径"
                  value={draft.enginePath}
                  onChange={(e) => set("enginePath", e.target.value)}
                  placeholder="自动从 PATH 检测"
                />
                <IconButton
                  label="选择 yt-dlp.exe"
                  onClick={() => void select("enginePath", false)}
                >
                  <FolderOpen size={17} />
                </IconButton>
              </div>
            </SettingRow>
            <SettingRow
              title="FFmpeg 路径"
              description="选择包含 ffmpeg 与 ffprobe 的目录"
            >
              <div className="path-control">
                <input
                  aria-label="FFmpeg 路径"
                  value={draft.ffmpegPath}
                  onChange={(e) => set("ffmpegPath", e.target.value)}
                  placeholder="自动从 PATH 检测"
                />
                <IconButton
                  label="选择 FFmpeg 目录"
                  onClick={() => void select("ffmpegPath", true)}
                >
                  <FolderOpen size={17} />
                </IconButton>
              </div>
            </SettingRow>
            <div className="engine-diagnostics">
              <EngineRow title="yt-dlp" info={snapshot.engines.ytdlp} />
              <EngineRow title="FFmpeg" info={snapshot.engines.ffmpeg} />
              <EngineRow title="FFprobe" info={snapshot.engines.ffprobe} />
              <EngineRow
                title="JavaScript Runtime"
                info={snapshot.engines.runtime}
              />
            </div>
          </div>
          <div className="notice engine-explanation">
            <CircleAlert size={16} />
            <span>
              版本检测不代表所有站点均可用。YouTube 可能需要 EJS 与受支持的
              JavaScript 运行时；本应用不会自动安装依赖。
              <a
                href="https://github.com/yt-dlp/yt-dlp/wiki/EJS"
                target="_blank"
                rel="noreferrer"
                onClick={(event) => {
                  if (isDesktop) {
                    event.preventDefault();
                    void api.help("ejs").catch(useApp.getState().notify);
                  }
                }}
              >
                查看官方指引
                <SquareArrowOutUpRight size={11} />
              </a>
            </span>
          </div>
          {snapshot.engines.warnings.map((w, i) => (
            <div className="notice warning" key={i}>
              <CircleAlert size={16} />
              <span>{w}</span>
            </div>
          ))}
        </section>
        <section>
          <h2 className="settings-section-label">
            <ShieldCheck size={15} />
            关于
          </h2>
          <div className="settings-card about-card">
            <Brand small />
            <p>轻巧、安静，本地优先的媒体下载工具。</p>
            <span>FluxDL 0.1.0 · MIT License</span>
            <small>不捆绑 yt-dlp 或 FFmpeg。第三方组件按各自许可证使用。</small>
            <Button
              onClick={() => {
                void navigator.clipboard
                  .writeText(
                    JSON.stringify(
                      {
                        version: "0.1.0",
                        engines: snapshot.engines,
                        platform: navigator.userAgent,
                      },
                      null,
                      2,
                    ),
                  )
                  .then(() => setSaved(true))
                  .catch(useApp.getState().notify);
              }}
            >
              <Copy size={14} />
              复制诊断信息
            </Button>
          </div>
        </section>
      </div>
      {dirty && (
        <div className="unsaved-bar">
          <span>
            <span className="tiny-dot" />
            有尚未保存的更改
          </span>
          <Button
            className="primary"
            disabled={busy}
            onClick={() => void save()}
          >
            保存更改
          </Button>
        </div>
      )}
    </>
  );
}

function Modal({
  title,
  children,
  onClose,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const prev = document.activeElement as HTMLElement | null;
    ref.current?.focus();
    return () => prev?.focus();
  }, []);
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div
        ref={ref}
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
          if (e.key === "Tab") {
            const buttons = ref.current?.querySelectorAll<HTMLButtonElement>(
              "button:not(:disabled)",
            );
            if (!buttons?.length) return;
            const first = buttons[0],
              last = buttons[buttons.length - 1];
            if (
              e.shiftKey &&
              (document.activeElement === first ||
                document.activeElement === ref.current)
            ) {
              e.preventDefault();
              last.focus();
            } else if (!e.shiftKey && document.activeElement === last) {
              e.preventDefault();
              first.focus();
            }
          }
        }}
      >
        <div className="modal-title">
          <h2>{title}</h2>
          <IconButton label="关闭对话框" onClick={onClose}>
            <X size={17} />
          </IconButton>
        </div>
        {children}
      </div>
    </div>
  );
}

function QuickCapture() {
  const [media, setMedia] = useState<MediaInfo | null>(null);
  const [added, setAdded] = useState(false);
  const notify = useApp((s) => s.notify);
  const shell = useRef<HTMLDivElement>(null);
  const result = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!isDesktop || !shell.current) return;
    let timer: number;
    let previous = 0;
    const observer = new ResizeObserver(() => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        const height = Math.min(
          720,
          Math.max(260, Math.ceil(shell.current?.scrollHeight ?? 260) + 2),
        );
        if (height !== previous) {
          previous = height;
          void api.resizeQuick(height).catch(notify);
        }
      }, 60);
    });
    observer.observe(shell.current);
    return () => {
      observer.disconnect();
      window.clearTimeout(timer);
    };
  }, [notify]);
  useEffect(() => {
    const handle = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (media) setMedia(null);
        else void api.hideQuick().catch(notify);
      }
    };
    window.addEventListener("keydown", handle);
    return () => window.removeEventListener("keydown", handle);
  }, [media, notify]);
  return (
    <div ref={shell} className={`quick-shell ${media ? "expanded" : ""}`}>
      <div className="quick-title" data-tauri-drag-region>
        <Brand small />
        <span className="quick-caption">Quick Capture</span>
        <div className="quick-title-actions">
          <IconButton
            label="收起快速捕获"
            onClick={() => void api.hideQuick().catch(notify)}
          >
            <X size={18} />
          </IconButton>
        </div>
      </div>
      <div className="quick-content">
        <CaptureInput
          quick
          onMedia={(m) => {
            setMedia(m);
            setAdded(false);
          }}
        />
        {media && (
          <div ref={result} className="quick-result">
            <button
              className="text-button"
              onClick={() => {
                setMedia(null);
                setAdded(false);
              }}
            >
              <RefreshCw size={13} />
              解析其他链接
            </button>
            <MediaOptions
              key={media.url}
              media={media}
              onAdded={() => {
                setAdded(true);
                result.current?.scrollTo({ top: 0, behavior: 'instant' });
              }}
            />
          </div>
        )}
      </div>
      <div className="quick-footer">
        <span>
          <kbd>Esc</kbd> 收起浮窗 · 下载将在后台继续
        </span>
        <button
          className="text-button"
          onClick={() => void api.workspace().catch(notify)}
        >
          {added ? "查看任务" : "打开工作台"}
          <ArrowRight size={14} />
        </button>
      </div>
    </div>
  );
}

export default function App() {
  const [page, setPage] = useState<Page>("home");
  const [media, setMedia] = useState<MediaInfo | null>(null);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [closing, setClosing] = useState(false);
  const [exiting, setExiting] = useState(false);
  const { snapshot, loading, error, refresh, setSnapshot, notify, clearError } =
    useApp();
  const quick =
    new URLSearchParams(window.location.search).get("window") === "quick";
  const activeCount = snapshot.tasks.filter((t) => isActive(t.status)).length;
  const selectedTask = snapshot.tasks.find((t) => t.id === taskId);
  const navigate = useCallback((next: Page) => {
    setPage(next);
    setMedia(null);
    setTaskId(null);
  }, []);
  useEffect(() => {
    void refresh();
    const dispose: (() => void)[] = [];
    let cancelled = false;
    if (isDesktop) {
      void api
        .onSnapshot(setSnapshot)
        .then((unlisten) => (cancelled ? unlisten() : dispose.push(unlisten)));
      void api
        .onClose(() => setClosing(true))
        .then((unlisten) => (cancelled ? unlisten() : dispose.push(unlisten)));
      void api
        .onShutdownFailed((message) => {
          setExiting(false);
          notify(message);
        })
        .then((unlisten) => (cancelled ? unlisten() : dispose.push(unlisten)));
    }
    const focus = () => void refresh();
    window.addEventListener("focus", focus);
    return () => {
      cancelled = true;
      dispose.forEach((fn) => fn());
      window.removeEventListener("focus", focus);
    };
  }, [refresh, setSnapshot]);
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    const apply = () => {
      document.documentElement.dataset.theme =
        snapshot.settings.theme === "system"
          ? mq.matches
            ? "light"
            : "dark"
          : snapshot.settings.theme;
    };
    apply();
    mq.addEventListener("change", apply);
    document.documentElement.dataset.density = snapshot.settings.density;
    document.documentElement.dataset.reducedMotion = String(
      snapshot.settings.reducedMotion,
    );
    return () => mq.removeEventListener("change", apply);
  }, [snapshot.settings]);
  useEffect(() => {
    if (quick) return;
    const handle = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setMedia(null);
        setTaskId(null);
      }
    };
    window.addEventListener("keydown", handle);
    return () => window.removeEventListener("keydown", handle);
  }, [quick]);
  const windowAction = (action: "close" | "minimize" | "maximize") => {
    void api.window(action).catch(notify);
  };
  const exit = async () => {
    setExiting(true);
    try {
      await api.exit();
    } catch (err) {
      notify(err);
      setExiting(false);
    }
  };
  return (
    <>
      <div className={`application ${quick ? "quick-application" : ""}`}>
        {quick ? (
          <QuickCapture />
        ) : (
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
                      className={`engine-dot ${snapshot.engines.ytdlp.available ? "ready" : ""}`}
                    />
                    <span>
                      {snapshot.engines.ytdlp.available
                        ? "引擎已就绪"
                        : "引擎待配置"}
                      <small>
                        {snapshot.engines.ytdlp.available
                          ? `yt-dlp ${snapshot.engines.ytdlp.version || ""}`
                          : "选择本地 yt-dlp"}
                      </small>
                    </span>
                    <ChevronRight size={13} />
                  </button>
                  <div className="sidebar-version">
                    <span>本地 · 私有 · 自由</span>
                    <span>v0.1</span>
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
                    <Home
                      onMedia={setMedia}
                      onTask={(t) => setTaskId(t.id)}
                      go={navigate}
                    />
                  ) : page === "queue" ? (
                    <Queue onTask={(t) => setTaskId(t.id)} />
                  ) : page === "history" ? (
                    <HistoryPage onTask={(t) => setTaskId(t.id)} />
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
                <MediaOptions
                  key={media.url}
                  media={media}
                  onAdded={() => {}}
                />
              </Drawer>
            )}
            {selectedTask && !media && (
              <Drawer title="任务详情" onClose={() => setTaskId(null)}>
                <TaskDetails task={selectedTask} />
              </Drawer>
            )}
          </>
        )}
      </div>
      {error && (
        <div className="error-toast" role="alert">
          <CircleAlert size={19} />
          <span>{error}</span>
          <IconButton label="关闭提示" onClick={clearError}>
            <X size={16} />
          </IconButton>
        </div>
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
