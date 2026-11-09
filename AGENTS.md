# FluxDL 项目协作规则

## 开发与验证

- 沟通、工程说明和交付使用简体中文。产品需求以 `prompt/FluxDL_Codex_MVP_Prompt.md` 为基线；实现以当前源码和配置为准。
- Windows-first，React / TypeScript / Vite + Tauri 2 / Rust。真实下载、队列、进度与持久化由 Rust 管理；浏览器只作界面预览。
- 修改前检查 `git status --short --branch`、当前 HEAD 与实际远程分支，保留用户和其他任务的改动。
- 按修改范围执行必要检查：前端 `npm run typecheck`、`npm run lint`、`npm run format:check`、`npm test`；后端 `npm run check:rust`、`npm run format:rust`、`npm run lint:rust`、`npm run test:rust`；队列生命周期使用 `./scripts/test-queue.ps1`。仅改记忆或文档时检查相关引用和记忆，不为记忆重跑业务测试。当前0.2.1验证见 `docs/engineering/0.2.1-validation-report.md`；`diagnostics-validation.md`、`0.2.0-validation-report.md`与`docs/validation.md`保留历史范围。

## Git 交付：持续授权

用户于 2026-11-09 再次明确要求：以后每次完整对话产生的修改均提交并推送到 `https://github.com/DewMysimple/FluxDL.git`。代码、配置、文档和工程记忆修改都适用；该要求持续有效，直至用户明确修改、暂停或撤销。

0.2.0任务书第2.2、14.H节曾要求该轮不自动commit/push、不创建Release；该历史执行当时保留了本地修改。用户随后重申本条持续约束，已将这批成果补交并推送为 `876718aec554670972b417afe96cee4df6ad9846`。历史任务中的局部例外不自动延续，也不能覆盖用户之后明确给出的要求。

- 远程名 `origin` 指向上述仓库；交付前核对实际 URL 和 upstream。当前主分支为 `main`，其他任务分支遵循其实际 upstream，不悄悄推到其他仓库。
- 完成修改后，先同步有持久价值的工程记忆、执行必要检查、审查差异，再提交本次已授权修改并推送。该范围内的提交和推送已有授权，无需重复确认。
- 纯只读对话或没有文件变化时不制造空提交。暂存前检查变更归属，不将用户无关改动或其他任务成果混入本次提交。
- 依赖、构建目录、安装包、测试媒体、用户数据库、凭证和本机临时数据不纳入提交；遵循 `.gitignore`。最新本地交付固定为 `release-output/FluxDL/` 和 `release-output/FluxDL.zip`，或通过CI artifacts交付。
- 不使用强制推送、重写远程历史或丢弃用户改动来完成交付。远程领先时先获取并保全、整合相关变更；推送失败时保留本地提交并如实报告原因及恢复位置。
- 用户要求命令/测试打开的资源管理器窗口用完即关闭；只关闭本轮新开的自有窗口，保留原有或无关窗口，不结束Explorer进程。
- 推送后核对远程分支 SHA 与本地 HEAD 一致，并在最终答复中说明提交与推送结果。验证报告只声称实际执行和覆盖的范围。

## 构建与本地产物

- 日常dev/test和前端build不发布产物；desktop:build仅更新Tauri输出和成功构建来源收据。完整正式流程用 `npm run release:build`，复用已确认构建用 `npm run release:stage`，独立只读验收用 `npm run release:verify`。
- 遵循 `docs/build-and-release.md`：同卷暂存、完整SHA256/ZIP CRC与内容校验、发布锁、整体替换、失败回滚；旧备份未经核实不可删除。禁止自动终止用户FluxDL实例。
- `release:clean`默认仅预览；显式apply仍只处理经所有权/路径/链接校验的对象。保留node_modules和Rust增量缓存；旧release仅显式legacy白名单清理，未知文件或运行中程序保留。绝不触碰用户AppData、下载目录或其他项目。

<!-- wiki-memory:start -->
## 工程记忆

本项目使用工程根目录下的 `wiki_memory/`。新会话读取 `wiki_memory/AGENTS.md` 和 `wiki_memory/README.md`。按协议区分只读窄查询与修改、接续等任务，后者先读取规定的当前状态与全局约束。已读且未变的内容可复用。
在已授权开发任务完成或中断时，按协议同步有证据的状态、验证和恢复下一步；只读任务遵守只读范围。
日常读取、同步和交接直接遵循项目协议，不调用 wiki-memory Skill。仅用户明确要求配置、迁移、体检等记忆管理时加载 Skill；检查频率按项目协议，不把完整验收用于每轮收尾。
<!-- wiki-memory:end -->

## 本地诊断

- 诊断统一由`src-tauri/src/diagnostics/`记录；只允许类型白名单事件，禁止原始URL/标题/路径/Cookie/命令行/stderr进入新日志、报告或ZIP。新引擎技术日志使用固定安全摘要。
- Debug默认工程根`dev-logs/`；Release只能用Tauri app_log_dir，不回退源码/发行目录。日志、诊断ZIP及测试证据禁止提交；capabilities不扩大通用文件权限。
- 耗时使用Rust Instant / 前端performance.now各自计算，解析/入队分Trace，用户选择时间不计入内部耗时。保留二次安全复核、attempt准入及durable ack后终态。
- 相关改动执行`npm run test:diagnostics`；重复基准`benchmark:diagnostics`，真实桌面`node scripts/diagnostics-smoke.mjs <本地yt-dlp.exe>`。仅管理自有隔离debug实例，不停止用户发行版。结果及边界见`docs/engineering/diagnostics-validation.md`。

## 下载准备与可信预览

- 二次安全复核必须保留；可用Rust内存持有的可信预览替代重复联网提取，默认180秒/128条/单条256KiB序列化上限。前端MediaInfo与引用本身不授权格式；URL/模式/格式/目录/参数/引擎配置代次均复核，无效引用完整重提取或安全拒绝。引用不入任务SQLite、日志、报告或ZIP。
- 不改变下载进程仍按原URL提取的语义；只有离线帮助已确认支持才加before_dl标记。首输出/标记/首协议进度使用实际Instant，不解释为DNS完成或媒体首字节；缺失阶段保持未知。
- 编译期注入revision/dirty，运行时不访问Git或源码。历史按session/environment/cache策略分别统计，旧版本未知就保持unknown，不用最新探测版本回填。
- A/B只在debug/测试，正式版不提供测试策略开关。复用test:diagnostics与benchmark:diagnostics；真实对照命令为诊断smoke追加20 --latency-ab，逐样本只归档净化数字。Scope/所有权核验通过后才可操作自有原生菜单，不把UIA超时标成功。
