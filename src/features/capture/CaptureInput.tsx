import { useRef, useState, type FormEvent } from "react";
import {
  CircleAlert,
  Clipboard,
  Link2,
  LoaderCircle,
  Sparkles,
} from "lucide-react";
import { api } from "../../lib/api";
import { type MediaInfo } from "../../lib/types";
import { errorText, validateUrl } from "../../lib/utils";
import { useApp } from "../../stores/app";
import { IconButton, Button } from "../../components/ui";
import { qualityName } from "../../lib/presets";

export function CaptureInput({
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
  const settings = useApp((s) => s.settings);
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
