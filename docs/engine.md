# 本地下载引擎

FluxDL 的下载权威状态位于 Rust `Manager`，界面只展示快照与事件。命令均通过可执行文件加参数数组启动，URL 放在 `--` 之后，使用 `--ignore-config`，关闭第三方插件目录与远程 EJS 组件下载；不读取浏览器 Cookies。

## 发现与配置

yt-dlp 的检测顺序是有效的用户路径 → 安装资源目录中的 `binaries/yt-dlp.exe` 或 `yt-dlp.exe` → PATH。前一候选失效时会保留告警并继续检测。当前交付**没有内置 yt-dlp 或 FFmpeg 二进制**，不会静默下载引擎。路径选择后，后端运行版本命令确认可执行性，每次版本检测最多 8 秒。

FFmpeg 与 FFprobe 优先查用户指定目录（也接受 ffmpeg.exe 路径，再找同目录的 ffprobe.exe），否则检测 PATH。设置页展示实际使用的路径、来源、版本及告警。这里检测的是程序，不是 Python 的同名包。

## 解析、格式和最终文件

解析使用 `--dump-single-json --skip-download --no-playlist --flat-playlist --playlist-end 1`，最多 75 秒，元数据上限 16 MiB，错误输出上限 64 KiB。stdout 和 stderr 同时消费，防止进程管道堵塞。Windows 版本检测、媒体解析和下载均持有 kill-on-close Job Object，退出时清理引擎及其 JS/FFmpeg 子进程；建立隔离失败时保守拒绝执行。列表链接返回明确提示并禁止整列表入队。

视频预设提供最佳、2160p、1440p、1080p、720p、480p，质量数值代表已知分辨率的上限；后端使用实际格式选择器，并偏好 MP4/M4A。缺少分辨率信息的直接媒体格式仍可选（yt-dlp `[height<=?1080]` 语义），其实际分辨率无法提前保证。存在 FFmpeg 和 FFprobe 时允许视频与音轨分离下载后合并，容器优先 MP4，必要时 MKV。也可以选择实际返回的明确格式。部分站点或直接媒体链接没有时长、尺寸或封面，界面按未知信息展示。

缺少完整 FFmpeg / FFprobe 时，视频仅允许同时含音视频的直接格式；没有这种格式就提前提示配置依赖。仅音频模式固定提取为 MP3，因此需要 FFmpeg 和 FFprobe。MVP 不提供原始音轨直存模式。格式不可用时应重新解析；应用不以假进度替代失败。

下载通过带专用标记的 JSON `--progress-template` 报告实际字节数、总大小、速度与 ETA，未知大小保持不确定进度。分离音视频的进度是当前下载流的进度，不承诺它代表合并文件的整体百分比。后处理独立展示。最终路径来自 `--print after_move:...%(filepath)j`，成功退出后检查文件真实存在且位于任务保存目录中，才标为完成。

0.2将有界progress字段和format身份构造为一个对象，再一次性`j`序列化；不能逐字段拼JSON，因为yt-dlp对缺失值会先输出裸NA。parser兼容新dotted keys与旧flat记录，缺失total/ETA保持unknown，空/无关对象拒绝；不对整行做NA文本替换。每路输出保留序号、运行attempt及已退出stream护栏，新流可真实reset。真实2026.07.04快慢下载29条记录和最终15项桌面回归已通过，证据见[0.2验证](engineering/0.2.0-validation-report.md)。

Generic 解析器可能接受任意文件链接，FluxDL 在元数据层只接受明确的音视频容器扩展名白名单；程序、脚本、快捷方式、网页和普通文档不会作为媒体格式入队。文件打开入口还会再次检查扩展名与真实保存路径。

文件名采用 yt-dlp 的 Windows 安全文件名规则、受限标题与媒体 ID 长度，并附任务 UUID 避免不同任务覆盖。取消可能留下 `.part` 或合并中的临时文件；重试复用任务输出模板，由 yt-dlp 自己处理已下载分片，界面不会声称实现了暂停。

## YouTube EJS 与 JavaScript

依次检测 yt-dlp 所在目录和 PATH 中的 Deno、Node：Deno 最低 2.3.0，Node 最低 22，检测通过后显式传入 `--js-runtimes`。版本存在不代表所有站点均可解析，也不能单凭版本检测断言 EJS 完整可用。EJS/运行时相关解析告警与错误保留在诊断中并提示官方指引。

官方打包的 yt-dlp 可执行文件通常自带匹配的 EJS 脚本；其他安装形态应自行核对。FluxDL 不启用 `--remote-components`，不会自行安装 Deno、Node、EJS 或更新引擎。

官方参考：[yt-dlp CLI](https://github.com/yt-dlp/yt-dlp)、[EJS 安装指引](https://github.com/yt-dlp/yt-dlp/wiki/EJS)、[FFmpeg 许可](https://ffmpeg.org/legal.html)。

## 真实后端联调

下列公网文件数字保留0.1 MVP当时的实测，未在0.2优化中重复请求。0.2使用本地原创HTTP素材的真实视频/MP3、双窗口及进程回归，当前结果见工程验证报告。

从项目根目录运行（需要本机真实 yt-dlp、FFmpeg 与网络）：

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --example engine_smoke -- "E:\VedioDownload\yt-dlp.exe" "https://download.blender.org/peach/trailer/trailer_400p.ogg" "E:\Software Development\FluxDL\artifacts"
```

路径仅为本次验证示例，应用本身不硬编码此路径。示例通过与桌面相同的 `Manager` 解析、顺序下载两个真实任务，检查文件、进度、设置与历史的重启保留情况；生成 `artifacts/engine-smoke-report.json`。Blender 的 OGG 链接被 yt-dlp Generic 标记为音频格式，因此默认选择 MP3；可传入自己的授权媒体 URL，末尾可指定 `video` 或 `audio`。

本次 Windows 主机已实际通过以下链路：

- 音频：上述 OGG → 两个 MP3，各 919,772 字节，FFprobe 确认 MP3 音轨、32.997 秒。总联调 28.75 秒。
- 视频：`https://download.blender.org/peach/trailer/trailer_iphone.m4v` → 两个 M4V，各 3,889,885 字节，真实进度与顺序调度通过；总联调 26.38 秒，报告在 `artifacts/video-e2e/engine-smoke-report.json`。
- 两次均验证同时运行任务最多 1 个，输出文件存在，重启后的设置和完成记录保留。第一次视频解析曾遇到站点 TLS EOF，应用返回了网络错误；重新运行后通过，没有关闭证书校验。

主应用、示例和测试可执行文件统一通过链接器嵌入 `src-tauri/windows.manifest` 的 Common-Controls v6 清单，避免 Windows 在加载 Tauri 对话框依赖时出现 `0xc0000139`。构建脚本使用 Tauri 官方 `WindowsAttributes::new_without_app_manifest()` 保留图标与版本资源，同时避免资源库与链接器重复嵌入同一清单。

统一清单还声明普通用户权限 `asInvoker`、Windows 10/11 兼容、PerMonitorV2 DPI 与长路径感知；设置依据[微软应用清单规范](https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests)和[DPI 清单指引](https://learn.microsoft.com/en-us/windows/win32/hidpi/setting-the-default-dpi-awareness-for-a-process)。
