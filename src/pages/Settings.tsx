import { useEffect, useState, type ReactNode } from "react";
import { version as appVersion } from "../../package.json";
import {
  Check,
  CheckCheck,
  CircleAlert,
  Copy,
  Download,
  Folder,
  FolderOpen,
  LoaderCircle,
  Minus,
  Monitor,
  Moon,
  RefreshCw,
  ShieldCheck,
  SquareArrowOutUpRight,
  Sun,
  Terminal,
} from "lucide-react";
import { api, isDesktop } from "../lib/api";
import { type EngineInfo, type Settings } from "../lib/types";
import { useApp } from "../stores/app";
import { Brand, IconButton, Button } from "../components/ui";
import { useAction } from "../hooks/useAction";
import { qualityOptions } from "../lib/presets";

export function SettingsPage() {
  const settings = useApp((s) => s.settings);
  const engines = useApp((s) => s.engines);
  const [draft, setDraft] = useState<Settings>(settings);
  const [saved, setSaved] = useState(false);
  const [diagnosing, setDiagnosing] = useState(false);
  const { busy, run } = useAction();
  const set = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    setDraft((v) => ({ ...v, [key]: value }));
    setSaved(false);
  };
  useEffect(() => {
    setDraft(settings);
  }, [settings]);
  const dirty = JSON.stringify(draft) !== JSON.stringify(settings);
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
              <EngineRow title="yt-dlp" info={engines.ytdlp} />
              <EngineRow title="FFmpeg" info={engines.ffmpeg} />
              <EngineRow title="FFprobe" info={engines.ffprobe} />
              <EngineRow title="JavaScript Runtime" info={engines.runtime} />
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
          {engines.warnings.map((w, i) => (
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
            <span>FluxDL {appVersion} · MIT License</span>
            <small>不捆绑 yt-dlp 或 FFmpeg。第三方组件按各自许可证使用。</small>
            <Button
              onClick={() => {
                void navigator.clipboard
                  .writeText(
                    JSON.stringify(
                      {
                        version: appVersion,
                        engines: engines,
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

export function SettingRow({
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

export function EngineRow({
  title,
  info,
}: {
  title: string;
  info: EngineInfo;
}) {
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
