# Third-party notices

FluxDL is MIT licensed. Exact JavaScript versions are locked in `package-lock.json`; Rust dependencies are locked in `src-tauri/Cargo.lock`. The generated `licenses/README.md` records dependency SPDX expressions and links to original license/copyright notices. These files are included in the Windows installation resources. Regenerate with `python scripts/collect-licenses.py` after changing dependencies.

The application uses React, Vite, TypeScript, Lucide, Zustand, Tauri 2, Tokio, Serde, rusqlite/SQLite and their dependencies. Lucide icons originate from the Lucide project (ISC); its license and attribution are in `licenses/npm/`. The FluxDL download mark is original code-drawn artwork (MIT).

This release does **not** include yt-dlp, FFmpeg, FFprobe, Deno, Node or any media file. It invokes executables already installed by the user. No executable is silently installed or updated by FluxDL. Their licenses are separate from FluxDL:

- yt-dlp: https://github.com/yt-dlp/yt-dlp/blob/master/LICENSE (source and standalone distribution components can have different terms; inspect the particular distribution).
- FFmpeg/FFprobe: https://ffmpeg.org/legal.html (license depends on build options; GPL-enabled builds are not LGPL-only).
- Deno: https://github.com/denoland/deno/blob/main/LICENSE.md
- Microsoft Edge WebView2: Microsoft runtime distribution terms apply. The NSIS installer offers the official runtime bootstrapper if missing; this is explicit installer behavior, not an engine download.

Public integration test source: Blender Foundation Big Buck Bunny trailer, https://download.blender.org/peach/trailer/trailer_400p.ogg. Film project licensing and attribution: https://peach.blender.org/about/ and https://peach.blender.org/about/#freedom (Creative Commons Attribution 3.0). Downloaded test media is excluded from source control and installation packages. Deterministic local smoke tests use an original FFmpeg-generated test pattern/sine wave; these are also excluded from packages.
