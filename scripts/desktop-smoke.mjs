// Runs the real debug desktop executable against an original local media fixture.
// No mock IPC, task data or download process is used.
import { chromium, expect } from '@playwright/test';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
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
// Refuse an existing diagnostic listener before starting a test application.
// Otherwise CDP could attach to another debug instance with its own data.
await new Promise((resolve, reject) => {
  const probe = net.createServer();
  probe.once('error', error => reject(new Error('Diagnostic port 9322 must be free before desktop smoke starts.', { cause: error })));
  probe.listen(9322, '127.0.0.1', () => probe.close(error => error ? reject(error) : resolve()));
});
const runId = new Date().toISOString().replace(/[:.]/g, '-');
const artifacts = path.join(root, 'artifacts');
const dataDir = path.join(artifacts, 'e2e-data', runId);
const outputDir = path.join(artifacts, 'e2e-downloads', runId, '中文保存目录');
fs.mkdirSync(outputDir, { recursive: true });
function readTestDatabase(database) {
  const result = spawnSync('python', ['-c', "import json,pathlib,sqlite3,sys; c=sqlite3.connect(pathlib.Path(sys.argv[1]).resolve().as_uri()+'?mode=ro',uri=True); print(json.dumps({'schema':c.execute('PRAGMA user_version').fetchone()[0],'settings':json.loads(c.execute('SELECT document FROM app_settings WHERE id=1').fetchone()[0]),'rows':[{'id':r[0],'raw':r[1],'task':json.loads(r[1])} for r in c.execute('SELECT id,document FROM tasks')]})); c.close()", database], { windowsHide: true, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(`Could not inspect isolated test database: ${result.stderr}`);
  return JSON.parse(result.stdout);
}
let seed;
if (process.env.FLUXDL_SMOKE_SEED_DATABASE) {
  const source = fs.realpathSync(process.env.FLUXDL_SMOKE_SEED_DATABASE);
  if (!source.startsWith(fs.realpathSync(artifacts) + path.sep)) throw new Error('Upgrade smoke seed must be an isolated artifact, never the live user database');
  seed = readTestDatabase(source);
  fs.mkdirSync(dataDir, { recursive: true });
  fs.copyFileSync(source, path.join(dataDir, 'fluxdl.sqlite3'));
}
const fixture = await startMediaServer();
const baseUrl = `http://127.0.0.1:${fixture.port}`;
const report = { runId, engine, dataDir, outputDir, checks: [], errors: [], events: [] };
let application, browser, workspace, quick;
function pass(name) { report.checks.push(name); console.log(`PASS ${name}`); }
async function launch() {
  application = spawn(executable, [], { cwd: root, windowsHide: true, env: { ...process.env, FLUXDL_TEST_DATA_DIR: dataDir, WEBVIEW2_USER_DATA_FOLDER: path.join(dataDir, 'webview'), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=9322' }, stdio: ['ignore', 'pipe', 'pipe'] });
  application.stdout.on('data', () => {});
  application.stderr.on('data', data => { const value = data.toString(); if (!value.includes('DevTools listening')) console.log(value.slice(0, 800)); });
  const deadline = Date.now() + 45000;
  let ready = false;
  let lastConnectionError;
  while (Date.now() < deadline) {
    if (application.exitCode !== null) throw new Error(`Desktop exited before diagnostic startup (code ${application.exitCode})`);
    try { const response = await fetch('http://127.0.0.1:9322/json/version'); if (response.ok) { ready = true; break; } lastConnectionError = new Error(`Diagnostic endpoint returned HTTP ${response.status}`); } catch (error) { lastConnectionError = error; }
    await delay(250);
  }
  if (!ready) throw new Error('Desktop diagnostic endpoint did not become ready', { cause: lastConnectionError });
  browser = await chromium.connectOverCDP('http://127.0.0.1:9322');
  for (let i = 0; i < 80; i++) {
    const pages = browser.contexts().flatMap(c => c.pages());
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
  for (const page of [workspace, quick]) {
    await page.evaluate(async () => {
      window.__FLUXDL_SMOKE_TRACE__ = [];
      for (const event of ['app-state-updated', 'app-state-changed']) {
        const handler = window.__TAURI_INTERNALS__.transformCallback(({ payload }) => {
          const change = payload.change || payload;
          window.__FLUXDL_SMOKE_TRACE__.push({
            event, type: change.type || change.kind || 'snapshot',
            taskId: change.taskId || change.task?.id || null,
            bytes: new TextEncoder().encode(JSON.stringify(payload)).length,
            at: performance.now(), sessionId: payload.sessionId || null,
            revision: payload.revision ?? null,
          });
        });
        await window.__TAURI_INTERNALS__.invoke('plugin:event|listen', { event, target: { kind: 'Any' }, handler });
      }
    });
  }
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
  await collectTrace('graceful-exit');
  const process = application;
  const exited = new Promise(resolve => process.once('exit', resolve));
  await rpc('request_exit');
  if (process.exitCode == null) await Promise.race([exited, delay(15000).then(() => { throw new Error('App did not exit safely'); })]);
  await browser.close().catch(() => {});
  browser = null; application = null;
}
async function collectTrace(segment) {
  for (const [label, page] of [['workspace', workspace], ['quick', quick]]) {
    const records = await page.evaluate(() => window.__FLUXDL_SMOKE_TRACE__ || []);
    report.events.push({ segment, window: label, records });
  }
}
try {
  await launch();
  const initial = await snapshot();
  if (seed) {
    const statuses = new Set(['queued', 'resolving', 'downloading', 'postprocessing', 'cancelling', 'completed', 'cancelled', 'failed', 'interrupted']);
    const active = new Set(['queued', 'resolving', 'downloading', 'postprocessing', 'cancelling']);
    const known = seed.rows.filter(row => statuses.has(row.task.status));
    assert.equal(initial.tasks.length, known.length, 'Legacy known records must all remain visible');
    for (const { task: previous } of known) {
      const current = initial.tasks.find(task => task.id === previous.id);
      assert(current, 'Legacy task IDs must survive migration');
      assert.equal(current.status, active.has(previous.status) ? 'interrupted' : previous.status);
      for (const field of ['createdAt', 'outputDir', 'mode', 'quality', 'formatId']) assert.equal(current[field], previous[field]);
      assert.deepEqual(current.media, Object.fromEntries(Object.entries(previous.media).filter(([key]) => key !== 'futureMediaField')));
    }
    for (const key of Object.keys(initial.settings)) assert.deepEqual(initial.settings[key], seed.settings[key]);
    report.upgrade = { originalSchema: seed.schema, originalRows: seed.rows.length, visibleLegacyRows: known.length };
    pass('legacy database preserves tasks/settings and interrupts queued work without auto-resume');
  }
  await rpc('update_settings', { settings: { ...initial.settings, enginePath: engine, downloadDir: outputDir, defaultMode: 'video', quality: '720', completionAction: 'none', theme: 'dark', density: 'comfortable' } });
  assert((await snapshot()).engines.ytdlp.available);
  pass('existing local engine configured and detected');
  if (process.env.FLUXDL_SMOKE_ERROR_PROTOCOL === '1') {
    const failures = await workspace.evaluate(async () => {
      const url = 'https://SECRET:SECRET@example.com/media.mp4?token=SECRET';
      const reject = async args => {
        try {
          await window.__TAURI_INTERNALS__.invoke('probe_media', args);
          return { rejected: false, error: null };
        } catch (error) {
          return { rejected: true, error };
        }
      };
      return {
        legacy: await reject({ url }),
        structured: await reject({ url, errorFormat: 'structured' }),
      };
    });
    assert(failures.legacy.rejected && typeof failures.legacy.error === 'string');
    assert(failures.structured.rejected);
    assert.equal(failures.structured.error.version, 1);
    assert.equal(failures.structured.error.code, 'INVALID_INPUT');
    assert.equal(failures.structured.error.phase, 'validate');
    assert(failures.structured.error.message.length > 0);
    assert(!JSON.stringify(failures).includes('SECRET'));
    pass('real IPC preserves legacy String errors and negotiates safe structured errors');
  }
  await rpc('show_quick_capture');
  assert.notEqual(await workspace.evaluate(() => window.__TAURI_INTERNALS__.metadata.currentWindow.label), await quick.evaluate(() => window.__TAURI_INTERNALS__.metadata.currentWindow.label));
  await quick.getByRole('textbox', { name: '媒体链接' }).fill(`${baseUrl}/fast.mp4`);
  await quick.getByRole('button', { name: /解析链接/ }).click();
  await expect(quick.getByRole('heading', { name: 'fast', exact: true })).toBeVisible({ timeout: 30000 });
  // Heading visibility precedes Tauri's async native resize. A screenshot in
  // that interval can capture the old 260px WebView viewport; wait for geometry.
  await quick.waitForFunction(() => {
    const shell = document.querySelector('.quick-shell');
    return shell && innerHeight >= Math.min(720, shell.scrollHeight) && Math.abs(innerHeight - outerHeight) <= 2;
  });
  await quick.screenshot({ path: path.join(artifacts, 'quick-capture-parsed.png') });
  await quick.getByRole('button', { name: '加入下载队列', exact: true }).click();
  await expect(quick.getByRole('button', { name: '已加入队列', exact: true })).toBeVisible({ timeout: 30000 });
  const added = (await snapshot()).tasks.at(-1);
  const completed = await taskState(added.id, ['completed']);
  assert(fs.existsSync(completed.finalPath));
  assert(fs.statSync(completed.finalPath).size > 100000);
  assert(completed.downloadedBytes > 0, 'A completed download must retain real byte progress');
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
  if (process.env.FLUXDL_SMOKE_AUDIO === '1') {
    const media = await rpc('probe_media', { url: `${baseUrl}/fast-audio.mp4` });
    const audio = await rpc('enqueue_download', { input: { media, mode: 'audio', quality: 'best', formatId: null, outputDir } });
    const converted = await taskState(audio.id, ['completed']);
    assert.equal(path.extname(converted.finalPath).toLowerCase(), '.mp3');
    const inspected = spawnSync('ffprobe', ['-v', 'error', '-show_entries', 'stream=codec_name,codec_type', '-of', 'json', converted.finalPath], { windowsHide: true, encoding: 'utf8' });
    assert.equal(inspected.status, 0, inspected.stderr);
    const streams = JSON.parse(inspected.stdout).streams;
    assert(streams.length > 0 && streams.every(stream => stream.codec_type === 'audio' && stream.codec_name === 'mp3'));
    await workspace.getByRole('button', { name: '下载历史', exact: true }).click();
    await workspace.getByRole('button', { name: `查看任务：${converted.media.title}`, exact: true }).click();
    await expect(workspace.getByRole('heading', { name: converted.media.title, exact: true })).toBeVisible();
    await workspace.getByText('技术诊断与日志', { exact: true }).click();
    await expect(workspace.getByRole('button', { name: '复制日志', exact: true })).toBeVisible();
    assert(converted.logs.length > 0, 'Real postprocessing diagnostics must be preserved');
    await workspace.screenshot({ path: path.join(artifacts, 'task-details-audio.png') });
    await workspace.getByRole('button', { name: '关闭详情', exact: true }).click();
    pass('real MP3 extraction is ffprobe verified and task details/logs remain available');
  }
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
  await collectTrace('before-forced-termination');
  spawnSync(path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'taskkill.exe'), ['/PID', String(crashPid), '/T', '/F'], { windowsHide: true });
  await browser.close().catch(() => {}); browser = null; application = null;
  await delay(1000);
  await launch();
  assert.equal((await snapshot()).tasks.find(t => t.id === crashTask.id).status, 'interrupted');
  pass('forced application termination restores active task as interrupted');
  await stopGracefully();
  if (seed) {
    const current = readTestDatabase(path.join(dataDir, 'fluxdl.sqlite3'));
    assert.equal(current.schema, 2);
    for (const previous of seed.rows) {
      const row = current.rows.find(row => row.id === previous.id);
      assert(row, 'Migration/regression must never delete a legacy row');
      if (previous.task.status === 'future-reserved-state') assert.equal(row.raw, previous.raw, 'Unknown state row must remain byte-for-byte intact');
      if (previous.task.futureTopLevel) assert.deepEqual(row.task.futureTopLevel, previous.task.futureTopLevel);
      if (previous.task.media.futureMediaField) assert.equal(row.task.media.futureMediaField, previous.task.media.futureMediaField);
    }
    assert.deepEqual(current.settings.futurePreference, seed.settings.futurePreference);
    report.upgrade.schemaAfter = current.schema;
    report.upgrade.originalRowsRetained = true;
    report.upgrade.unknownFieldsRetained = true;
    report.upgrade.unknownStateRawRetained = true;
    pass('v1 to v2 upgrade retains every original row and unknown JSON fields after real app lifecycle');
  }
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
