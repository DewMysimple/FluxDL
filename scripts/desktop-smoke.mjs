// Runs the real debug desktop executable against an original local media fixture.
// No mock IPC, task data or download process is used.
import { chromium, expect } from '@playwright/test';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import assert from 'node:assert/strict';
import { startMediaServer } from './test-media-server.mjs';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const engine = process.argv[2] || process.env.YTDLP_PATH;
if (!engine || !fs.existsSync(engine)) throw new Error('Usage: node scripts/desktop-smoke.mjs C:\\path\\to\\yt-dlp.exe');
const executable = path.join(root, 'src-tauri/target/debug/fluxdl.exe');
if (!fs.existsSync(executable)) throw new Error('First run: npx tauri build --debug --no-bundle');
const runId = new Date().toISOString().replace(/[:.]/g, '-');
const artifacts = path.join(root, 'artifacts');
const dataDir = path.join(artifacts, 'e2e-data', runId);
const outputDir = path.join(artifacts, 'e2e-downloads', runId, '中文保存目录');
fs.mkdirSync(outputDir, { recursive: true });
const fixture = await startMediaServer();
const baseUrl = `http://127.0.0.1:${fixture.port}`;
const report = { runId, engine, dataDir, outputDir, checks: [], errors: [], events: [] };
let application, browser, workspace, quick;
function pass(name) { report.checks.push(name); console.log(`PASS ${name}`); }
async function launch() {
  application = spawn(executable, [], { cwd: root, windowsHide: true, env: { ...process.env, FLUXDL_TEST_DATA_DIR: dataDir, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=9322' }, stdio: ['ignore', 'pipe', 'pipe'] });
  application.stdout.on('data', () => {});
  application.stderr.on('data', data => { const value = data.toString(); if (!value.includes('DevTools listening')) console.log(value.slice(0, 800)); });
  const deadline = Date.now() + 45000;
  while (Date.now() < deadline) {
    try { const response = await fetch('http://127.0.0.1:9322/json/version'); if (response.ok) break; } catch {}
    await delay(250);
  }
  browser = await chromium.connectOverCDP('http://127.0.0.1:9322');
  let pages = [];
  for (let i = 0; i < 80; i++) {
    pages = browser.contexts().flatMap(c => c.pages());
    workspace = pages.find(p => !p.url().includes('window=quick') && p.url().includes('tauri'));
    quick = pages.find(p => p.url().includes('window=quick'));
    if (workspace && quick) break;
    await delay(250);
  }
  assert(workspace && quick, 'Both native webviews must exist');
  workspace.on('pageerror', error => report.errors.push(error.message));
  quick.on('pageerror', error => report.errors.push(error.message));
  await workspace.waitForFunction(() => !!window.__TAURI_INTERNALS__);
  await workspace.getByRole('heading', { name: /^下载工作台/ }).waitFor();
}
async function rpc(command, args, target = workspace) { return target.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args }); }
async function snapshot() { return rpc('get_app_state'); }
async function taskState(id, acceptable, timeout = 45000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const task = (await snapshot()).tasks.find(t => t.id === id);
    if (acceptable.includes(task?.status)) return task;
    if (task?.status === 'failed' && !acceptable.includes('failed')) throw new Error(`Unexpected failed task: ${task.error}\n${task.logs.join('\n')}`);
    await delay(150);
  }
  throw new Error(`Timeout waiting for ${id}: ${acceptable}`);
}
async function enqueue(url, title) {
  const media = await rpc('probe_media', { url });
  assert(media.formats.length > 0);
  return rpc('enqueue_download', { input: { media: { ...media, title: title || media.title }, mode: 'video', quality: '720', formatId: null, outputDir } });
}
async function stopGracefully() {
  const process = application;
  const exited = new Promise(resolve => process.once('exit', resolve));
  await rpc('request_exit');
  if (process.exitCode == null) await Promise.race([exited, delay(15000).then(() => { throw new Error('App did not exit safely'); })]);
  await browser.close().catch(() => {});
  browser = null; application = null;
}
try {
  await launch();
  const initial = await snapshot();
  await rpc('update_settings', { settings: { ...initial.settings, enginePath: engine, downloadDir: outputDir, defaultMode: 'video', quality: '720', completionAction: 'none' } });
  assert((await snapshot()).engines.ytdlp.available);
  pass('existing local engine configured and detected');
  await rpc('show_quick_capture');
  assert.notEqual(await workspace.evaluate(() => window.__TAURI_INTERNALS__.metadata.currentWindow.label), await quick.evaluate(() => window.__TAURI_INTERNALS__.metadata.currentWindow.label));
  await quick.getByRole('textbox', { name: '媒体链接' }).fill(`${baseUrl}/fast.mp4`);
  await quick.getByRole('button', { name: /解析链接/ }).click();
  await expect(quick.getByRole('heading', { name: 'fast', exact: true })).toBeVisible({ timeout: 30000 });
  await quick.screenshot({ path: path.join(artifacts, 'quick-capture-parsed.png') });
  await quick.getByRole('button', { name: '加入下载队列', exact: true }).click();
  await expect(quick.getByRole('button', { name: '已加入队列', exact: true })).toBeVisible({ timeout: 30000 });
  const added = (await snapshot()).tasks.at(-1);
  const completed = await taskState(added.id, ['completed']);
  assert(fs.existsSync(completed.finalPath));
  assert(fs.statSync(completed.finalPath).size > 100000);
  pass('Quick Capture UI parses real media and downloads real file');
  await rpc('show_workspace');
  await workspace.screenshot({ path: path.join(artifacts, 'workspace-dark.png') });
  await rpc('show_quick_capture');
  await quick.screenshot({ path: path.join(artifacts, 'quick-capture.png') });
  await rpc('hide_quick_capture');
  const first = await enqueue(`${baseUrl}/slow-first.mp4`);
  await taskState(first.id, ['downloading']);
  const second = await enqueue(`${baseUrl}/fast-second.mp4`);
  assert.equal((await snapshot()).tasks.find(t => t.id === second.id).status, 'queued');
  await workspace.locator('nav').getByRole('button', { name: /下载队列/ }).click();
  await expect(workspace.getByRole('progressbar', { name: '下载进度' })).toBeVisible();
  await workspace.screenshot({ path: path.join(artifacts, 'queue-progress.png') });
  await rpc('cancel_task', { taskId: second.id });
  assert.equal((await snapshot()).tasks.find(t => t.id === second.id).status, 'cancelled');
  pass('serial queue keeps second task waiting and cancels queued task');
  await workspace.getByRole('button', { name: '关闭窗口', exact: true }).click();
  await expect(workspace.getByRole('heading', { name: '下载仍在进行中' })).toBeVisible();
  await workspace.getByRole('button', { name: '在后台继续', exact: true }).click();
  await delay(1000);
  const background = (await snapshot()).tasks.find(t => t.id === first.id);
  assert(background.status === 'downloading' && background.downloadedBytes > 0);
  await rpc('show_workspace');
  await rpc('cancel_task', { taskId: first.id });
  await taskState(first.id, ['cancelled']);
  pass('workspace close offers background behavior; real running process cancels');
  await rpc('retry_task', { taskId: second.id });
  await taskState(second.id, ['completed']);
  pass('cancelled task retries and completes');
  const failure = await enqueue(`${baseUrl}/slow-failure.mp4`);
  await taskState(failure.id, ['downloading']);
  await rpc('cancel_task', { taskId: failure.id });
  await taskState(failure.id, ['cancelled']);
  fixture.setUnavailable(true);
  await rpc('retry_task', { taskId: failure.id });
  const failed = await taskState(failure.id, ['failed']);
  assert(failed.error?.length);
  fixture.setUnavailable(false);
  await rpc('retry_task', { taskId: failure.id });
  await taskState(failure.id, ['completed'], 60000);
  pass('actual HTTP failure has readable error and retry works');
  let state = await snapshot();
  await rpc('update_settings', { settings: { ...state.settings, theme: 'light', density: 'compact' } });
  await workspace.getByRole('button', { name: '下载历史', exact: true }).click();
  await expect(workspace.getByRole('heading', { name: '下载历史', exact: true })).toBeVisible();
  await workspace.screenshot({ path: path.join(artifacts, 'history-light-compact.png') });
  assert.equal(await quick.evaluate(() => document.documentElement.dataset.theme), 'light');
  pass('theme and density settings synchronize across native windows');
  const missing = (await snapshot()).tasks.find(t => t.id === added.id);
  fs.renameSync(missing.finalPath, `${missing.finalPath}.moved`);
  assert.equal((await snapshot()).tasks.find(t => t.id === added.id).fileExists, false);
  await assert.rejects(() => rpc('open_download_file', { taskId: added.id }));
  await rpc('remove_history_record', { taskId: added.id });
  assert(fs.existsSync(`${missing.finalPath}.moved`));
  pass('missing file detected; removing history never removes local media');
  const beforeRestart = (await snapshot()).tasks.length;
  await stopGracefully();
  await delay(500);
  await launch();
  state = await snapshot();
  assert.equal(state.settings.theme, 'light');
  assert.equal(state.settings.density, 'compact');
  assert.equal(state.tasks.length, beforeRestart);
  pass('real application restart preserves settings and history');
  const crashTask = await enqueue(`${baseUrl}/slow-crash.mp4`);
  await taskState(crashTask.id, ['downloading']);
  const crashPid = application.pid;
  assert(Number.isInteger(crashPid) && crashPid > 0);
  spawnSync(path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'taskkill.exe'), ['/PID', String(crashPid), '/T', '/F'], { windowsHide: true });
  await browser.close().catch(() => {}); browser = null; application = null;
  await delay(1000);
  await launch();
  assert.equal((await snapshot()).tasks.find(t => t.id === crashTask.id).status, 'interrupted');
  pass('forced application termination restores active task as interrupted');
  await stopGracefully();
  assert.equal(report.errors.length, 0, report.errors.join('\n'));
  pass('no frontend runtime exceptions');
} catch (error) {
  report.errors.push(String(error?.stack || error));
  if (workspace) await workspace.screenshot({ path: path.join(artifacts, 'desktop-smoke-failure.png') }).catch(() => {});
  console.error(error);
  process.exitCode = 1;
} finally {
  if (application) {
    await rpc('request_exit').catch(() => {});
    await delay(1500);
    if (application.exitCode == null) spawnSync('taskkill.exe', ['/PID', String(application.pid), '/T', '/F'], { windowsHide: true });
  }
  if (browser) await browser.close().catch(() => {});
  fixture.server.closeAllConnections(); fixture.server.close();
  fs.writeFileSync(path.join(artifacts, 'desktop-smoke.json'), JSON.stringify(report, null, 2));
}
