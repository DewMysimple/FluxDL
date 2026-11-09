// Real Windows WebView2 + user-provided yt-dlp + original local HTTP media.
// Only this script's debug child/profile/data are controlled. No production data.
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
import { startMediaServer } from "./test-media-server.mjs";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const explorerHelper = path.join(
  root,
  "scripts/diagnostics-explorer-check.ps1",
);
const explorerSnapshot = spawnSync(
  "powershell.exe",
  ["-NoProfile", "-STA", "-File", explorerHelper, "-Action", "Snapshot"],
  { windowsHide: true, encoding: "utf8" },
);
assert.equal(explorerSnapshot.status, 0, explorerSnapshot.stderr);
const originalExplorerHandles =
  JSON.parse(explorerSnapshot.stdout.trim()).join(",") || "none";
const engine = fs.realpathSync(process.argv[2]);
const ffmpegLookup = spawnSync("where.exe", ["ffmpeg.exe"], {
  windowsHide: true,
  encoding: "utf8",
});
assert.equal(ffmpegLookup.status, 0, "Real MP3 smoke requires local FFmpeg");
const ffmpeg = fs.realpathSync(ffmpegLookup.stdout.trim().split(/\r?\n/)[0]);
// Isolate only the owned child's PATH so the missing-dependency case cannot
// silently fall back to another host FFmpeg. Normal cases use its explicit path.
const ownedEnvironment = { ...process.env };
const pathKey = Object.keys(ownedEnvironment).find(
  (key) => key.toLowerCase() === "path",
);
const ownedPath = (ownedEnvironment[pathKey] ?? "")
  .split(path.delimiter)
  .filter(
    (entry) =>
      !["ffmpeg.exe", "ffprobe.exe"].some((name) =>
        fs.existsSync(path.join(entry, name)),
      ),
  )
  .join(path.delimiter);
if (pathKey) delete ownedEnvironment[pathKey];
ownedEnvironment.PATH = ownedPath;
const repetitions = Number(process.argv[3] ?? 5);
const latencyAB = process.argv[4] === "--latency-ab";
assert(Number.isInteger(repetitions) && repetitions >= 5 && repetitions <= 40);
const run = path.join(
  root,
  "artifacts",
  "diagnostics",
  `desktop-${Date.now()}`,
);
fs.mkdirSync(run, { recursive: true });
const data = path.join(run, "data");
const downloads = path.join(run, "downloads");
fs.mkdirSync(downloads);
const report = {
  checks: [],
  errors: [],
  fixture: false,
  network:
    "loopback HTTP original generated media; no proxy configured; repeat downloads to unique local files",
  measurements: [],
  latencySamples: [],
  snapshots: [],
};
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
await new Promise((resolve, reject) => {
  const server = net.createServer();
  server.once("error", reject);
  server.listen(9322, "127.0.0.1", () => server.close(resolve));
});
const fixture = await startMediaServer();
let app, browser, workspace, quick;
const pass = (message) => {
  report.checks.push(message);
  console.log(`PASS ${message}`);
};
async function launch() {
  app = spawn(path.join(root, "src-tauri/target/debug/fluxdl.exe"), [], {
    cwd: root,
    windowsHide: true,
    env: {
      ...ownedEnvironment,
      FLUXDL_TEST_DATA_DIR: data,
      WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"),
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  app.stdout.on("data", () => {});
  app.stderr.on("data", () => {});
  let ready = false;
  for (let i = 0; i < 180; i++) {
    if (app.exitCode !== null)
      throw new Error(`owned app exited ${app.exitCode}`);
    try {
      if ((await fetch("http://127.0.0.1:9322/json/version")).ok) {
        ready = true;
        break;
      }
    } catch {
      /* Endpoint or child can be transitioning; bounded caller retries. */
    }
    await wait(250);
  }
  assert(ready, "CDP ready");
  const owner = spawnSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-Command",
      "$expected=[int]$env:FLUXDL_OWNED_APP_PID; $connections=@(Get-NetTCPConnection -LocalPort 9322 -State Listen -ErrorAction Stop); if (!$connections.Count) {throw 'No diagnostic listener'}; foreach ($c in $connections) {$candidate=[int]$c.OwningProcess; $found=$false; for($depth=0;$depth -lt 12;$depth++){if($candidate -eq $expected){$found=$true;break}; $p=Get-CimInstance Win32_Process -Filter ('ProcessId='+$candidate); if(!$p){break};$candidate=[int]$p.ParentProcessId};if(!$found){throw 'Diagnostic listener is not owned by this child'}}",
    ],
    {
      windowsHide: true,
      encoding: "utf8",
      env: { ...process.env, FLUXDL_OWNED_APP_PID: String(app.pid) },
    },
  );
  assert.equal(owner.status, 0, owner.stderr);
  browser = await chromium.connectOverCDP("http://127.0.0.1:9322");
  for (let i = 0; i < 80; i++) {
    const pages = browser.contexts().flatMap((c) => c.pages());
    workspace = pages.find(
      (p) =>
        (p.url().includes("tauri") ||
          p.url().startsWith("http://127.0.0.1:1420")) &&
        !p.url().includes("window=quick"),
    );
    quick = pages.find((p) => p.url().includes("window=quick"));
    if (workspace && quick) break;
    await wait(250);
  }
  assert(
    workspace && quick,
    `Owned application pages not ready: ${browser
      .contexts()
      .flatMap((context) => context.pages())
      .map((page) => page.url())
      .join(", ")}`,
  );
  for (const page of [workspace, quick])
    page.on("pageerror", (error) => report.errors.push(error.message));
  await workspace.getByRole("heading", { name: /下载工作台/ }).waitFor();
}
const rpc = (command, args = {}, target = workspace) =>
  target.evaluate(
    ({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args),
    { command, args },
  );
async function measuredRpc(command, args, phase) {
  return workspace.evaluate(
    async ({ command, args, phase }) => {
      const start = performance.now();
      let success = false;
      try {
        const result = await window.__TAURI_INTERNALS__.invoke(command, args);
        success = true;
        return result;
      } finally {
        await window.__TAURI_INTERNALS__.invoke("record_frontend_diagnostic", {
          traceId: args.traceId,
          phase,
          durationMs: performance.now() - start,
          success,
        });
      }
    },
    { command, args, phase },
  );
}
async function terminal(id) {
  for (let i = 0; i < 300; i++) {
    const s = await rpc("get_app_state");
    const t = s.tasks.find((t) => t.id === id);
    if (
      t &&
      ![
        "queued",
        "resolving",
        "downloading",
        "postprocessing",
        "cancelling",
      ].includes(t.status)
    )
      return t;
    await wait(100);
  }
  throw new Error("terminal timeout");
}
async function stop() {
  await rpc("request_exit");
  for (let i = 0; i < 100 && app.exitCode === null; i++) await wait(100);
  assert.equal(app.exitCode, 0);
  await browser.close();
  browser = null;
  app = null;
}
try {
  await launch();
  let state = await rpc("get_app_state");
  await rpc("update_settings", {
    settings: {
      ...state.settings,
      enginePath: engine,
      ffmpegPath: ffmpeg,
      downloadDir: downloads,
      completionAction: "none",
    },
  });
  const configured = await rpc("get_app_state");
  report.engineAvailability = Object.fromEntries(
    Object.entries(configured.engines)
      .filter(
        ([, value]) =>
          value && typeof value === "object" && "available" in value,
      )
      .map(([name, value]) => [
        name,
        {
          available: value.available,
          source: value.source,
          version: value.version,
        },
      ]),
  );
  let health = await rpc("get_diagnostics_health");
  assert.equal(health.mode, "development");
  assert.equal(path.resolve(health.directory), path.join(root, "dev-logs"));
  pass("真实开发实例使用源码根dev-logs，数据库与profile隔离");
  await rpc("refresh_diagnostic_report");
  assert(fs.existsSync(path.join(root, "dev-logs/latest-report.md")));
  assert(
    fs
      .readdirSync(path.join(root, "dev-logs/sessions"))
      .some((f) => f.endsWith(".jsonl")),
  );
  pass("运行中自动报告与真实JSONL存在");
  for (let i = 0; i < repetitions * (latencyAB ? 2 : 1); i++) {
    const policy =
      !latencyAB || i % 4 === 0 || i % 4 === 3
        ? "trusted_preview_cache"
        : "full_recheck";
    if (latencyAB) await rpc("configure_latency_test_mode", { mode: policy });
    let finishedTask;
    const url = `http://127.0.0.1:${fixture.port}/diagnostics-${latencyAB ? Math.floor(i / 2) : i}.mp4?sig=FIXTURE_PRIVATE_SIGNATURE`;
    if (i === 0) {
      await workspace.getByLabel("媒体链接").fill(url);
      await workspace.getByRole("button", { name: /解析链接/ }).click();
      await workspace.getByRole("heading", { name: /diagnostics-0/ }).waitFor();
      await workspace
        .getByRole("button", { name: "加入下载队列", exact: true })
        .click();
      await workspace
        .getByRole("button", { name: "已加入队列", exact: true })
        .waitFor();
      state = await rpc("get_app_state");
      const t = state.tasks[0];
      finishedTask = await terminal(t.id);
      assert.equal(
        finishedTask.status,
        "completed",
        finishedTask.error ?? "terminal task",
      );
      report.firstTask = t.id;
      await workspace.getByRole("button", { name: "关闭详情" }).click();
    } else {
      const parseTrace = crypto.randomUUID();
      const media = await measuredRpc(
        "probe_media",
        { url, traceId: parseTrace },
        "parse_ui_request",
      );
      const t = await measuredRpc(
        "enqueue_download",
        {
          input: {
            media,
            mode: "video",
            quality: "1080",
            formatId: null,
            outputDir: null,
          },
          traceId: crypto.randomUUID(),
          relatedTraceId: parseTrace,
        },
        "enqueue_ui_request",
      );
      finishedTask = await terminal(t.id);
      assert.equal(finishedTask.status, "completed");
    }

    assert(fs.statSync(finishedTask.finalPath).size > 0);
    assert.equal(
      finishedTask.media.trustedPreviewRef ?? null,
      null,
      "private reference must not persist into tasks",
    );
    const records = (
      await rpc("query_diagnostic_events", {
        filter: { taskId: finishedTask.id, limit: 200 },
      })
    ).events;
    const related = records.find((e) => e.fields.related_trace_id)?.fields
      .related_trace_id;
    const parseRecords = related
      ? (
          await rpc("query_diagnostic_events", {
            filter: { traceId: related, limit: 200 },
          })
        ).events
      : [];
    const value = (phase) =>
      [...records, ...parseRecords].find(
        (e) => e.phase === phase && e.status === "ok",
      )?.duration_ms ?? null;
    const outcome =
      records.find((e) => e.fields.cache_outcome)?.fields.cache_outcome ??
      "legacy";
    if (latencyAB) {
      assert.equal(
        outcome,
        policy === "trusted_preview_cache" ? "hit" : "disabled",
      );
      assert.equal(
        records.some((e) => e.phase === "enqueue_metadata_recheck"),
        policy === "full_recheck",
      );
    }
    report.latencySamples.push({
      order: i,
      policy,
      success: true,
      fileVerified: true,
      cacheOutcome: outcome,
      frontendParseMs: value("parse_ui_request"),
      parseMs: value("parse_total"),
      enqueueResponseMs: value("enqueue_ui_request"),
      recheckMs: value("enqueue_metadata_recheck"),
      trustValidateMs: value("enqueue_trusted_validate"),
      enqueueToProgressMs: value("enqueue_to_progress"),
      spawnToProgressMs: value("download_first_progress"),
      firstOutputMs: value("download_first_output"),
      beforeDlMs: value("download_before_dl"),
      beforeDlToProgressMs: value("download_before_dl_to_progress"),
      engineSpawnMs: value("download_engine_spawn"),
      queueMs: value("queue_wait"),
      totalMs: value("download_total"),
    });
  }
  pass(
    `真实yt-dlp${repetitions * (latencyAB ? 2 : 1)}次本地合法媒体解析、复核、下载与终态`,
  );
  await rpc("refresh_diagnostic_report");
  const overview = await rpc("get_diagnostics_overview", {
    filter: { sessionId: health.sessionId },
  });
  const page = await rpc("query_diagnostic_events", {
    filter: { taskId: report.firstTask, limit: 200 },
  });
  const trace = page.events.find(
    (e) => e.phase === "download_terminal",
  ).trace_id;
  report.traceId = trace;
  const linkedParse = page.events.find((e) => e.fields.related_trace_id)?.fields
    .related_trace_id;
  assert(linkedParse, "UI parse and enqueue must be explicitly linked");
  assert(
    (
      await rpc("query_diagnostic_events", {
        filter: { traceId: linkedParse, limit: 200 },
      })
    ).events.some(
      (e) => e.phase === "parse_ui_request" && e.source === "frontend",
    ),
  );
  const linkedEvents = (
    await rpc("query_diagnostic_events", {
      filter: { traceId: trace, limit: 200 },
    })
  ).events;
  assert.equal(
    new Set(linkedEvents.map((e) => e.fields.attempt_id).filter(Boolean)).size,
    1,
  );
  for (const phase of [
    "enqueue_trusted_validate",
    "enqueue_persist",
    "queue_wait",
    "download_engine_spawn",
    "download_first_progress",
    "download_first_output",
    "download_before_dl",
    "download_before_dl_to_progress",
    "download_process_exit",
    "download_terminal",
  ]) {
    const events = await rpc("query_diagnostic_events", {
      filter: { traceId: trace, limit: 200 },
    });
    assert(
      events.events.some((e) => e.phase === phase && e.status === "ok"),
      phase,
    );
  }
  report.measurements = overview.statistics;
  report.engineVersion = (await rpc("get_app_state")).engines.ytdlp.version;
  let restartMedia;
  await rpc("configure_latency_test_mode", { mode: "trusted_preview_cache" });
  report.controlledHttp = [];
  for (const profile of ["delayed", "drop"]) {
    const media = await rpc("probe_media", {
      url: `http://127.0.0.1:${fixture.port}/${profile}-controlled.mp4`,
    });
    fixture.setResponseProfile(profile);
    let terminalTask;
    try {
      const task = await rpc("enqueue_download", {
        input: {
          media,
          mode: "video",
          quality: "1080",
          formatId: null,
          outputDir: null,
        },
        traceId: crypto.randomUUID(),
      });
      terminalTask = await terminal(task.id);
      assert.equal(
        terminalTask.status,
        profile === "drop" ? "failed" : "completed",
      );
    } finally {
      fixture.setResponseProfile("normal");
    }
    if (profile === "drop") {
      await rpc("retry_task", { taskId: terminalTask.id });
      assert.equal((await terminal(terminalTask.id)).status, "completed");
    }
    report.controlledHttp.push({
      profile,
      expectedTerminalVerified: true,
      retryVerified: profile === "drop",
    });
  }
  pass("可控延迟HTTP仍成功；断流不假报完成，恢复网络后重试成功");
  const originalSettings = (await rpc("get_app_state")).settings;
  const audioMedia = await rpc("probe_media", {
    url: `http://127.0.0.1:${fixture.port}/audio-check.mp4`,
  });
  const direct = audioMedia.formats.find((f) => f.hasVideo && f.hasAudio);
  assert(direct);
  const audioTask = await rpc("enqueue_download", {
    input: {
      media: audioMedia,
      mode: "audio",
      quality: "1080",
      formatId: direct.formatId,
      outputDir: null,
    },
  });
  const audioDone = await terminal(audioTask.id);
  assert.equal(audioDone.status, "completed");
  const snapshot = await rpc("get_app_state");
  const videoDone = snapshot.tasks.find((t) => t.id === report.firstTask);
  const probeFile = (file) => {
    const result = spawnSync(
      snapshot.engines.ffprobe.path,
      [
        "-v",
        "error",
        "-show_entries",
        "format=format_name:stream=codec_type",
        "-of",
        "json",
        file,
      ],
      { windowsHide: true, encoding: "utf8", timeout: 15000 },
    );
    assert.equal(result.status, 0);
    return JSON.parse(result.stdout);
  };
  const videoFormat = probeFile(videoDone.finalPath),
    audioFormat = probeFile(audioDone.finalPath);
  assert(
    videoFormat.streams.some((s) => s.codec_type === "video") &&
      videoFormat.streams.some((s) => s.codec_type === "audio"),
  );
  assert.equal(audioFormat.format.format_name, "mp3");
  assert(audioFormat.streams.every((s) => s.codec_type === "audio"));
  report.mediaFormats = { videoVerified: true, mp3Verified: true };
  pass("真实ffprobe确认视频含音视频，MP3仅含音轨");
  try {
    await rpc("update_settings", {
      settings: {
        ...originalSettings,
        ffmpegPath: path.join(run, "missing-ffmpeg.exe"),
      },
    });
    assert.equal((await rpc("get_app_state")).engines.ffmpeg.available, false);
    await assert.rejects(() =>
      rpc("enqueue_download", {
        input: {
          media: audioMedia,
          mode: "audio",
          quality: "1080",
          formatId: direct.formatId,
          outputDir: null,
        },
      }),
    );
  } finally {
    await rpc("update_settings", { settings: originalSettings });
  }
  pass("FFmpeg缺失时音频模式安全拒绝，原配置恢复");
  pass("Trace含实际二次校验、DB、排队、真正spawn、首次有效进度及持久终态");
  await rpc("show_task_diagnostics", { taskId: report.firstTask });
  await workspace.getByRole("heading", { name: "诊断中心." }).waitFor();
  await workspace
    .getByRole("button", { name: new RegExp(trace) })
    .first()
    .click();
  await workspace.getByRole("heading", { name: "阶段时间轴" }).waitFor();
  await workspace.getByLabel("Trace排序").selectOption("slow");
  assert(
    (await workspace.locator(".diagnostic-trace-list button").count()) > 0,
  );
  await workspace.screenshot({ path: path.join(run, "diagnostics-dark.png") });
  pass("实际任务跳转诊断性能时间轴");
  await workspace.getByRole("button", { name: "复制 Trace ID" }).click();
  await workspace.getByText("Trace ID 已复制", { exact: true }).waitFor();
  pass("实际复制Trace ID");
  await workspace
    .getByRole("button", { name: "复制本任务诊断摘要", exact: true })
    .click();
  const safeSummary = await workspace.evaluate(() =>
    navigator.clipboard.readText(),
  );
  assert(safeSummary.includes(trace) && safeSummary.includes(linkedParse));
  assert(
    !safeSummary.includes("FIXTURE_PRIVATE_SIGNATURE") &&
      !safeSummary.includes(downloads) &&
      !safeSummary.includes("http://"),
  );
  pass("本任务安全摘要复制包含关联解析与代码版本，不含URL或保存路径");
  await workspace
    .getByRole("button", { name: "打开日志目录", exact: true })
    .click();
  await workspace
    .getByText("已请求资源管理器打开日志目录", { exact: true })
    .waitFor();
  pass("真实Explorer日志目录请求成功");
  const explorerFolder = spawnSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-STA",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      path.join(root, "scripts/diagnostics-explorer-check.ps1"),
      "-Folder",
      path.join(root, "dev-logs"),
    ],
    { windowsHide: true, encoding: "utf8" },
  );
  assert.equal(explorerFolder.status, 0, explorerFolder.stderr);
  pass("真实Explorer窗口已打开实际日志目录");
  for (const tab of ["概览", "事件", "异常", "性能"]) {
    await workspace.getByRole("tab", { name: tab, exact: true }).click();
  }
  pass("四个真实诊断视图可切换");
  await workspace.getByRole("tab", { name: "事件", exact: true }).click();
  await workspace.getByLabel("搜索诊断").fill("download_terminal");
  await workspace.getByRole("button", { name: "应用筛选" }).click();
  await wait(500);
  pass("事件搜索与受控分页接口运行");
  await workspace
    .getByRole("button", { name: "导出诊断包", exact: true })
    .click();
  await workspace.getByRole("dialog").waitFor();
  await workspace.screenshot({ path: path.join(run, "export-preview.png") });
  await workspace.getByRole("button", { name: "关闭对话框" }).click();
  pass("真实导出内容预览与取消无文件写入");
  await workspace
    .getByRole("button", { name: "导出诊断包", exact: true })
    .click();
  await workspace.getByRole("button", { name: "选择文件名并导出" }).click();
  const exportPath = path.join(run, "FluxDL-Diagnostics-smoke.zip");
  const dialog = spawn(
    "powershell.exe",
    [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      path.join(root, "scripts/diagnostics-export-dialog.ps1"),
      "-ProcessId",
      String(app.pid),
      "-Destination",
      exportPath,
    ],
    { windowsHide: true, stdio: ["ignore", "pipe", "pipe"] },
  );
  let dialogOutput = "";
  dialog.stdout.on("data", (d) => (dialogOutput += d.toString()));
  dialog.stderr.on("data", (d) => (dialogOutput += d.toString()));
  const dialogExit = await new Promise((resolve) => dialog.on("exit", resolve));
  assert.equal(dialogExit, 0, dialogOutput);
  await workspace
    .getByText("诊断包已保存到您选择的位置，未上传。", { exact: true })
    .waitFor();
  assert(fs.existsSync(exportPath));
  const zip = spawnSync(
    "python",
    [
      "-c",
      "import sys,zipfile,json; z=zipfile.ZipFile(sys.argv[1]); assert z.testzip() is None; assert len(z.namelist())==6; t=b'\\n'.join(z.read(n) for n in z.namelist()); assert b'FIXTURE_PRIVATE_SIGNATURE' not in t; assert b'Users' not in t",
      exportPath,
    ],
    { windowsHide: true, encoding: "utf8" },
  );
  assert.equal(zip.status, 0, zip.stderr);
  pass("真实原生保存对话框确认、ZIP导出及独立CRC/隐私检查");
  await workspace.getByRole("button", { name: "定位导出文件" }).click();
  await workspace
    .getByText("已请求定位最近导出文件", { exact: true })
    .waitFor();
  pass("导出后Explorer定位请求成功");
  const explorerFile = spawnSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-STA",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      path.join(root, "scripts/diagnostics-explorer-check.ps1"),
      "-Folder",
      run,
      "-SelectedFile",
      exportPath,
    ],
    { windowsHide: true, encoding: "utf8" },
  );
  assert.equal(explorerFile.status, 0, explorerFile.stderr);
  pass("真实Explorer窗口已选中本次导出ZIP");
  await assert.rejects(() => rpc("open_diagnostics_directory", {}, quick));
  await assert.rejects(() => rpc("export_diagnostics_bundle", {}, quick));
  pass("Quick Capture不可执行诊断目录或导出系统操作");
  state = await rpc("get_app_state");
  await rpc("update_settings", {
    settings: { ...state.settings, theme: "light" },
  });
  await workspace.screenshot({ path: path.join(run, "diagnostics-light.png") });
  assert.equal(
    await quick.evaluate(() => document.documentElement.dataset.theme),
    "light",
  );
  pass("诊断页浅色及Quick Capture主题同步");
  restartMedia = await rpc("probe_media", {
    url: `http://127.0.0.1:${fixture.port}/restart-reference.mp4`,
  });
  const session = health.sessionId;
  await stop();
  assert(fs.existsSync(path.join(root, "dev-logs/latest-report.md")));
  await launch();
  health = await rpc("get_diagnostics_health");
  assert.notEqual(health.sessionId, session);
  assert(
    (
      await rpc("query_diagnostic_events", {
        filter: { traceId: trace, limit: 200 },
      })
    ).events.length > 0,
  );
  assert.equal(
    (
      await rpc(
        "query_diagnostic_events",
        { filter: { traceId: trace, limit: 200 } },
        quick,
      )
    ).events.length > 0,
    true,
  );
  pass("退出日志保留，重启新session且双窗可查询旧Trace");
  const restartedTask = await rpc("enqueue_download", {
    input: {
      media: restartMedia,
      mode: "video",
      quality: "1080",
      formatId: null,
      outputDir: null,
    },
    traceId: crypto.randomUUID(),
  });
  assert.equal((await terminal(restartedTask.id)).status, "completed");
  const restartRecords = (
    await rpc("query_diagnostic_events", {
      filter: { taskId: restartedTask.id, limit: 200 },
    })
  ).events;
  assert(restartRecords.some((e) => e.fields.cache_outcome === "missing"));
  assert(
    restartRecords.some(
      (e) => e.phase === "enqueue_metadata_recheck" && e.status === "ok",
    ),
  );
  pass("重启后旧可信引用失效并走完整安全复核");
  const fallbackValue = (phase) =>
    restartRecords.find((e) => e.phase === phase && e.status === "ok")
      ?.duration_ms ?? null;
  report.fallbackSamples = [
    {
      policy: "restart_missing",
      samples: 1,
      recheckMs: fallbackValue("enqueue_metadata_recheck"),
      enqueueToProgressMs: fallbackValue("enqueue_to_progress"),
      spawnToProgressMs: fallbackValue("download_first_progress"),
    },
  ];
  const records = await rpc("query_diagnostic_events", {
    filter: { limit: 200 },
  });
  const text =
    JSON.stringify(records) +
    fs.readFileSync(path.join(root, "dev-logs/latest-report.md"), "utf8");
  assert(!text.includes("FIXTURE_PRIVATE_SIGNATURE"));
  assert(!text.includes(downloads));
  pass("真实URL虚构签名与私有保存目录不进入诊断查询/报告");
  await stop();
  assert.equal(report.errors.length, 0);
  pass("无前端运行异常，正常退出自有进程");
} catch (error) {
  report.errors.push(String(error.stack || error));
  console.error(error);
  process.exitCode = 1;
} finally {
  for (const folder of [path.join(root, "dev-logs"), run]) {
    const cleanup = spawnSync(
      "powershell.exe",
      [
        "-NoProfile",
        "-STA",
        "-File",
        explorerHelper,
        "-Action",
        "Cleanup",
        "-Folder",
        folder,
        "-BeforeHandles",
        originalExplorerHandles,
      ],
      { windowsHide: true, encoding: "utf8" },
    );
    if (cleanup.status !== 0) {
      report.errors.push("Explorer cleanup did not complete");
      process.exitCode = 1;
    }
  }
  if (app) {
    try {
      await rpc("request_exit");
    } catch {
      /* Endpoint or child can be transitioning; bounded caller retries. */
    }
    await wait(1000);
    if (app.exitCode === null)
      spawnSync("taskkill.exe", ["/PID", String(app.pid), "/T", "/F"], {
        windowsHide: true,
      });
  }
  if (browser) await browser.close().catch(() => {});
  fixture.server.closeAllConnections();
  fixture.server.close();
  fs.writeFileSync(
    path.join(run, "report.json"),
    JSON.stringify(report, null, 2),
  );
  console.log(`Report: ${path.join(run, "report.json")}`);
}
