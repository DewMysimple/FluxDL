// Real native picker/tray verification for one controller-owned debug instance.
// Never attaches to an existing diagnostic port or acts on another FluxDL PID.
import { chromium } from "@playwright/test";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { finished } from "node:stream/promises";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { startMediaServer } from "./test-media-server.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const expectedVersion = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version;
assert(typeof expectedVersion === "string" && /^\d+\.\d+\./.test(expectedVersion), "Package version metadata must include major/minor/patch");
const engineArgument = process.argv[2] || process.env.YTDLP_PATH;
if (process.platform !== "win32") throw new Error("Native smoke requires Windows.");
if (!engineArgument || !fs.existsSync(engineArgument) || process.argv.length > 3)
  throw new Error("Usage: node scripts/native-desktop-smoke.mjs C:\\path\\to\\yt-dlp.exe");
const engine = fs.realpathSync(engineArgument);
const executable = path.join(root, "src-tauri", "target", "debug", "fluxdl.exe");
const helperPath = path.join(root, "scripts", "native-smoke.ps1");
const powershell = path.join(process.env.SystemRoot || "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
const taskkill = path.join(process.env.SystemRoot || "C:\\Windows", "System32", "taskkill.exe");
const endpoint = "http://127.0.0.1:9322";
const runId = `${new Date().toISOString().replace(/[:.]/g, "-")}-${randomUUID().slice(0, 8)}`;
const artifacts = path.join(root, "artifacts");
const runDirectory = path.join(artifacts, "native-smoke", runId);
const dataDir = path.join(runDirectory, "data");
const outputDir = path.join(runDirectory, "downloads");
fs.mkdirSync(outputDir, { recursive: true });
fs.mkdirSync(dataDir, { recursive: true });

const report = {
  schemaVersion: 1, runId, generatedAt: new Date().toISOString(),
  engine, executable, dataDir, outputDir,
  checks: [], errors: [], runtimeErrors: [], helpers: {},
  limitations: [
    "Only the isolated controller-owned debug process is an interaction target.",
    "Registered tray callbacks exercise the real native menu/callback chain; they do not prove a physical pointer hit.",
    "Unavailable UIAutomation capabilities remain unmeasured, never passed.",
    "Directory pickers are cancelled without selecting files or changing the clipboard.",
  ],
};
const evidence = [];
let application, applicationIdentity, browser, workspace, fixture;
let protectedProcesses = [];

class NativeUnavailable extends Error {}
function check(name, status = "passed") {
  report.checks.push({ name, status });
  console.log(`${status.toUpperCase()} ${name}`);
}
function writeJson(filename, value) {
  fs.writeFileSync(filename, JSON.stringify(value, null, 2) + "\n");
}
function rememberEvidence(label, filename) {
  evidence.push({ label, sha256: createHash("sha256").update(fs.readFileSync(filename)).digest("hex") });
}
async function bounded(promise, milliseconds, description) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`Timed out: ${description}`)), milliseconds); }),
    ]);
  } finally { clearTimeout(timer); }
}
function powershellJson(script, environment = {}) {
  const result = spawnSync(powershell, ["-NoProfile", "-NonInteractive", "-Command", "[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false); $ErrorActionPreference = 'Stop'; " + script], {
    windowsHide: true, encoding: "utf8", timeout: 15000, maxBuffer: 1024 * 1024,
    env: { ...process.env, ...environment },
  });
  if (result.error || result.status !== 0)
    throw new Error("Read-only native process inspection failed", { cause: result.error || new Error(result.stderr) });
  return JSON.parse(result.stdout.replace(/^\uFEFF/, "").trim());
}
function identity(processId) {
  assert(Number.isSafeInteger(processId) && processId > 0);
  // Preserve the .NET round-trip string, including all seven fractional digits.
  return powershellJson("$nativeProcess = Get-Process -Id ([int]$env:FLUXDL_NATIVE_PID) -ErrorAction SilentlyContinue; if ($null -eq $nativeProcess) { 'null' } else { [ordered]@{ processId=$nativeProcess.Id; processStartTimeUtc=$nativeProcess.StartTime.ToUniversalTime().ToString('o'); executable=$nativeProcess.Path } | ConvertTo-Json -Compress }", { FLUXDL_NATIVE_PID: String(processId) });
}
function existingFluxProcesses() {
  return powershellJson("$items = @(Get-Process -Name fluxdl -ErrorAction SilentlyContinue | ForEach-Object { [ordered]@{ processId=$_.Id; processStartTimeUtc=$_.StartTime.ToUniversalTime().ToString('o') } }); ConvertTo-Json -InputObject $items -Compress");
}
function sameIdentity(left, right) {
  return Boolean(left && right && left.processId === right.processId && left.processStartTimeUtc === right.processStartTimeUtc && left.executable.toLowerCase() === right.executable.toLowerCase());
}
async function requireUnusedPort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once("error", () => reject(new Error("Diagnostic port 9322 is already occupied; no existing process was touched.")));
    server.listen({ host: "127.0.0.1", port: 9322, exclusive: true }, resolve);
  });
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
}
function verifyDiagnosticOwner() {
  // A free-port check alone has a bind race. Refuse CDP attachment unless each
  // listener belongs to this application's own WebView process ancestry.
  const owners = powershellJson("$listeners = @(Get-NetTCPConnection -State Listen -LocalPort 9322 -ErrorAction SilentlyContinue); $items = @($listeners | ForEach-Object { $nativeOwner = [int]$_.OwningProcess; $chain = @(); for ($step=0; $step -lt 16 -and $nativeOwner -gt 0; $step++) { $chain += $nativeOwner; $nativeNode = Get-CimInstance Win32_Process -Filter ('ProcessId=' + $nativeOwner); if ($null -eq $nativeNode) { break }; $nativeOwner = [int]$nativeNode.ParentProcessId }; [ordered]@{ processId=$_.OwningProcess; ancestors=$chain } }); ConvertTo-Json -InputObject $items -Depth 4 -Compress");
  assert(owners.length > 0 && owners.every(owner => owner.ancestors.includes(application.pid)), "Diagnostic listener must belong to the exact controller-owned process tree");
  assert(sameIdentity(applicationIdentity, identity(application.pid)), "Owned application identity changed before CDP attachment");
  report.diagnosticOwners = owners;
}
async function launch() {
  assert(fs.existsSync(executable), "Build the debug executable before native smoke");
  assert(fs.existsSync(helperPath), "Native PowerShell helper must exist");
  protectedProcesses = existingFluxProcesses();
  report.protectedProcesses = protectedProcesses;
  await requireUnusedPort();
  application = spawn(executable, [], {
    cwd: root, windowsHide: true,
    env: { ...process.env, FLUXDL_TEST_DATA_DIR: dataDir, WEBVIEW2_USER_DATA_FOLDER: path.join(dataDir, "webview"), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-address=127.0.0.1 --remote-debugging-port=9322" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let launchError;
  application.on("error", error => { launchError = error; });
  report.applicationLog = "";
  for (const stream of [application.stdout, application.stderr]) stream.on("data", data => { report.applicationLog = (report.applicationLog + data.toString()).slice(-24000); });
  assert(Number.isSafeInteger(application.pid), "Native test application must have its own PID");
  applicationIdentity = identity(application.pid);
  assert(applicationIdentity && path.resolve(applicationIdentity.executable).toLowerCase() === executable.toLowerCase(), "Exact debug executable identity must match");
  report.processIdentity = applicationIdentity;
  const proof = { schemaVersion: 1, processId: application.pid, processStartTimeUtc: applicationIdentity.processStartTimeUtc, debugExecutable: executable, testDataDir: dataDir, testGate: true };
  const proofFile = path.join(dataDir, "native-smoke-target.json");
  writeJson(proofFile, proof);
  rememberEvidence("controller-isolation-proof", proofFile);
  const deadline = Date.now() + 45000;
  let connected = false;
  while (Date.now() < deadline) {
    if (launchError) throw launchError;
    if (application.exitCode !== null) throw new Error(`Native test application exited during launch: ${application.exitCode}`);
    try {
      const response = await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(1500) });
      if (response.ok) { connected = true; break; }
    } catch { /* Retry only this newly launched application's diagnostic startup. */ }
    await delay(250);
  }
  assert(connected, "Owned diagnostic endpoint did not become ready");
  verifyDiagnosticOwner();
  browser = await chromium.connectOverCDP(endpoint);
  for (let attempt = 0; attempt < 100; attempt++) {
    const pages = browser.contexts().flatMap(context => context.pages());
    workspace = pages.find(page => page.url().includes("tauri") && !page.url().includes("window=quick"));
    if (workspace) break;
    await delay(200);
  }
  assert(workspace, "Owned Workspace WebView must exist");
  for (const page of browser.contexts().flatMap(context => context.pages()))
    page.on("pageerror", error => report.runtimeErrors.push(error.message));
  await workspace.waitForFunction(() => Boolean(window.__TAURI_INTERNALS__), undefined, { timeout: 30000 });
  await workspace.getByRole("heading", { name: /^下载工作台/ }).waitFor({ timeout: 30000 });
  assert(fs.existsSync(path.join(dataDir, "fluxdl.sqlite3")), "Isolated test database must exist");
  check("isolated debug instance, exact process start time and owned diagnostic listener");
}
function rpc(command, args) {
  return bounded(workspace.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args }), 45000, `owned IPC ${command}`);
}
async function runHelper(phase, flag) {
  assert(sameIdentity(applicationIdentity, identity(application.pid)), "Native helper target identity changed");
  const source = path.join(dataDir, "native-smoke.json");
  // The previous phase has already been copied to runDirectory. Do not mistake
  // it for a fresh helper report if the next invocation fails before writing.
  if (fs.existsSync(source)) fs.unlinkSync(source);
  const stdoutFile = path.join(runDirectory, `${phase}.stdout.txt`);
  const stderrFile = path.join(runDirectory, `${phase}.stderr.txt`);
  const child = spawn(powershell, ["-NoProfile", "-NonInteractive", "-File", helperPath, "-TargetProcessId", String(application.pid), "-TestDataDir", dataDir, "-WorkspaceRoot", root, flag], { cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
  const stdout = fs.createWriteStream(stdoutFile);
  const stderr = fs.createWriteStream(stderrFile);
  child.stdout.pipe(stdout);
  child.stderr.pipe(stderr);
  child.stdout.on("data", data => {
    for (const line of data.toString("utf8").split(/\r?\n/))
      if (/^(PASS|UNMEASURED) /.test(line)) console.log(`NATIVE ${line}`);
  });
  let timedOut = false;
  const timeout = setTimeout(() => { timedOut = true; child.kill(); }, 120000);
  const result = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (exitCode, signal) => resolve({ exitCode, signal }));
  }).finally(() => clearTimeout(timeout));
  await Promise.all([finished(stdout), finished(stderr)]);
  let nativeReport;
  if (fs.existsSync(source)) {
    nativeReport = JSON.parse(fs.readFileSync(source, "utf8").replace(/^\uFEFF/, ""));
    const saved = path.join(runDirectory, `${phase}.json`);
    writeJson(saved, nativeReport);
    rememberEvidence(phase, saved);
  }
  report.helpers[phase] = { ...result, timedOut, report: nativeReport ?? null };
  rememberEvidence(`${phase}-stdout`, stdoutFile);
  rememberEvidence(`${phase}-stderr`, stderrFile);
  assert(!timedOut, `Native ${phase} helper timed out; its actions are not passed`);
  assert(nativeReport, `Native ${phase} helper produced no report`);
  assert.equal(nativeReport.processId, application.pid);
  assert.equal(nativeReport.gate.controllerIsolationProof, true);
  assert.equal(nativeReport.cancelDialogOnly, flag === "-CancelDialogOnly");
  for (const item of nativeReport.checks) check(`${phase}: ${item.name}`, item.status);
  if (result.exitCode === 2) throw new NativeUnavailable(`Native ${phase} helper could not safely complete; inspect its raw report`);
  assert.equal(result.exitCode, 0, `Native ${phase} helper failed; inspect its raw report`);
  assert.equal(nativeReport.errors.length, 0, `Native ${phase} helper reported errors`);
  assert.equal(nativeReport.summary.failed, 0, `Native ${phase} helper reported failed checks`);
  assert(nativeReport.protectedProcessesUnchanged.every(item => item.sameProcessStillRunning), "Native helper could not confirm other FluxDL processes stayed unchanged");
  return nativeReport;
}
async function verifyNativeDialog() {
  const title = `FluxDL Native Smoke ${application.pid}`;
  // Keep the real plugin Promise pending while the native helper cancels only
  // this PID's exact, uniquely titled dialog. Catch immediately for cleanup.
  const dialog = workspace.evaluate(title => window.__TAURI_INTERNALS__.invoke("plugin:dialog|open", { options: { directory: true, multiple: false, title } }), title)
    .then(value => ({ value }), error => ({ error: String(error) }));
  await runHelper("native-dialog", "-CancelDialogOnly");
  const result = await bounded(dialog, 15000, "native directory picker Promise cancellation");
  assert(!result.error, result.error);
  assert.equal(result.value, null, "Real dialog plugin must resolve null after native Cancel");
  report.pickerReturnedNull = true;
  check("real dialog plugin Promise resolves null after owned native Cancel");
}
async function configure() {
  fixture = await startMediaServer();
  const initial = await rpc("get_app_state");
  await rpc("update_settings", { settings: { ...initial.settings, enginePath: engine, downloadDir: outputDir, defaultMode: "video", quality: "720", completionAction: "none", closeBehavior: "tray" } });
  const configured = await rpc("get_app_state");
  assert(configured.engines.ytdlp.available, "Local yt-dlp must be detected");
  assert.equal(configured.settings.closeBehavior, "tray");
  check("local engine detected and isolated close behavior configured for tray");
  const footer = (await workspace.locator(".sidebar-version > span").last().innerText()).trim();
  const expectedFooter = `v${expectedVersion.split(".").slice(0, 2).join(".")}`;
  report.version = { expected: expectedVersion, workspaceFooter: footer };
  assert.equal(footer, expectedFooter, "Real Workspace footer must use the current package version");
  const previousPage = await workspace.locator("nav button.active").getAttribute("title");
  assert(previousPage, "Current navigation page must be known before About verification");
  try {
    await workspace.locator("nav").getByRole("button", { name: "设置", exact: true }).click();
    const about = (await workspace.locator(".about-card > span").innerText()).trim();
    report.version.settingsAbout = about;
    assert.equal(about, `FluxDL ${expectedVersion} · MIT License`, "Real About card must show the full package version");
  } finally {
    await workspace.locator("nav").getByRole("button", { name: previousPage, exact: true }).click();
  }
  check("real Workspace footer and About versions match package metadata");
}
async function startRealDownload() {
  const url = `http://127.0.0.1:${fixture.port}/native-slow.mp4`;
  const media = await rpc("probe_media", { url });
  assert(media.formats.length > 0, "Real media fixture must produce downloadable formats");
  const task = await rpc("enqueue_download", { input: { media, mode: "video", quality: "720", formatId: null, outputDir } });
  report.download = { taskId: task.id, url };
  const deadline = Date.now() + 45000;
  while (Date.now() < deadline) {
    const current = (await rpc("get_app_state")).tasks.find(item => item.id === task.id);
    assert(current, "Real native smoke task must remain in backend state");
    if (current.status === "failed") throw new Error(`Native smoke real download failed: ${current.error}`);
    if (current.status === "completed") throw new Error("Native-slow fixture completed before native shutdown testing");
    if (current.status === "downloading" && current.downloadedBytes > 0) {
      report.download = { ...report.download, observedStatus: current.status, downloadedBytes: current.downloadedBytes };
      check("real local native-slow download is downloading with nonzero bytes");
      return;
    }
    await delay(150);
  }
  throw new Error("Real native-slow download never reported nonzero downloading bytes");
}
async function verifyTrayExit() {
  const nativeReport = await runHelper("native-tray", "-AllowExit");
  if (application.exitCode === null) {
    const exited = await Promise.race([new Promise(resolve => application.once("exit", () => resolve(true))), delay(3000).then(() => false)]);
    if (!exited) throw new NativeUnavailable("Native tray Exit did not exit the owned application; no graceful-exit pass is claimed");
  }
  assert.equal(application.exitCode, 0, "Owned application must exit normally from the native tray menu");
  assert.equal(nativeReport.targetExitCode, 0);
  if (nativeReport.engineStopped?.status === "failed") throw new Error("Owned engine process handle did not signal after native tray Exit");
  if (nativeReport.engineStopped?.status !== "passed") {
    check("owned running engine exit confirmed by a fixed kernel process handle", "unmeasured");
  } else {
    assert.equal(nativeReport.engineStopped.observedAliveBeforeQuit, true);
    assert.equal(nativeReport.engineStopped.kernelHandleSignaledAfterQuit, true);
    check("owned running engine exit confirmed by a fixed kernel process handle");
  }
  assert(nativeReport.trayRoutes?.length > 0, "A native tray interaction route must be recorded");
  report.trayRoutes = nativeReport.trayRoutes;
  check("owned application exited normally through its real native tray menu");
}
async function cleanup() {
  report.cleanup = { forcedOwnedTermination: false, mediaServerClosed: !fixture };
  if (application && application.exitCode === null && applicationIdentity) {
    try {
      assert(sameIdentity(applicationIdentity, identity(application.pid)), "Cleanup refuses a reused or foreign PID");
      if (workspace) await bounded(rpc("request_exit"), 1500, "owned cleanup request_exit").catch(() => {});
      const deadline = Date.now() + 14000;
      while (application.exitCode === null && Date.now() < deadline) await delay(100);
      if (application.exitCode === null) {
        assert(sameIdentity(applicationIdentity, identity(application.pid)), "Forced cleanup refuses a reused or foreign PID");
        report.cleanup.forcedOwnedTermination = true;
        const result = spawnSync(taskkill, ["/PID", String(application.pid), "/T", "/F"], { windowsHide: true, encoding: "utf8", timeout: 15000 });
        report.cleanup.taskkillExitCode = result.status;
        if (result.error || result.status !== 0) throw new Error("Could not terminate the exact controller-owned debug process tree");
        report.errors.push("Native verification required forced cleanup of its own test process; graceful exit was not established.");
      }
    } catch (error) { report.errors.push(String(error.stack || error)); }
  }
  if (browser) await bounded(browser.close(), 5000, "owned CDP disconnect").catch(error => report.errors.push(String(error)));
  if (fixture) {
    fixture.server.closeAllConnections();
    await bounded(new Promise((resolve, reject) => fixture.server.close(error => error ? reject(error) : resolve())), 5000, "local fixture server shutdown");
    report.cleanup.mediaServerClosed = true;
  }
  const current = existingFluxProcesses();
  report.protectedProcessesUnchanged = protectedProcesses.map(previous => ({ processId: previous.processId, sameProcessStillRunning: current.some(item => item.processId === previous.processId && item.processStartTimeUtc === previous.processStartTimeUtc) }));
  assert(report.protectedProcessesUnchanged.every(item => item.sameProcessStillRunning), "A pre-existing FluxDL process changed; preservation cannot be claimed");
}
function saveReports() {
  report.generatedAt = new Date().toISOString();
  report.summary = {
    passed: report.checks.filter(item => item.status === "passed").length,
    failed: report.checks.filter(item => item.status === "failed").length,
    unmeasured: report.checks.filter(item => item.status === "unmeasured").length,
  };
  report.status = report.errors.length || report.runtimeErrors.length || report.summary.failed ? "failed" : report.blocked || report.summary.unmeasured ? "partial" : "passed";
  const rawFile = path.join(runDirectory, "native-desktop-smoke.json");
  writeJson(rawFile, report);
  rememberEvidence("native-desktop-smoke-raw", rawFile);
  const summary = {
    schemaVersion: 1, runId, status: report.status, summary: report.summary,
    assertions: report.checks.map(({ name, status }) => ({ name, status })),
    errors: report.errors.length, runtimeErrors: report.runtimeErrors.length,
    ...(report.version ? { version: report.version } : {}),
    pickerReturnedNull: report.pickerReturnedNull === true,
    appExitCode: application?.exitCode ?? null,
    engineStopped: report.helpers["native-tray"]?.report?.engineStopped?.status || "unmeasured",
    trayRoutes: [...new Set((report.trayRoutes || report.helpers["native-tray"]?.report?.trayRoutes || []).map(item => item.route))],
    protectedProcesses: { observed: protectedProcesses.length, unchanged: report.protectedProcessesUnchanged?.filter(item => item.sameProcessStillRunning).length ?? 0 },
    cleanup: report.cleanup,
    limitations: report.limitations,
    evidence,
  };
  // Only these path-free results/hashes are suitable for a published summary.
  writeJson(path.join(runDirectory, "native-smoke-summary.json"), summary);
  writeJson(path.join(artifacts, "native-desktop-smoke-summary.json"), summary);
  writeJson(path.join(artifacts, "native-desktop-smoke.json"), report);
  console.log(JSON.stringify(summary, null, 2));
  process.exitCode = report.status === "passed" ? 0 : report.status === "partial" ? 2 : 1;
}

try {
  await launch();
  await configure();
  await verifyNativeDialog();
  await startRealDownload();
  await verifyTrayExit();
  assert.equal(report.runtimeErrors.length, 0, "Native WebViews must not raise runtime errors");
  check("no frontend runtime exceptions during native interaction");
} catch (error) {
  if (error instanceof NativeUnavailable) {
    report.blocked = error.message;
    check("native verification could not safely cover all required actions", "unmeasured");
  } else report.errors.push(String(error.stack || error));
  console.error(error.message);
} finally {
  try { await cleanup(); } catch (error) { report.errors.push(String(error.stack || error)); }
  saveReports();
}
