// Automatic, bounded comparison. URL is read from stdin/environment only and never saved.
import fs from "node:fs";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import {
  DownloadBudget,
  TransferMetrics,
  localProxy,
  proxyCandidates,
  probeLocalProxy,
  safeRoot,
  speedLabel,
  runSafeChild,
  selectFormat,
  totalBudgetBytes,
  firstProgressDuration,
  safeStage,
  mergeStages,
  conditionList,
} from "./network-core.mjs";

const workspace = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const owner = "fluxdl-network-diagnostics-v1";
const say = (message) => process.stdout.write(`${message}\n`);
function powerShell(filename, args = [], timeout = 10000) {
  const r = spawnSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-NonInteractive",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      path.join(workspace, "scripts", filename),
      ...args,
    ],
    {
      cwd: workspace,
      windowsHide: true,
      encoding: "utf8",
      timeout,
      maxBuffer: 512 * 1024,
    },
  );
  if (r.status !== 0) return null;
  try {
    return JSON.parse(r.stdout.trim());
  } catch {
    return null;
  }
}
function powerShellAsync(filename, args = [], timeout = 3500) {
  return new Promise((resolve) => {
    const child = spawn(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        path.join(workspace, "scripts", filename),
        ...args,
      ],
      {
        cwd: workspace,
        windowsHide: true,
        stdio: ["ignore", "pipe", "ignore"],
      },
    );
    let output = "";
    const timer = setTimeout(() => child.kill(), timeout);
    child.stdout.setEncoding("utf8").on("data", (chunk) => {
      output += chunk;
      if (output.length > 65536) child.kill();
    });
    child.once("error", () => {
      clearTimeout(timer);
      resolve(null);
    });
    child.once("close", (code) => {
      clearTimeout(timer);
      if (code !== 0) {
        resolve(null);
        return;
      }
      try {
        resolve(JSON.parse(output));
      } catch {
        resolve(null);
      }
    });
  });
}
function readInput() {
  const raw =
    process.env.FLUXDL_NETWORK_INPUT_JSON ??
    (!process.stdin.isTTY ? fs.readFileSync(0, "utf8") : "{}");
  if (raw.length > 32768) throw new Error("input_too_large");
  const input = JSON.parse(raw || "{}");
  if (!input.url && process.env.FLUXDL_TEST_URL)
    input.url = process.env.FLUXDL_TEST_URL;
  if (!input.enginePath && process.env.FLUXDL_ENGINE_PATH)
    input.enginePath = process.env.FLUXDL_ENGINE_PATH;
  if (input.url) {
    const u = new URL(input.url);
    if (
      !["https:", "http:"].includes(u.protocol) ||
      u.username ||
      u.password ||
      /[\r\n\0]/.test(input.url)
    )
      throw new Error("invalid_authorized_url");
  }
  if (
    input.seconds !== undefined &&
    (!Number.isFinite(input.seconds) ||
      input.seconds < 0.1 ||
      input.seconds > 45)
  )
    throw new Error("invalid_round_seconds");
  if (input.repetitions !== undefined && ![1, 2].includes(input.repetitions))
    throw new Error("invalid_repetitions");
  totalBudgetBytes(input.totalBudgetBytes);
  conditionList(input.conditions);
  return input;
}
function selfTest() {
  const files = [
    "scripts/network-diagnostics.test.mjs",
    "scripts/network-transfer.test.mjs",
  ].filter((p) => fs.existsSync(path.join(workspace, p)));
  const r = spawnSync(process.execPath, ["--test", ...files], {
    cwd: workspace,
    windowsHide: true,
    encoding: "utf8",
    timeout: 45000,
    maxBuffer: 1024 * 1024,
  });
  if (r.status !== 0) {
    say("预算、隐私或自有进程清理自检失败；未启动网络下载。");
    return false;
  }
  say("预算、滑动速度、代理探测、隐私与自有进程清理自检通过。");
  return true;
}
function enginePath(input) {
  const candidates = [];
  if (typeof input.enginePath === "string") candidates.push(input.enginePath);
  for (const segment of (process.env.PATH ?? "").split(path.delimiter))
    if (segment)
      candidates.push(path.join(segment.replace(/^"|"$/g, ""), "yt-dlp.exe"));
  // Read only the existing settings record, never tasks/URLs or unrelated user files.
  try {
    const require = createRequire(import.meta.url);
    const { DatabaseSync } = require("node:sqlite");
    for (const base of [process.env.APPDATA, process.env.LOCALAPPDATA].filter(
      Boolean,
    )) {
      const file = path.join(base, "io.fluxdl.desktop", "fluxdl.sqlite3");
      if (!fs.existsSync(file)) continue;
      const db = new DatabaseSync(file, { readOnly: true });
      try {
        const row = db
          .prepare("SELECT document FROM app_settings WHERE id=1")
          .get();
        const s = JSON.parse(row?.document ?? "{}");
        if (typeof s.enginePath === "string" && s.enginePath)
          candidates.push(s.enginePath);
        if (!input.ffmpegPath && typeof s.ffmpegPath === "string")
          input.ffmpegPath = s.ffmpegPath;
      } finally {
        db.close();
      }
    }
  } catch {
    /* Optional database discovery; missing/locked/future schema is never repaired. */
  }
  // Known application/engine processes only; executable paths remain in memory.
  if (process.platform === "win32") {
    const script =
      "$p=@(Get-CimInstance Win32_Process | Where-Object { $_.Name -in @('FluxDL.exe','fluxdl.exe','yt-dlp.exe') } | Select-Object -ExpandProperty ExecutablePath); ConvertTo-Json -InputObject $p -Compress";
    const r = spawnSync(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-Command", script],
      { windowsHide: true, encoding: "utf8", timeout: 8000, maxBuffer: 65536 },
    );
    try {
      for (const p of JSON.parse(r.stdout))
        if (p) {
          if (/yt-dlp\.exe$/i.test(p)) candidates.push(p);
          else {
            candidates.push(
              path.join(path.dirname(p), "yt-dlp.exe"),
              path.join(path.dirname(p), "binaries/yt-dlp.exe"),
            );
          }
        }
    } catch {
      /* Discovery failure is reported without private output. */
    }
  }
  for (const p of candidates) {
    try {
      if (path.isAbsolute(p) && /\.exe$/i.test(p) && fs.statSync(p).isFile())
        return fs.realpathSync(p);
    } catch {
      /* Try the next safe local candidate. */
    }
  }
  return null;
}
function summaryMarkdown(result) {
  const rows = result.samples.map(
    (s) =>
      `| ${s.condition} / ${s.repetition} | ${speedLabel(s.metrics.mean_Bps)} | ${speedLabel(s.metrics.median_5s_Bps)} | ${speedLabel(s.metrics.last_10s_Bps)} | ${s.format_id ?? "未测量"} / ${s.protocol ?? "未知"} | ${s.proxy_mode} | ${s.terminal ?? s.failure?.error_code ?? "未完成"} |`,
  );
  return `# FluxDL 下载网络自动调查\n\n类型：${result.test_kind}。仅本次自动测试，不代表持续监控；没有修改Clash、TUN、系统代理或用户数据。\n\n${result.limitations.join("\n\n")}\n\n| 测试 | 平均速度 | 5秒窗口中位数 | 最后10秒速率 | 格式/协议 | 网络配置 | 终态 |\n| --- | ---: | ---: | ---: | --- | --- | --- |\n${rows.join("\n")}\n\n预算：单轮外层截止45秒、约40MiB；本次媒体总预算${(result.budget_limits.total_bytes / 1024 / 1024).toFixed(3)}MiB（绝对上限300MiB，可扣除前轮实际消耗）。进度/文件大小约束与自有进程树截止同时启用。观测到HTTP 403/429即停止后续测试。下载字节是观测进度增量，不包含未知的首次进度前传输；不把解析、后处理、取消清理算网络速度。\n\nURL、标题、媒体签名、代理凭据、控制器密钥、原始命令/stdout/stderr与本机路径均未写入此报告。\n`;
}
function mediaBytes(directory) {
  let bytes = 0,
    count = 0;
  function visit(folder) {
    for (const entry of fs.readdirSync(folder, { withFileTypes: true })) {
      if (++count > 512) throw new Error("owned_media_count_bound");
      const p = path.join(folder, entry.name),
        stat = fs.lstatSync(p);
      if (stat.isSymbolicLink()) throw new Error("owned_media_link");
      if (stat.isDirectory()) visit(p);
      else if (stat.isFile()) bytes += stat.size;
    }
  }
  visit(directory);
  return bytes;
}
export async function main() {
  if (!selfTest()) {
    process.exitCode = 1;
    return;
  }
  if (
    process.argv.includes("--self-test") ||
    process.argv.includes("--fixture")
  )
    return;
  const input = readInput();
  const run = safeRoot(
    workspace,
    path.join(
      workspace,
      "artifacts/network-diagnostics",
      `run-${Date.now()}-${crypto.randomUUID()}`,
    ),
  );
  fs.mkdirSync(run, { recursive: true });
  fs.writeFileSync(
    path.join(run, ".network-owner.json"),
    JSON.stringify({ owner, id: path.basename(run) }),
  );
  const environment =
    process.platform === "win32" ? powerShell("network-environment.ps1") : null;
  const safeEnvironment = environment ?? {
    os: process.platform,
    environment_discovery: "unavailable",
    actual_outbound_route: "uncertain",
  };
  const ports = proxyCandidates(safeEnvironment);
  const probes = [];
  for (const port of ports)
    for (const protocol of ["http", "socks5"])
      probes.push(await probeLocalProxy(port, protocol));
  safeEnvironment.local_proxy_protocol_probes = probes;
  safeEnvironment.actual_outbound_route =
    "uncertain beyond observed loopback connections; explicit proxy and TUN may share the same outbound";
  fs.writeFileSync(
    path.join(run, "environment.safe.json"),
    JSON.stringify(safeEnvironment, null, 2),
  );
  const result = {
    schema_version: 1,
    test_kind: input.url
      ? /^https:\/\/(www\.)?(youtube\.com|youtu\.be)\//i.test(input.url)
        ? "actual YouTube"
        : "explicit authorized ordinary HTTP(S) control"
      : "environment only; no authorized URL",
    samples: [],
    limitations: [
      "Internet conditions are not controlled; same format and engine are held fixed but node/CDN route beyond local proxy is unconfirmed.",
      "Do not interpret missing measurements, metadata-only success or fixture self-tests as YouTube throughput.",
      "A uses production Manager/SQLite/second probe with 100ms snapshots, without WebView rendering; B/C are independent direct yt-dlp CLI children using the same production argument builder. Windows Job cleanup is shared for ownership, not media flow control.",
    ],
  };
  let engine = enginePath(input);
  let stopAll = null;
  if (!input.url) stopAll = "no_authorized_url";
  if (!engine) stopAll = "engine_unavailable";
  const harness = path.join(
    workspace,
    "src-tauri/target/debug/examples/network_bench.exe",
  );
  if (!stopAll) {
    say("正在构建当前worktree的真实生产管线测试入口。");
    const build = spawnSync(
      "cargo",
      [
        "build",
        "--manifest-path",
        "src-tauri/Cargo.toml",
        "--example",
        "network_bench",
      ],
      { cwd: workspace, windowsHide: true, stdio: "ignore", timeout: 900000 },
    );
    if (build.status !== 0) stopAll = "production_harness_build_failed";
  }
  const budget = new DownloadBudget({
    seconds: Math.min(45, Math.max(1, input.seconds ?? 30)),
    total: totalBudgetBytes(input.totalBudgetBytes),
  });
  const proxy = input.proxy ? localProxy(input.proxy) : null;
  if (input.proxy && !proxy) stopAll = "invalid_local_proxy";
  const confirmed = probes.filter((p) => p.confirmed);
  const chosen = proxy
    ? confirmed.find(
        (p) => p.port === proxy.port && p.protocol === proxy.protocol,
      )
    : (confirmed.find((p) => p.protocol === "http") ?? confirmed[0]);
  if (proxy && !chosen) stopAll = "configured_local_proxy_not_confirmed";
  let metadata = null,
    versions = null,
    chosenFormat = null;
  let preflightOutcome = null;
  async function execute(
    action,
    condition,
    repetition,
    explicit = false,
    fragments = null,
  ) {
    const sampleRoot = safeRoot(
      workspace,
      path.join(run, `${condition}-${repetition}-${crypto.randomUUID()}`),
    );
    fs.mkdirSync(sampleRoot);
    const metrics = new TransferMetrics();
    let final = null,
      failure = null,
      http = null,
      args = null;
    let observedFirstProgress = null;
    let observedStages = [];
    const route = [],
      routePromises = [];
    let nextRouteAt = 0,
      routing = false;
    const options = {
      action,
      url: input.url,
      enginePath: engine,
      ffmpegPath: input.ffmpegPath ?? "",
      runRoot: run,
      sampleRoot,
      formatId: chosenFormat?.id ?? input.formatId ?? null,
      proxy:
        explicit && chosen
          ? `${chosen.protocol}://127.0.0.1:${chosen.port}`
          : null,
      fragments,
      maxSeconds: action === "metadata" ? 45 : budget.seconds,
      maxBytes: budget.roundLimit(),
    };
    say(
      `${condition} / ${repetition}：${action === "metadata" ? "格式预检查（不下载）" : "受控实际下载"}。`,
    );
    const child = await runSafeChild(harness, [], {
      cwd: workspace,
      input: options,
      hardSeconds: action === "metadata" ? 50 : 45,
      onRecord(record) {
        if (record.type === "stage") {
          const stage = safeStage(record.stage);
          if (stage) observedStages = mergeStages(observedStages, [stage]);
        }
        const firstProgress = firstProgressDuration(record);
        if (firstProgress !== null) observedFirstProgress ??= firstProgress;
        if (record.type === "metadata") metadata = record;
        if (record.type === "versions") versions = record.versions;
        if (record.type === "arguments") args = record.summary;
        if (record.type === "failure")
          failure = {
            error_code: record.error_code,
            error_kind: record.error_kind ?? null,
            phase: record.phase ?? null,
          };
        if (record.type === "http_error") {
          http = record.code;
          return "site_access_restricted";
        }
        if (record.type === "progress") {
          metrics.add(record.elapsed_s, record.bytes, record.phase);
          if (record.bytes >= budget.roundLimit()) return "round_byte_budget";
        }
        if (record.type === "result") final = record;
        return null;
      },
      onTick(elapsed, ownedPid) {
        if (
          action !== "metadata" &&
          process.platform === "win32" &&
          elapsed >= nextRouteAt &&
          !routing
        ) {
          routing = true;
          nextRouteAt = elapsed + 8;
          // A bounded read-only process snapshot; only numeric route evidence is retained.
          routePromises.push(
            powerShellAsync(
              "network-connections.ps1",
              [
                "-OwnedPid",
                String(ownedPid),
                "-ProxyPort",
                String(chosen?.port ?? 0),
              ],
              3500,
            )
              .then((observation) => {
                if (observation) route.push(observation);
              })
              .finally(() => {
                routing = false;
              }),
          );
        }
        return null;
      },
    });
    await Promise.allSettled(routePromises);
    metrics.stop(child.duration_s);
    const measurement = metrics.summary();
    measurement.first_observed_progress_clock_s = measurement.first_progress_s;
    const knownStages = mergeStages(observedStages, final?.stages);
    const firstStage = knownStages.find(
      (s) => s.phase === "download_first_progress" && s.status === "ok",
    );
    measurement.first_progress_s = Number.isFinite(final?.first_progress_s)
      ? final.first_progress_s
      : Number.isFinite(firstStage?.duration_ms)
        ? firstStage.duration_ms / 1000
        : observedFirstProgress;
    measurement.first_progress_clock =
      "process-spawn to first progress when recorded; otherwise unmeasured";
    let fileBytes = 0;
    try {
      fileBytes = mediaBytes(
        action === "manager" &&
          fs.existsSync(path.join(sampleRoot, "downloads"))
          ? path.join(sampleRoot, "downloads")
          : sampleRoot,
      );
    } catch {
      stopAll = "owned_media_inspection_failed";
    }
    const observed = Math.max(
      measurement.observed_total_bytes,
      fileBytes,
      Number.isFinite(final?.budget_observed_bytes)
        ? final.budget_observed_bytes
        : 0,
    );
    if (action !== "metadata") budget.account(observed);
    if (action !== "metadata")
      result.samples.push({
        condition,
        repetition,
        sample_kind: result.test_kind,
        format_id: chosenFormat?.id ?? null,
        protocol: chosenFormat?.protocol ?? null,
        segmented: chosenFormat?.segmented ?? null,
        engine_versions: versions,
        proxy_mode: explicit
          ? `explicit local ${chosen?.protocol ?? "unavailable"} ${chosen?.port ?? ""}`
          : "inherited OS/environment proxy or TUN; route not assumed",
        route_evidence: route,
        argv_whitelist: args,
        metrics: measurement,
        terminal: final?.terminal ?? (child.killed ? "runner_cutoff" : null),
        stop_reason: final?.stop_reason ?? child.stop_reason,
        stages: knownStages,
        live_stage_evidence_count: observedStages.length,
        http_errors: http ? [http] : (final?.http_errors ?? null),
        retries: final?.retries ?? null,
        failure,
        process_exit_code: child.code,
        pipeline_wall_seconds: child.duration_s,
        budget_observed_bytes: observed,
        budget_filesystem_bytes: fileBytes,
        local_resources: {
          cpu: "not measured",
          memory: "not measured",
          disk_throughput:
            "not measured; media file sizes checked for budget only",
        },
        cleanup: "pending",
      });
    let cleanup = "passed";
    try {
      safeRoot(workspace, sampleRoot);
      fs.rmSync(sampleRoot, { recursive: true, force: false });
    } catch {
      cleanup = "failed_owned_artifacts_retained";
      stopAll = "owned_cleanup_failed";
    }
    if (action !== "metadata") result.samples.at(-1).cleanup = cleanup;
    if (http === 403 || http === 429 || final?.terminal === "failed" || failure)
      stopAll = http
        ? `HTTP_${http}_stop`
        : "engine_failure_stop_no_repeated_requests";
    if (!budget.remaining) stopAll = "total_budget_exhausted";
    return { child, failure, http, cleanup };
  }
  if (!stopAll) {
    const pre = await execute("metadata", "preflight", 1);
    preflightOutcome = {
      attempted: true,
      error: pre.failure,
      http_error: pre.http,
      process_exit_code: pre.child.code,
      runner_stop_reason: pre.child.stop_reason,
      cleanup: pre.cleanup,
    };
    if (!stopAll && (!metadata || pre.failure || pre.child.code !== 0))
      stopAll = "format_preflight_failed";
    if (!stopAll) {
      const formats = metadata.formats;
      chosenFormat = selectFormat(formats, input.formatId ?? null);
      if (!chosenFormat || metadata.playlist)
        stopAll = "no_comparable_single_video_format";
    }
  }
  result.preflight = {
    attempted: preflightOutcome?.attempted ?? false,
    outcome: preflightOutcome,
    metadata_received: metadata !== null,
    selected_format: chosenFormat,
    available_formats: metadata?.formats ?? [],
    engine_versions: versions,
    no_media_download: true,
  };
  if (input.preflightOnly && !stopAll) {
    stopAll = "preflight_only_requested";
    result.test_kind = "YouTube/HTTP metadata only; no throughput measured";
  }
  if (!stopAll && budget.remaining < 1024 * 1024)
    stopAll = "remaining_budget_too_small";
  if (!stopAll) {
    const repetitions = Math.min(2, Math.max(1, input.repetitions ?? 2));
    const conditions = conditionList(input.conditions);
    for (let i = 1; i <= repetitions && !stopAll; i++) {
      for (const condition of i === 1
        ? conditions
        : [...conditions].reverse()) {
        if (stopAll) break;
        if (condition === "C" && !chosen) continue;
        await execute(
          condition === "A" ? "manager" : "cli",
          condition,
          i,
          condition === "C",
        );
        if (budget.remaining < 1024 * 1024)
          stopAll = "remaining_budget_too_small";
      }
    }
    if (
      !stopAll &&
      input.conditions === undefined &&
      chosenFormat.segmented &&
      chosen &&
      budget.remaining >= 40 * 1024 * 1024
    )
      await execute("cli", "D-fragments-4", 1, true, 4);
  }
  result.engine_versions = versions;
  result.selected_format = chosenFormat;
  result.selected_conditions = conditionList(input.conditions);
  result.comparison_scope = {
    selection_policy: input.formatId
      ? "explicit pinned video format"
      : "highest recognized video height <= default 1080 preset; no preference for lower-resolution muxed media",
    audio_selection: chosenFormat?.has_audio
      ? "audio included in selected format"
      : "production format+bestaudio retained; audio format ID is not separately pinned",
    partial_download: true,
    completed_media_or_postprocessing_not_assumed: true,
  };
  result.stop_reason = stopAll;
  result.remaining_budget_bytes = budget.remaining;
  result.budget_limits = {
    round_seconds: budget.seconds,
    round_bytes: budget.bytes,
    total_bytes: totalBudgetBytes(input.totalBudgetBytes),
    absolute_max_total_bytes: 300 * 1024 * 1024,
  };
  if (!chosen)
    result.limitations.push(
      "No process-owned loopback HTTP/SOCKS proxy could be confirmed; explicit-proxy C was not run.",
    );
  if (!chosenFormat?.segmented)
    result.limitations.push(
      "No actual DASH/HLS segmented format selected; -N4 was skipped. HTTP chunk experiments were not enabled without evidence.",
    );
  result.limitations.push(
    "未在本机重现100 KB/s时不能断言根因；未形成同条件重复样本时不计算加速百分比。The runner does not switch nodes or alter network settings.",
  );
  fs.writeFileSync(
    path.join(run, "benchmark-results.safe.json"),
    JSON.stringify(result, null, 2),
  );
  fs.writeFileSync(
    path.join(run, "benchmark-summary.md"),
    summaryMarkdown(result),
  );
  say(
    `已完成：${result.samples.length}轮实际下载记录；脱敏结果位于当前worktree artifacts/network-diagnostics/${path.basename(run)}/。`,
  );
  if (stopAll) say(`自动停止原因：${stopAll}。`);
}
if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch(() => {
    say(
      "诊断未完成：安全输入、子进程或本地环境检查失败；未输出原始错误/私有参数。",
    );
    process.exitCode = 1;
  });
}
