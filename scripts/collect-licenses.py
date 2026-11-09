"""Collect exact installed dependency license texts without installing any tools."""
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent
DEST = ROOT / 'licenses'
DEST.mkdir(exist_ok=True)
rows = []

def collect(kind, name, version, license_name, directory):
    target = DEST / kind / (name.replace('/', '_').replace('@', '') + '-' + version)
    texts = []
    for item in sorted(directory.iterdir()):
        if item.is_file() and item.name.lower().startswith(('license', 'licence', 'copying', 'notice', 'copyright')):
            target.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(item, target / item.name)
            texts.append(str((target / item.name).relative_to(ROOT)).replace('\\', '/'))
    rows.append((kind, name, version, license_name or 'SEE UPSTREAM', texts))

lock = json.loads((ROOT / 'package-lock.json').read_text(encoding='utf-8'))
for location, entry in lock['packages'].items():
    if not location or entry.get('dev') or entry.get('optional'):
        continue
    folder = ROOT / location
    pkg = json.loads((folder / 'package.json').read_text(encoding='utf-8'))
    collect('npm', pkg['name'], pkg['version'], pkg.get('license', ''), folder)

metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc', '--manifest-path', str(ROOT / 'src-tauri/Cargo.toml')], cwd=ROOT))
reachable = set()
nodes = {node['id']: node for node in metadata['resolve']['nodes']}
pending = [metadata['resolve']['root']]
while pending:
    node_id = pending.pop()
    if node_id in reachable:
        continue
    reachable.add(node_id)
    pending.extend(nodes[node_id]['dependencies'])
for pkg in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
    if pkg['id'] in reachable and pkg['name'] != 'fluxdl':
        collect('cargo', pkg['name'], pkg['version'], pkg.get('license', ''), Path(pkg['manifest_path']).parent)

report = ['# Dependency license inventory', '', 'Generated from package-lock.json and Cargo.lock for Windows x64. Includes build dependencies. Original notices are preserved in the adjacent directories. No yt-dlp or FFmpeg binary is distributed.', '', '| Ecosystem | Package | Version | License expression | Texts |', '|---|---|---|---|---|']
for kind, name, version, expression, texts in rows:
    links = ', '.join(f'[{Path(p).name}]({p.removeprefix("licenses/")})' for p in texts) or 'License expression in package metadata; consult upstream crate source'
    report.append(f'| {kind} | {name} | {version} | {expression} | {links} |')
(DEST / 'README.md').write_text('\n'.join(report) + '\n', encoding='utf-8')
print(f'Collected {len(rows)} dependency entries into {DEST}')
