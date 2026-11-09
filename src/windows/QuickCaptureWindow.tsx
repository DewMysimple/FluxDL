import { useAppSync } from "../hooks/useAppSync";
import { useEffect, useRef, useState } from "react";
import { ArrowRight, RefreshCw, X } from "lucide-react";
import { api, isDesktop } from "../lib/api";
import { type MediaInfo } from "../lib/types";
import { useApp } from "../stores/app";
import { Brand, IconButton } from "../components/ui";
import { CaptureInput } from "../features/capture/CaptureInput";
import { MediaOptions } from "../features/capture/MediaOptions";

export function QuickCaptureWindow() {
  useAppSync();
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
                result.current?.scrollTo({ top: 0, behavior: "instant" });
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
