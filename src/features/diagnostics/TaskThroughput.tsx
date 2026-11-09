import {
  statusLabels,
  type DiagnosticEvent,
  type ThroughputSnapshot,
} from "./types";

const measured = (value: number | null | undefined): value is number =>
  value != null && Number.isFinite(value) && value >= 0;

export function throughputBytes(value: number | null | undefined): string {
  if (!measured(value)) return "未测量";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(unit ? 2 : 0)} ${units[unit]}`;
}

export function throughputSpeed(value: number | null | undefined): string {
  return measured(value) ? `${throughputBytes(value)}/s` : "未测量";
}

const seconds = (value: number | null | undefined) =>
  measured(value) ? `${(value / 1000).toFixed(1)} 秒` : "未测量";

const protocolLabel = (protocol: ThroughputSnapshot["protocol"]) => {
  switch (protocol) {
    case "http":
      return "HTTP";
    case "https":
      return "HTTPS";
    case "http_dash_segments":
      return "DASH 分片";
    case "m3u8":
    case "m3u8_native":
      return "HLS 分片";
    default:
      return "未测量";
  }
};

// Queries return newest first. A new attempt/Trace must never inherit old speed.
export function latestThroughput(
  events: DiagnosticEvent[],
): DiagnosticEvent | undefined {
  const latest = events.find(
    (e) => e.source === "rust" && e.phase !== "session",
  );
  if (!latest) return undefined;
  return events.find(
    (e) =>
      e.source === "rust" &&
      e.phase === "download_throughput" &&
      e.event === "throughput_sample" &&
      e.fields.throughput != null &&
      e.trace_id === latest.trace_id &&
      e.fields.attempt_id === latest.fields.attempt_id,
  );
}

export function diagnosticStatusLabel(event: DiagnosticEvent): string {
  if (event.phase !== "download_throughput" || !event.fields.throughput)
    return statusLabels[event.status];
  return event.fields.throughput.final_sample
    ? `最终采样 · ${statusLabels[event.status]}`
    : "采样有效";
}

export function TaskThroughput({
  event,
  sessionId,
}: {
  event?: DiagnosticEvent;
  sessionId?: string;
}) {
  const sample = event?.fields.throughput;
  const timestamp = event ? new Date(event.timestamp_utc) : null;
  return (
    <section className="diagnostic-throughput" aria-label="下载吞吐诊断">
      <h3>下载吞吐诊断</h3>
      {!sample || !event ? (
        <p className="muted">
          尚无持续下载吞吐记录。只有真实字节进度才能生成样本。
        </p>
      ) : (
        <>
          <p className="muted">
            {sessionId && event.session_id !== sessionId
              ? "上一会话记录 · "
              : ""}
            {diagnosticStatusLabel(event)} ·{" "}
            {timestamp && Number.isFinite(timestamp.getTime())
              ? timestamp.toLocaleTimeString()
              : "采样时间未知"}
          </p>
          <dl className="diagnostic-throughput-metrics">
            <div>
              <dt>媒体协议</dt>
              <dd>{protocolLabel(sample.protocol)}</dd>
            </div>
            <div>
              <dt>下载器配置</dt>
              <dd>
                {sample.downloader === "ytdlp_default"
                  ? "yt-dlp 默认选择"
                  : "未测量"}
              </dd>
            </div>
            <div>
              <dt>网络配置</dt>
              <dd>
                {sample.network_mode === "inherited"
                  ? "继承网络环境（实际路由未确认）"
                  : "未测量"}
              </dd>
            </div>
            <div>
              <dt>5 秒窗口</dt>
              <dd>{throughputSpeed(sample.window_5s_bps)}</dd>
            </div>
            <div>
              <dt>10 秒窗口</dt>
              <dd>{throughputSpeed(sample.window_10s_bps)}</dd>
            </div>
            <div>
              <dt>观测均速</dt>
              <dd>{throughputSpeed(sample.mean_bps)}</dd>
            </div>
            <div>
              <dt>累计观测字节</dt>
              <dd>{throughputBytes(sample.observed_bytes)}</dd>
            </div>
            <div>
              <dt>下载观测时长</dt>
              <dd>{seconds(sample.duration_ms)}</dd>
            </div>
            <div>
              <dt>停滞记录</dt>
              <dd>
                {measured(sample.stall_count)
                  ? `${sample.stall_count} 次`
                  : "未测量"}{" "}
                / {seconds(sample.stall_duration_ms)}
              </dd>
            </div>
            <div>
              <dt>末次增长间隔</dt>
              <dd>{seconds(sample.progress_gap_ms)}</dd>
            </div>
          </dl>
          <p className="muted">
            仅显示最近低频记录，不随页面计时增长。观测字节不含首次已有或续传字节，
            不等于任务总大小；窗口不足时为未测量。解析、排队和后处理不计入吞吐，
            进度间隔不代表网络断开。协议来自实际进度；下载器与网络项只记录配置事实，
            不据此认定 TUN 或实际出站节点。
          </p>
        </>
      )}
    </section>
  );
}
