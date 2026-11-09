import { useState } from "react";
import {
  AudioLines,
  CheckCheck,
  ChevronDown,
  CircleAlert,
  Folder,
  LoaderCircle,
  MoreHorizontal,
  Plus,
  ShieldCheck,
  Video,
} from "lucide-react";
import { api } from "../../lib/api";
import { type MediaInfo } from "../../lib/types";
import { bytes, duration } from "../../lib/utils";
import { useApp } from "../../stores/app";
import { IconButton, Button, MediaImage } from "../../components/ui";
import { qualityOptions } from "../../lib/presets";
import { TaskDiagnosticStatus } from "../diagnostics/TaskDiagnosticStatus";

export function MediaOptions({
  media,
  onAdded,
}: {
  media: MediaInfo;
  onAdded: (taskId?: string) => void;
}) {
  const settings = useApp((s) => s.settings);
  const engines = useApp((s) => s.engines);
  const notify = useApp((s) => s.notify);
  const [mode, setMode] = useState(settings.defaultMode);
  const [quality, setQuality] = useState(settings.quality);
  const [format, setFormat] = useState("");
  const [directory, setDirectory] = useState(settings.downloadDir);
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false);
  const [added, setAdded] = useState(false);
  const [traceId, setTraceId] = useState("");
  const ffmpeg = engines.ffmpeg.available && engines.ffprobe.available;
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
      const task = await api.enqueue(
        {
          media,
          mode,
          quality,
          formatId: format || null,
          outputDir: directory || null,
        },
        setTraceId,
      );
      await useApp.getState().refresh();
      setAdded(true);
      onAdded(task.id);
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
      {busy && traceId && (
        <TaskDiagnosticStatus traceId={traceId} active={busy} />
      )}
    </div>
  );
}
