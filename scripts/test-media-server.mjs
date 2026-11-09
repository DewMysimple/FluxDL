// Original, locally generated media only. Used by the optional desktop smoke test.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export function startMediaServer() {
  const directory = path.join(root, 'artifacts', 'test-media');
  fs.mkdirSync(directory, { recursive: true });
  const media = path.join(directory, 'fluxdl-original-pattern.mp4');
  if (!fs.existsSync(media)) {
    const result = spawnSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=640x360:rate=24', '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=44100', '-t', '20', '-c:v', 'libx264', '-preset', 'ultrafast', '-crf', '25', '-c:a', 'aac', '-movflags', '+faststart', '-y', media], { windowsHide: true, encoding: 'utf8' });
    if (result.status !== 0) throw new Error(`FFmpeg fixture generation failed: ${result.stderr}`);
  }
  const content = fs.readFileSync(media);
  let unavailable = false;
  const server = http.createServer((req, res) => {
    if (unavailable || req.url?.includes('/missing')) { res.writeHead(404).end('Missing test fixture'); return; }
    const slow = req.url?.includes('slow');
    const chunkSize = req.url?.includes('native-slow') ? 4096 : 16384;
    const range = /bytes=(\d+)-(\d*)/.exec(req.headers.range || '');
    let start = range ? Number(range[1]) : 0;
    const end = range?.[2] ? Math.min(Number(range[2]), content.length - 1) : content.length - 1;
    const headers = { 'Content-Type': 'video/mp4', 'Accept-Ranges': 'bytes', 'Content-Length': end - start + 1 };
    if (range) headers['Content-Range'] = `bytes ${start}-${end}/${content.length}`;
    res.writeHead(range ? 206 : 200, headers);
    if (req.method === 'HEAD') { res.end(); return; }
    if (!slow) { res.end(content.subarray(start, end + 1)); return; }
    const interval = setInterval(() => {
      const next = Math.min(start + chunkSize, end + 1);
      res.write(content.subarray(start, next));
      start = next;
      if (start > end) { clearInterval(interval); res.end(); }
    }, 80);
    res.on('close', () => clearInterval(interval));
  });
  return new Promise(resolve => server.listen(0, '127.0.0.1', () => resolve({ server, port: server.address().port, media, setUnavailable: value => { unavailable = value; } })));
}
