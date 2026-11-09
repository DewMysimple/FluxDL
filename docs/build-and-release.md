# 开发、正式构建与本地产物管理

## 改造前审计（2026-11-09）

起点为main/699c75e5aaf66d99d366d6be5bc626a66ee95f47，实际origin/main同SHA。工作树仅有用户提供的本轮任务书。读取了根规则、standard工程记忆、README、历史及0.2验证报告、package/lockfile、Tauri/Cargo/bundle配置、现有暂存与许可证/验证脚本、Git忽略和Windows工作流。

- `desktop:dev` 为 `tauri dev`；`build` 只构建前端；`desktop:build` 为 `tauri build --bundles nsis`。
- 原 `release:stage` 直接向 `release/` 复制，没有暂存、ZIP、来源记录、完整文件哈希、发布锁或回滚；旧文件会残留。
- 实际版本源package.json、package-lock.json根记录、Cargo.toml/Cargo.lock根包、tauri.conf.json均为0.2.0。
- Tauri原始输出为 `src-tauri/target/release/fluxdl.exe` 和 `bundle/nsis/FluxDL_0.2.0_x64-setup.exe`；bundle中另有旧0.1安装器。主EXE及NSIS的PE产品名/版本均为FluxDL/0.2.0。
- bundle资源仅为项目LICENSE、THIRD_PARTY_NOTICES.md和licenses；不包含外部下载引擎。
- 旧 `release/` 有488个文件、13427205字节，包含0.1程序/安装器及许可证。该程序仍由用户运行，不能结束、覆盖或直接删除。`release-output/` 尚不存在。
- Node24.16/npm12.0.1、Rust1.97 MSVC和Windows x64工具链可用；Node提供流式zlib和CRC32，无需新增ZIP依赖。

## 实施方案

复用 `stage-release.mjs`，增加共享的发布模块、完整构建、只读验证和显式清理入口。`desktop:build`仍只生成Tauri构建目录中的产物，并增加成功构建来源收据；只有release:build/release:stage发布正式输出。

仓库同卷的 `.fluxdl-release-work/` 持有专用锁、带所有权标记的临时事务、候选目录和短期备份。候选中先生成 `FluxDL/` 与 `FluxDL.zip`，验证全部文件SHA256、ZIP内容及CRC后，再在锁内整体重命名 `release-output`。这不是跨两个对象的完全事务：短时目录名可能不可见，但不会边复制边覆盖。失败恢复旧目录；恢复失败保留备份并报告路径。未知残留与活锁不会被自动删除。

ZIP使用Node内置流式deflate及CRC32，生成UTF-8名称的标准ZIP32；文件/归档/数量超出实现限额时在发布前失败，不尝试静默截断。测试使用独占系统临时fixture，不触碰实际交付或用户数据。

## 日常开发与正式命令

下列磁盘位置只是当前工作区示例；脚本按自身位置自动定位仓库，不硬编码盘符、用户名或当前shell工作目录。

```powershell
cd 'E:\Software Development\FluxDL'
npm run desktop:dev
```

保持已有Tauri/Vite热更新：不改产品、窗口、UI、下载引擎、任务状态或IPC。浏览器预览仍为 `npm run dev`。普通dev/test/build不会触碰release-output，也不会删除node_modules或Rust target。Vite明确不监听release-output、发布暂存区、旧release、artifacts及Rust target，避免缓存扫描和Windows目录监听句柄阻止发布重命名。开发端口1420若已被自己的Vite占用，应复用已有会话或选择独立端口；不要擅自终止已有进程。

| 命令 | 行为 | 更新正式交付 |
| --- | --- | --- |
| `desktop:dev` / `dev` | Tauri热更新 / 浏览器预览 | 否 |
| `build` | TypeScript及Vite前端生产资源 | 否 |
| `desktop:build` | 原Tauri Release+NSIS；另记成功构建来源收据 | 否 |
| `release:build` | 环境/版本检查、工程检查、许可证、Tauri构建、暂存、验证、替换 | 是，全部成功后 |
| `release:stage` | 只复用可核实的已有构建，暂存、验证、替换 | 是，全部成功后 |
| `release:verify` | 只读核实目录/元数据/PE/全部SHA256/ZIP解压内容及CRC | 否 |
| `release:clean` | 默认dry-run列出候选及大小/原因；明确apply才执行 | 默认否 |
| `test:release` | 独占临时项目的发布/失败/回滚/锁/路径测试 | 否 |

完整正式构建：

```powershell
npm run release:build
npm run release:verify
```

需要Windows10/11 x64、Node24+、npm、Rust MSVC/C++工具链、Python（复用许可证脚本）及当前已安装的Tauri CLI。不每轮npm ci，不自动安装新工具。拒绝CARGO_TARGET_DIR/CARGO_BUILD_TARGET/TAURI_CONFIG/TAURI_TARGET_TRIPLE覆盖，避免从另一个目标目录拿历史二进制。

流水线依次运行typecheck、lint、format:check、前端测试、test:release、check:rust、format:rust、严格Clippy、Rust测试、显式queue fixture、collect-licenses.py，然后使用同一个desktop:build实现构建。CI统一调用完整入口，避免再列一套重复检查。

先手工构建再整理：

```powershell
npm run desktop:build
npm run release:stage
npm run release:verify
```

desktop:build仍执行现有 `tauri build --bundles nsis`；包装器仅加环境/版本/互斥和成功收据。没有predev/postbuild等隐式发布钩子。release:stage不自动重编译。直接裸调用Tauri产生的无收据历史文件会被拒绝，应改用上述命令。

## 固定交付契约

成功后，当前工作区中的示例位置为：

```text
E:\Software Development\FluxDL\release-output\
├── FluxDL\
│   ├── FluxDL.exe
│   ├── FluxDL_<当前版本>_x64-setup.exe
│   ├── LICENSE
│   ├── THIRD_PARTY_NOTICES.md
│   ├── licenses\...
│   ├── 使用说明.txt
│   ├── build-info.json
│   └── SHA256SUMS.txt
└── FluxDL.zip
```

只有当前版本的一个安装器；ZIP内部为FluxDL/，不含自身、源码、缓存、数据库、媒体或用户配置。SHA256SUMS以稳定排序的UTF-8相对路径记录所有常规文件（包括build-info），仅排除清单自身；ZIP还会逐项验证清单文件本身。验证同时拒绝遗漏/多余文件、重复/逃逸路径、链接、空目录及额外旧安装器，避免文件清单/ZIP静默漏掉目录结构。

ZIP用Node内置流式压缩，不调用Compress-Archive，不增加npm依赖。明确上限：单文件≤2147483647B，ZIP总大小≤4294967294B，文件数1–65534；超限失败且保留旧版。验证严格接受本流程生成的UTF-8/deflate ZIP32格式，逐项流式解压、限制声明大小、比较CRC32/字节数/SHA256，并比对本地头与中央目录；不把未知第三方ZIP当成本流程产物。使用[Node zlib/CRC32文档](https://nodejs.org/api/zlib.html)中的内置能力；PE机器类型依据[微软PE格式](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format)。NSIS可用32位启动stub承载x64安装包，因此主程序要求AMD64/PE32+，安装器接受经实际版本资源确认的x86或x64 stub，不能仅靠安装器stub判定应用位数。

## 版本、来源与元数据

严格核对package.json、package-lock.json顶层及根包、Tauri配置、Cargo.toml和Cargo.lock根包的版本，拒绝静默分叉。只支持当前项目配置的数值三段版本与NSIS文件名；以后改变资源/打包目标/版本格式，需显式同步契约及测试。

成功收据位于忽略的 `src-tauri/target/release/.fluxdl-build.json`，包含实际构建开始/结束UTC、HEAD、dirty、构建输入指纹及两份EXE哈希。构建开始先失效旧收据和本版本目标NSIS（不删正式包、不清缓存），确保本次失败后不能拿上次的安装器冒充成功。Cargo可正常复用未变化的EXE增量结果。

构建前后HEAD、dirty和相关输入必须稳定。输入指纹涵盖src/public/src-tauri、脚本、package/lock/TS/Vite/index和许可证资源；忽略target等缓存。stage再次确认当前输入和二进制与收据一致，并在复制/ZIP完成后再核对一次。构建输入变动要求重建；若只提交了文档导致HEAD不同但构建输入完全一致，会明确报告并保留实际构建Commit，不把产物改标为当前HEAD。

build-info记录产品、版本、实际短/完整Git Commit、真实gitDirty、构建时间、windows-x64/release、tauri2-nsis-and-raw-exe、external引擎分发方式、源指纹及本次发布ID；不写开发绝对路径、用户名、Token或用户数据。dirty表示构建时的状态；提交之后不能回写成false冒充干净构建。测试收据和交付明确标fixture，生产CLI拒绝它们，也没有fixture参数。

生产校验同时读取MZ/PE签名、COFF机器类型、可执行标记及Windows文件版本资源；任意假二进制即使有相同文件名也不会通过。CLI只从本仓库标准Tauri输出取文件，不接受任意输入根目录。

## 安全替换与异常恢复

所有正式构建/desktop构建/stage/apply清理使用同一专用锁；verify看到锁会失败，不会改锁或抢占。锁记录PID与随机token，存活PID（包括可能的PID复用）保守视为占用。不会终止任何FluxDL进程。

`.fluxdl-release-work/transaction-<UUID>/`有所有权标记和journal；候选目录含完整的一对产物。准备完成后，把旧release-output整体移为该事务的previous-output，再把候选整体重命名为release-output；最终重新核对全套目录与ZIP。成功删除本次事务/短期备份；正常失败恢复旧目录，并只保留单个last-failure.json故障记录，不积累版本归档。

两个重命名不能形成完全事务，短时可能没有正式目录；进程崩溃/掉电也不能承诺文件系统绝对持久。异常残留会阻止下一次发布，避免盲目覆盖唯一可用副本。

```powershell
npm run release:clean                 # 先看候选、绝对路径、大小、原因
npm run release:clean -- --apply      # 只清理可确认无备份风险的残留
npm run release:clean -- --apply --recover
# 仅锁PID已退出、确认无遗留构建子进程后：
npm run release:clean -- --apply --stale-lock
# 同时需要恢复时可组合 --stale-lock --recover
```

含previous-output的残留，默认apply也不删除。显式recover先完整验证旧备份；若当前正式输出属于同一个事务，移到事务内独立位置，然后整体恢复旧版，再验证后清理。正式输出不属于该事务或备份无效时拒绝自动操作。journal损坏时仍根据所有权/实际备份及哈希进行保守恢复，不靠phase猜测删除备份。

回滚失败会输出备份绝对路径、正式路径和上述恢复命令。手工恢复时，先保留并移开现有正式目录，再把该previous-output整体移回release-output，不能合并复制、覆盖或删除唯一备份。未知文件、标记无效、链接、权限不足时保留现场，先人工核实。恢复锁stale-lock-guard.json若因异常退出残留，同样先确认其中PID已退出、无发布/清理任务，再将它移到另一个安全位置留证，重跑预览和显式恢复；绝不在任务存活时移走锁。

Windows占用可能阻止移动/删除：此时失败回滚或保留已验证新输出及短期备份，报告路径；不会杀应用。已验证新包后的备份清理若被占用阻止，后续clean会再次核实当前输出身份/哈希才清理；未处理的事务会阻止下一次发布。

## 清理边界与旧release

默认clean只读预览；显式apply只删除本脚本可证明所有权的临时事务。每个删除路径检查解析后的仓库边界、全部现有祖先、junction/符号链接及特殊文件；未知临时文件或目录保留报错。检查目录内是否夹带未知文件，避免仅靠名字推断所有权。报告候选字节数和实际删除字节数，重复执行安全。

旧release不再写入，也不会由正常发布自动删除。迁移只改变输出位置，保留旧文件。可按需显式预览：

```powershell
npm run release:clean -- --legacy
# 只有预览确认全部可删除，且旧程序已经由用户主动退出后：
npm run release:clean -- --apply --legacy
```

旧目录检查顶层白名单、二进制原哈希、许可证原文与仓库对应文件，以及是否仍有该路径的FluxDL进程。未知文件、无法确认的许可证/哈希或运行中程序均不删。不要为清理强制结束用户实例。当前旧0.1目录因仍运行而保留。

默认保留node_modules、target/debug、target/release及所有Rust增量缓存；未提供自动deep模式。空间不足时先用clean预览/清理脚本所有的临时文件，退出相关开发工具后再人工核对所需缓存范围，按工具维护方式处理；不要把删除缓存纳入每轮发布或运行git clean -fdx/cargo clean。不访问其他项目，不改AppData、用户下载目录、SQLite或设置。

## 运行与分发边界

NSIS是安装器，不是便携版。原始EXE不是完全自包含绿色单文件：需要Windows x64和系统WebView2，设置/SQLite仍在系统AppData，yt-dlp由用户配置，MP3/合并需要FFmpeg和FFprobe，部分站点另需用户自己的JS运行时/EJS。本轮不下载或内置任何媒体引擎。

仅LICENSE/声明/许可证资源按实际Tauri配置分发；没有凭空添加DLL或工具。最新本地包不等于GitHub Release，也不表示签名、完整安装/卸载、Windows10、所有DPI/多屏或外部站点均已验收。为保护正在运行的用户实例及真实AppData，发行EXE直运行必须具备有效隔离条件；不能用debug隔离启动冒充发行版直运行测试。

## 验证记录

自动化命令为 `npm run test:release`。临时fixture覆盖首次/同版本/新版本/重复发布、缺失或过期输入、假PE/fixture隔离、ZIP篡改及只读性、构建失败、替换及回滚失败、异常journal恢复、真实独立进程锁、Windows文件占用、Unicode/空格/嵌套目录和2MiB文件、ZIP32超限、junction防越界、未知文件和旧目录保护；不每次编译Tauri。

阶段A–E已实施。2026-11-09最终实现的完整release:build实际成功：90项前端、34项发布、80项Rust及独立队列fixture通过，真实NSIS生成；独立verify、Python和Windows解压的489文件哈希完全一致，重复stage后仅保留最新一对产物。准确来源、文件哈希、故障回归、开发启动和未测边界见[验收记录](engineering/release-workflow-validation.md)及[脱敏JSON](engineering/release-workflow-validation.json)。本机结果不冒充远程CI通过。
