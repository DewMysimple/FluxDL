import { useEffect, useState, type ReactNode } from "react";
import { ArrowDown, FileVideo } from "lucide-react";
import { type DownloadTask, type MediaInfo } from "../lib/types";
import { statusLabels } from "../lib/utils";

export function Brand({ small = false }: { small?: boolean }) {
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

export function IconButton({
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

export function Button({
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

export function Empty({
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

export function MediaImage({
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

export function Status({ task }: { task: DownloadTask }) {
  return (
    <span className={`status status-${task.status}`}>
      <span />
      {statusLabels[task.status]}
    </span>
  );
}

export function Progress({ task }: { task: DownloadTask }) {
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
