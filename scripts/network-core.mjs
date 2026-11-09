// Small bounded helpers for the network investigation runner. No raw URLs are logged.
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

export const MAX_ROUND_SECONDS = 45;
export const MAX_ROUND_BYTES = 40 * 1024 * 1024;
export const MAX_TOTAL_BYTES = 300 * 1024 * 1024;

export function totalBudgetBytes(value = MAX_TOTAL_BYTES) {
  if (!Number.isSafeInteger(value) || value < 1 || value > MAX_TOTAL_BYTES)
    throw new Error("invalid_total_budget_bytes");
  return value;
}
export function firstProgressDuration(record) {
  return record?.type === "first_progress" &&
    Number.isFinite(record.duration_s) &&
    record.duration_s >= 0 &&
    record.duration_s <= 604800
    ? record.duration_s
    : null;
}
const stagePhases = new Set([
  "session",
  "parse_ui_request",
  "parse_ipc_dispatch",
  "parse_validate",
  "parse_engine_spawn",
  "parse_metadata_extract",
  "parse_response",
  "parse_total",
  "enqueue_ui_request",
  "enqueue_validate",
  "enqueue_metadata_recheck",
  "enqueue_persist",
  "enqueue_db_queue",
  "enqueue_db_write",
  "enqueue_to_progress",
  "queue_wait",
  "download_engine_spawn",
  "download_first_progress",
  "download_active",
  "download_throughput",
  "download_postprocess",
  "download_terminal",
  "persistence_issue",
  "download_process_issue",
  "download_total",
]);
const stageStatuses = new Set([
  "started",
  "ok",
  "failed",
  "cancelled",
  "unobserved",
  "notapplicable",
  "interrupted",
]);
export function safeStage(value) {
  if (
    !value ||
    !stagePhases.has(value.phase) ||
    !stageStatuses.has(value.status)
  )
    return null;
  const duration = value.duration_ms ?? null;
  if (
    duration !== null &&
    (!Number.isFinite(duration) || duration < 0 || duration > 604800000)
  )
    return null;
  return { phase: value.phase, status: value.status, duration_ms: duration };
}
export function mergeStages(...groups) {
  const out = [],
    seen = new Set();
  for (const group of groups) {
    if (!Array.isArray(group)) continue;
    for (const value of group) {
      const stage = safeStage(value);
      if (!stage) continue;
      const key = JSON.stringify(stage);
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(stage);
      if (out.length >= 128) return out;
    }
  }
  return out;
}
export function conditionList(value) {
  if (value === undefined) return ["A", "B", "C"];
  if (
    !Array.isArray(value) ||
    value.length === 0 ||
    value.length > 64 ||
    !value.every((x) => ["A", "B", "C"].includes(x))
  )
    throw new Error("invalid_conditions");
  return [...new Set(value)];
}

export function selectFormat(formats, requested = null) {
  if (requested) return formats.find((f) => f.id === requested) ?? null;
  const video = formats.filter((f) => f.has_video);
  const withinPreset = video.filter(
    (f) => Number.isFinite(f.height) && f.height > 0 && f.height <= 1080,
  );
  const order = (a, b) =>
    b.height - a.height || Number(b.has_audio) - Number(a.has_audio);
  const https = withinPreset.filter((f) => f.protocol === "https").sort(order);
  return (
    https[0] ??
    withinPreset.sort(order)[0] ??
    video.find((f) => f.protocol === "https" && f.has_audio) ??
    video.find((f) => f.protocol === "https") ??
    video[0] ??
    null
  );
}

export function localProxy(value) {
  try {
    const parsed = new URL(value);
    const explicitPort =
      /^(?:http|socks5):\/\/(?:127\.0\.0\.1|localhost|\[::1\]):([0-9]{1,5})(?:\/)?$/i.exec(
        value,
      )?.[1];
    if (
      !["http:", "socks5:"].includes(parsed.protocol) ||
      !["127.0.0.1", "localhost", "[::1]"].includes(parsed.hostname) ||
      parsed.username ||
      parsed.password ||
      parsed.search ||
      parsed.hash ||
      !["", "/"].includes(parsed.pathname) ||
      !explicitPort ||
      Number(explicitPort) < 1 ||
      Number(explicitPort) > 65535
    )
      return null;
    return {
      protocol: parsed.protocol.slice(0, -1),
      port: Number(explicitPort),
      host: parsed.hostname,
    };
  } catch {
    return null;
  }
}

export function proxyCandidates(facts) {
  const owners = new Set(
    (facts.processes ?? [])
      .filter((p) => /mihomo|clash/i.test(p.name) && !/verge$/i.test(p.name))
      .map((p) => p.pid),
  );
  return [
    ...new Set(
      (facts.listeners ?? [])
        .filter(
          (x) =>
            owners.has(x.pid) &&
            ["127.0.0.1", "::1", "0.0.0.0", "::"].includes(x.address),
        )
        .map((x) => x.port),
    ),
  ]
    .filter((n) => Number.isInteger(n) && n > 0 && n <= 65535)
    .slice(0, 6);
}

export function probeLocalProxy(port, protocol, timeoutMs = 800) {
  return new Promise((resolve) => {
    let done = false;
    let socket;
    let target;
    const nonce = `FLUXDL-LOCAL-${crypto.randomUUID()}`;
    let collected = Buffer.alloc(0);
    const finish = (ok, authenticationRequired = false) => {
      if (done) return;
      done = true;
      socket?.destroy();
      target?.close();
      resolve({
        port,
        protocol,
        confirmed: ok,
        authentication_required: authenticationRequired,
      });
    };
    function connect(targetPort) {
      socket = net.connect({ host: "127.0.0.1", port });
      socket.setTimeout(timeoutMs, () => finish(false));
      socket.on("error", () => finish(false));
      socket.on("close", () => finish(false));
      socket.on("connect", () =>
        socket.write(
          protocol === "socks5"
            ? Buffer.from([5, 1, 0])
            : `CONNECT 127.0.0.1:${targetPort} HTTP/1.1\r\nHost: 127.0.0.1:${targetPort}\r\n\r\n`,
        ),
      );
      socket.on("data", (chunk) => {
        collected = Buffer.concat([collected, chunk]).subarray(0, 2048);
        if (protocol === "socks5") {
          if (collected.length >= 2)
            finish(
              collected[0] === 5 && collected[1] === 0,
              collected[0] === 5 && collected[1] === 2,
            );
        } else if (collected.includes(10)) {
          const text = collected.toString("ascii");
          if (/^HTTP\/1\.[01] 407\b/.test(text)) finish(false, true);
          else if (!/^HTTP\/1\.[01] 200\b/.test(text)) finish(false);
          else if (text.includes(nonce)) finish(true);
        }
      });
    }
    if (protocol === "http") {
      target = net.createServer((connection) => {
        connection.end(nonce);
      });
      target.once("error", () => finish(false));
      target.listen(0, "127.0.0.1", () => connect(target.address().port));
    } else connect(0);
  });
}

export class DownloadBudget {
  constructor({
    seconds = MAX_ROUND_SECONDS,
    bytes = MAX_ROUND_BYTES,
    total = MAX_TOTAL_BYTES,
  } = {}) {
    if (![seconds, bytes, total].every(Number.isFinite))
      throw new Error("invalid_budget");
    this.seconds = Math.min(MAX_ROUND_SECONDS, Math.max(0.05, seconds));
    this.bytes = Math.min(MAX_ROUND_BYTES, Math.max(1, bytes));
    this.remaining = Math.min(MAX_TOTAL_BYTES, Math.max(0, total));
  }
  roundLimit() {
    return Math.min(this.bytes, this.remaining);
  }
  reason(elapsed, bytes) {
    if (![elapsed, bytes].every(Number.isFinite) || elapsed < 0 || bytes < 0)
      throw new Error("invalid_budget_observation");
    if (bytes >= this.roundLimit())
      return this.remaining <= this.bytes
        ? "total_byte_budget"
        : "round_byte_budget";
    if (elapsed >= this.seconds) return "round_time_budget";
    return null;
  }
  account(bytes) {
    if (!Number.isFinite(bytes) || bytes < 0)
      throw new Error("invalid_budget_observation");
    this.remaining = Math.max(0, this.remaining - bytes);
  }
}

// Observed cumulative bytes, not yt-dlp's instantaneous speed text. No URL/title fields.
export class TransferMetrics {
  constructor() {
    this.points = [];
    this.first = null;
    this.end = null;
    this.stopped = false;
    this.maxBps = null;
    this.rateAnchor = null;
  }
  add(t, cumulative, phase = "downloading") {
    if (
      this.stopped ||
      !Number.isFinite(t) ||
      !Number.isFinite(cumulative) ||
      t < 0 ||
      cumulative < 0
    )
      return;
    if (!["downloading", "finished"].includes(phase)) {
      if (this.first !== null) this.stop(t);
      return;
    }
    const last = this.points.at(-1);
    if (last && (t < last.t || cumulative < last.bytes)) return;
    if (this.first === null && cumulative === 0) return;
    if (this.first === null) this.first = t;
    if (this.rateAnchor === null) this.rateAnchor = { t, bytes: cumulative };
    if (t - this.rateAnchor.t >= 0.15) {
      this.maxBps = Math.max(
        this.maxBps ?? 0,
        (cumulative - this.rateAnchor.bytes) / (t - this.rateAnchor.t),
      );
      this.rateAnchor = { t, bytes: cumulative };
    }
    this.points.push({ t, bytes: cumulative });
    // 45 seconds of 10Hz polling is bounded. Drop duplicate same-timestamp samples.
    if (this.points.length > 1000) this.points.splice(1, 1);
    if (phase === "finished") this.stop(t);
  }
  stop(t) {
    if (this.stopped) return;
    this.stopped = true;
    this.end = this.points.at(-1)?.t ?? t;
  }
  valueAt(t) {
    let previous = this.points[0];
    if (!previous) return 0;
    for (const current of this.points) {
      if (current.t >= t)
        return current.t === previous.t
          ? current.bytes
          : previous.bytes +
              ((current.bytes - previous.bytes) * (t - previous.t)) /
                (current.t - previous.t);
      previous = current;
    }
    return previous.bytes;
  }
  summary() {
    const first = this.points[0],
      last = this.points.at(-1);
    const duration = first && last ? last.t - first.t : 0;
    const bytes = first && last ? last.bytes - first.bytes : 0;
    const windows = [];
    if (first && last)
      for (let end = first.t + 5; end <= last.t + 1e-7; end += 5)
        windows.push((this.valueAt(end) - this.valueAt(end - 5)) / 5);
    windows.sort((a, b) => a - b);
    let stallCount = 0,
      stallSeconds = 0,
      unchangedSince = null;
    for (let i = 1; i < this.points.length; i++) {
      const a = this.points[i - 1],
        b = this.points[i];
      if (b.bytes === a.bytes) {
        unchangedSince ??= a.t;
      } else if (unchangedSince !== null) {
        const span = a.t - unchangedSince;
        if (span >= 5) {
          stallCount++;
          stallSeconds += span;
        }
        unchangedSince = null;
      }
    }
    if (unchangedSince !== null && last.t - unchangedSince >= 5) {
      stallCount++;
      stallSeconds += last.t - unchangedSince;
    }
    return {
      duration_s: duration,
      downloaded_bytes: bytes,
      observed_total_bytes: last?.bytes ?? 0,
      mean_Bps: duration >= 1 ? bytes / duration : null,
      median_5s_Bps: windows.length
        ? windows[Math.floor((windows.length - 1) / 2)]
        : null,
      last_10s_Bps:
        duration >= 10 ? (last.bytes - this.valueAt(last.t - 10)) / 10 : null,
      max_Bps: this.maxBps,
      first_progress_s: this.first,
      stall_count: stallCount,
      stall_seconds: stallSeconds,
      five_second_windows: windows.length,
      stall_threshold_s: 5,
      byte_basis:
        "observed cumulative progress delta after first progress; excludes unknown pre-first-progress transfer",
    };
  }
}

export function speedLabel(value) {
  if (!Number.isFinite(value)) return "未测量";
  return value >= 1024 * 1024
    ? `${(value / 1024 / 1024).toFixed(3)} MiB/s`
    : `${(value / 1024).toFixed(2)} KiB/s`;
}

export function safeRoot(workspace, candidate) {
  const base = path.resolve(workspace, "artifacts/network-diagnostics");
  const resolved = path.resolve(candidate);
  if (!resolved.startsWith(base + path.sep))
    throw new Error("unsafe_owned_output");
  let p = resolved;
  while (p !== path.dirname(p)) {
    if (fs.existsSync(p) && fs.lstatSync(p).isSymbolicLink())
      throw new Error("linked_output");
    p = path.dirname(p);
  }
  return resolved;
}

export async function terminateOwned(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  if (process.platform === "win32") {
    await new Promise((resolve) => {
      const killer = spawn(
        path.join(
          process.env.SystemRoot ?? "C:\\Windows",
          "System32/taskkill.exe",
        ),
        ["/PID", String(child.pid), "/T", "/F"],
        { windowsHide: true, stdio: "ignore" },
      );
      killer.once("error", resolve);
      killer.once("exit", resolve);
    });
  } else {
    try {
      process.kill(-child.pid, "SIGKILL");
    } catch {
      child.kill("SIGKILL");
    }
  }
}

export function runSafeChild(
  executable,
  args,
  {
    input = {},
    cwd,
    hardSeconds = 45,
    onRecord = () => {},
    onTick = () => {},
    onSpawn = () => {},
    env = process.env,
  } = {},
) {
  if (!Number.isFinite(hardSeconds) || hardSeconds < 0.05 || hardSeconds > 60)
    throw new Error("invalid_child_deadline");
  return new Promise((resolve, reject) => {
    const start = performance.now();
    const windows = process.platform === "win32";
    const helper = fileURLToPath(
      new URL("./network-owned-child.ps1", import.meta.url),
    );
    const child = spawn(
      windows ? "powershell.exe" : executable,
      windows
        ? [
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            helper,
          ]
        : args,
      {
        cwd,
        env,
        windowsHide: true,
        detached: !windows,
        stdio: ["pipe", "pipe", "pipe"],
      },
    );
    onSpawn(child);
    let pending = "",
      outputBytes = 0,
      stderrBytes = 0,
      killed = false,
      stopReason = null;
    const stop = async (reason) => {
      if (killed) return;
      killed = true;
      stopReason = reason;
      await terminateOwned(child);
    };
    const timer = setInterval(() => {
      const elapsed = (performance.now() - start) / 1000;
      const reason =
        elapsed >= hardSeconds
          ? "hard_time_budget"
          : onTick(elapsed, child.pid);
      if (reason) void stop(reason);
    }, 50);
    child.stdout.setEncoding("utf8").on("data", (data) => {
      outputBytes += Buffer.byteLength(data);
      if (outputBytes > 1024 * 1024) {
        void stop("output_budget");
        return;
      }
      pending += data;
      let n;
      while ((n = pending.indexOf("\n")) >= 0) {
        const line = pending.slice(0, n);
        pending = pending.slice(n + 1);
        try {
          const record = JSON.parse(line);
          const reason = onRecord(record);
          if (reason) void stop(reason);
        } catch {
          /* No raw stderr, malformed child output, paths or command lines enter reports. */
        }
      }
    });
    child.stderr.on("data", (data) => {
      stderrBytes += data.length;
      if (stderrBytes > 1024 * 1024) void stop("stderr_budget");
    });
    child.stdin.on("error", () => {});
    child.stdin.end(
      JSON.stringify(
        windows
          ? {
              executable,
              args,
              input: JSON.stringify(input),
              directory: cwd,
              timeoutMs: Math.ceil(hardSeconds * 1000),
            }
          : input,
      ),
    );
    child.once("error", () => {
      clearInterval(timer);
      reject(new Error("child_spawn_failed"));
    });
    child.once("close", (code, signal) => {
      clearInterval(timer);
      resolve({
        code,
        signal,
        killed,
        stop_reason: stopReason,
        duration_s: (performance.now() - start) / 1000,
        stderr_present: stderrBytes > 0,
      });
    });
  });
}
