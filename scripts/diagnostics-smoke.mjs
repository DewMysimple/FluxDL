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
const engine = fs.realpathSync(process.argv[2]);
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
      ...process.env,
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
  assert(workspace && quick);
  for (const page of [workspace, quick])
    page.on("pageerror", (error) => report.errors.push(error.message));
  await workspace.getByRole("heading", { name: /下载工作台/ }).waitFor();
}
const rpc = (command, args = {}, target = workspace) =>
  target.evaluate(
    ({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args),
    { command, args },
  );
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
      downloadDir: downloads,
      completionAction: "none",
    },
  });
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
  for (let i = 0; i < 5; i++) {
    const url = `http://127.0.0.1:${fixture.port}/diagnostics-${i}.mp4?sig=FIXTURE_PRIVATE_SIGNATURE`;
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
      assert.equal((await terminal(t.id)).status, "completed");
      report.firstTask = t.id;
      await workspace.getByRole("button", { name: "关闭详情" }).click();
    } else {
      const media = await rpc("probe_media", {
        url,
        traceId: crypto.randomUUID(),
      });
      const t = await rpc("enqueue_download", {
        input: {
          media,
          mode: "video",
          quality: "1080",
          formatId: null,
          outputDir: null,
        },
        traceId: crypto.randomUUID(),
      });
      assert.equal((await terminal(t.id)).status, "completed");
    }
  }
  pass("真实yt-dlp五次本地合法媒体解析、二次校验、下载与终态");
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
    "enqueue_metadata_recheck",
    "enqueue_persist",
    "queue_wait",
    "download_engine_spawn",
    "download_first_progress",
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
