// Same current DOM, two stylesheets: visual equivalence for the stage 2 extraction.
// This starts an isolated headless browser and a private ephemeral Vite server.
import { chromium } from '@playwright/test';
import { createServer } from 'vite';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const defaultBaseline = 'cdc47a33c78e22160e7c4ba75f51adb6abb420f5';
const options = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index];
  const value = process.argv[index + 1];
  if (!['--baseline-ref', '--output-dir', '--browser-executable'].includes(key) || !value) {
    throw new Error('Usage: node scripts/check-style-equivalence.mjs [--baseline-ref <ref>] [--output-dir artifacts/<new-directory>] [--browser-executable <browser.exe>]');
  }
  options.set(key, value);
}
const baselineRef = options.get('--baseline-ref') || defaultBaseline;
const git = args => execFileSync('git', args, { cwd: root, encoding: 'utf8', windowsHide: true }).trim();
const baselineCommit = git(['rev-parse', '--verify', '--end-of-options', `${baselineRef}^{commit}`]);
const baselineFiles = [];
function readBaseline(file, ancestors = []) {
  if (ancestors.includes(file)) throw new Error(`Circular baseline CSS import: ${file}`);
  baselineFiles.push(file);
  const content = git(['show', `${baselineCommit}:${file}`]);
  const expanded = content.replace(/@import\s+["']([^"']+)["'];/g, (_match, imported) => {
    if (!imported.startsWith('.') || !imported.endsWith('.css')) throw new Error(`Unsupported baseline CSS import: ${imported}`);
    const target = path.posix.normalize(path.posix.join(path.posix.dirname(file), imported));
    if (!target.startsWith('src/styles/')) throw new Error(`Baseline import escapes styles: ${target}`);
    return readBaseline(target, [...ancestors, file]);
  });
  if (/@import\b/.test(expanded)) throw new Error(`Unsupported @import syntax in ${file}; use a compatible baseline ref`);
  return expanded;
}
const css = readBaseline('src/styles/app.css');
const runId = new Date().toISOString().replace(/[:.]/g, '-');
const directory = path.resolve(root, options.get('--output-dir') || `artifacts/style-equivalence/${runId}`);
const artifacts = path.join(root, 'artifacts');
if (!directory.startsWith(`${artifacts}${path.sep}`)) throw new Error('Output must be a new directory inside project artifacts/');
if (fs.existsSync(directory)) throw new Error('Refusing to overwrite an existing evidence directory');
fs.mkdirSync(directory, { recursive: true });
const relative = file => path.relative(root, file).replaceAll(path.sep, '/');
const freeze = '*{transition:none!important;animation:none!important}';
const edgeCandidates = process.platform === 'win32' ? [
  path.join(process.env['ProgramFiles(x86)'] || 'C:/Program Files (x86)', 'Microsoft/Edge/Application/msedge.exe'),
  path.join(process.env.ProgramFiles || 'C:/Program Files', 'Microsoft/Edge/Application/msedge.exe'),
] : [];
const executablePath = options.get('--browser-executable') || process.env.FLUXDL_BROWSER_EXECUTABLE || edgeCandidates.find(file => fs.existsSync(file));
if (executablePath && !fs.existsSync(executablePath)) throw new Error('Selected browser executable does not exist');
const report = {
  schemaVersion: 1, runId, baselineRef, baselineCommit, baselineFiles,
  baselineCssSha256: createHash('sha256').update(css).digest('hex'),
  method: 'Same current DOM with current styles versus baseline-ref styles; PNG bytes compared with animations and caret disabled',
  scope: 'CSS extraction only; empty browser preview; not an original-DOM, native Tauri, DPI or download regression test',
  environment: { node: process.version, platform: process.platform, browser: null, browserSource: executablePath ? 'selected/system executable, fresh headless profile' : 'Playwright bundled Chromium, fresh headless profile' },
  checks: [], errors: [],
};
let server, browser;
try {
  server = await createServer({ root, configFile: path.join(root, 'vite.config.ts'), logLevel: 'error', clearScreen: false, server: { host: '127.0.0.1', port: 0, strictPort: false, open: false } });
  await server.listen();
  const address = server.httpServer.address();
  if (!address || typeof address === 'string') throw new Error('Vite did not bind a private TCP port');
  const baseUrl = `http://127.0.0.1:${address.port}`;
  browser = await chromium.launch({ headless: true, ...(executablePath ? { executablePath } : {}) });
  report.environment.browser = browser.version();
  const page = await browser.newPage({ viewport: { width: 1120, height: 740 } });
  page.on('pageerror', error => report.errors.push(error.message));
  for (const theme of ['dark', 'light']) {
    for (const view of ['home', 'queue', 'history', 'settings', 'narrow', 'quick']) {
      const quick = view === 'quick';
      await page.setViewportSize(quick ? { width: 632, height: 260 } : view === 'narrow' ? { width: 760, height: 640 } : { width: 1120, height: 740 });
      await page.goto(`${baseUrl}/${quick ? '?window=quick' : ''}`);
      if (quick) await page.locator('.quick-shell').waitFor();
      else await page.getByRole('heading', { name: /^下载工作台/ }).waitFor();
      if (['queue', 'history', 'settings'].includes(view)) await page.getByRole('button', { name: view === 'queue' ? /^下载队列/ : view === 'history' ? '下载历史' : '设置', exact: view !== 'queue' }).click();
      await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
      await page.addStyleTag({ content: freeze });
      await page.evaluate(() => document.fonts.ready);
      const currentPath = path.join(directory, `${view}-${theme}-current.png`);
      const baselinePath = path.join(directory, `${view}-${theme}-baseline.png`);
      const current = await page.screenshot({ path: currentPath, animations: 'disabled', caret: 'hide' });
      await page.evaluate(({ css, freeze }) => {
        window.__FLUXDL_STYLE_CHECK_SHEETS__ = Array.from(document.styleSheets);
        window.__FLUXDL_STYLE_CHECK_SHEETS__.forEach(sheet => { sheet.disabled = true; });
        const style = document.createElement('style');
        style.id = 'fluxdl-style-check-baseline';
        style.textContent = css + freeze;
        document.head.append(style);
      }, { css, freeze });
      const baseline = await page.screenshot({ path: baselinePath, animations: 'disabled', caret: 'hide' });
      const horizontalOverflow = await page.evaluate(() => document.body.scrollWidth - innerWidth);
      report.checks.push({ view, theme, pixelIdentical: current.equals(baseline), horizontalOverflow, currentScreenshot: relative(currentPath), baselineScreenshot: relative(baselinePath) });
      await page.evaluate(() => {
        document.querySelector('#fluxdl-style-check-baseline').remove();
        window.__FLUXDL_STYLE_CHECK_SHEETS__.forEach(sheet => { sheet.disabled = false; });
        delete window.__FLUXDL_STYLE_CHECK_SHEETS__;
      });
    }
  }
} catch (error) {
  report.errors.push(error.stack || String(error));
} finally {
  if (browser) await browser.close().catch(error => report.errors.push(`Browser cleanup: ${String(error)}`));
  if (server) await server.close().catch(error => report.errors.push(`Server cleanup: ${String(error)}`));
  fs.writeFileSync(path.join(directory, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
}
const passed = report.checks.filter(check => check.pixelIdentical && check.horizontalOverflow === 0).length;
console.log(JSON.stringify({ report: relative(path.join(directory, 'report.json')), passed, expected: 12, errors: report.errors }, null, 2));
if (passed !== 12 || report.errors.length) process.exitCode = 1;
