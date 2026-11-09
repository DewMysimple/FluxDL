import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const version = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8')).version;
const destination = path.join(root, 'release');
const installer = `FluxDL_${version}_x64-setup.exe`;
const inputs = [
  [path.join(root, 'src-tauri', 'target', 'release', 'bundle', 'nsis', installer), installer],
  [path.join(root, 'src-tauri', 'target', 'release', 'fluxdl.exe'), 'FluxDL.exe'],
];
for (const [source] of inputs) if (!fs.existsSync(source)) throw new Error(`Missing build output: ${source}. Run npm run desktop:build first.`);
fs.mkdirSync(destination, { recursive: true });
for (const [source, name] of inputs) fs.copyFileSync(source, path.join(destination, name));
for (const name of ['LICENSE', 'THIRD_PARTY_NOTICES.md']) fs.copyFileSync(path.join(root, name), path.join(destination, name));
fs.cpSync(path.join(root, 'licenses'), path.join(destination, 'licenses'), { recursive: true });
fs.writeFileSync(path.join(destination, '使用说明.txt'), `FluxDL ${version} · Windows x64\r\n\r\n${installer} 是 NSIS 安装包。\r\nFluxDL.exe 是程序二进制，需要系统已有 Edge WebView2。它不包含所有引擎依赖。\r\n\r\n首次运行请在设置中检测或选择本地 yt-dlp.exe；MP3 转换与音视频合并需 FFmpeg + FFprobe。\r\n此版本未捆绑 yt-dlp / FFmpeg，不会静默安装或更新引擎。\r\n关闭主窗口默认进入托盘；真正退出请使用托盘菜单。活动下载时会提示后台继续或停止并退出。\r\n程序未做代码签名。请保留相邻许可证文件。完整说明、测试结果和已知限制见项目 README.md 与 docs/validation.md。\r\n`, 'utf8');
const hashes = inputs.map(([, name]) => `${createHash('sha256').update(fs.readFileSync(path.join(destination, name))).digest('hex')}  ${name}`);
fs.writeFileSync(path.join(destination, 'SHA256SUMS.txt'), hashes.join('\n') + '\n');
console.log(`Staged installer, program, notices and checksums in ${destination}`);
