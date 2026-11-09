# 本地诊断中心

在Workspace侧栏底部进入「诊断中心」。概览查看当前会话计数和慢步骤；性能查看真实Trace瀑布、p50/p95与样本数；事件/异常支持搜索、时间、任务、Trace、等级与来源筛选、分页、复制和关联任务定位。任务详情的「查看任务诊断」及Quick Capture加入任务后的同名入口可直接定位。

开发版：从任何工作目录启动debug桌面应用，默认日志都在工程根`dev-logs/`。本机示例为`E:\Software Development\FluxDL\dev-logs\latest-report.md`与`sessions\*.jsonl`。可在诊断中心「实际日志目录」查看位置，并点「打开日志目录」。仅debug可显式设置FLUXDL_DIAGNOSTICS_DIR；正式版忽略此变量。

正式版：位置由Tauri 2 `app_log_dir()`解析。Windows通常是`%LOCALAPPDATA%\io.fluxdl.desktop\logs\`，以软件显示的实际路径为准；不会写EXE旁、源码、安装目录或release-output。换包/升级不清理用户日志。默认info/warn/error阶段白名单，没有可启用的原始stderr或高频trace模式。

报告会在运行中约2秒随事件刷新，启动、正常退出和「刷新报告」也可更新。损坏尾行、未知schema、查询截断、写入受限都会提示。磁盘问题不会阻止媒体下载；实时内存环只有最近2000条（另有最多20,000条、5秒TTL的历史查询缓存），断电或永久不可写可能失去未落盘事件，不能把未知耗时当0。最多保留5个10MiB会话文件和7天历史，只清理软件已知日志名，未知文件及主动导出不删除。查询分析窗口最多20,000条，过大的历史需缩小时间范围并注意截断标记。

点「导出诊断包」，先核对六个文件及隐私说明，再选择本地ZIP文件名；取消不会导出。为防误覆盖，已有同名文件会拒绝，请选择新名称。可点「定位导出文件」打开Explorer。ZIP内有summary.md、events.jsonl、errors.jsonl、environment.json、privacy-manifest.json、README.md。仅含阶段枚举、UUID、错误类别、真实耗时、安全版本和配置概要，不包含URL、标题、Cookie、token、文件名、私有路径、原始stderr、命令行或数据库。导出不表示已分享给Codex；请检查后自行决定分享。

在本地Codex中打开FluxDL工程，先读取`dev-logs/latest-report.md`，再按报告中的Trace ID查询`dev-logs/sessions/*.jsonl`，核对阶段耗时和相关Rust/React代码路径；优化前后使用相同测试条件复测。不要直接把本地日志提交GitHub。仅看GitHub的远程Codex不会自动获得本机dev-logs。

可重跑验证：

```powershell
npm run test:diagnostics
npm run benchmark:diagnostics
npx tauri build --debug --no-bundle
node scripts/diagnostics-smoke.mjs 'C:\工具\yt-dlp.exe'
```

前两项使用明确标注的确定性fixture和生产Manager，输出在ignored artifacts下；不代表互联网网站性能。桌面脚本使用真实用户已有yt-dlp、原创本地HTTP媒体、隔离数据库/profile，只控制自己启动的debug进程；9322被占用时拒绝附着其他实例。导出辅助器仅确认本脚本自己的debug原生保存对话框，目的限制在工程artifacts。常规开发仍用`npm run desktop:dev`，浏览器`npm run dev`只显示空状态，不提供伪造诊断样本。

协议、阶段边界和保留策略见[设计](engineering/observability-design.md)，实际测试及平台缺口见[验收](engineering/diagnostics-validation.md)，性能数据见[分析](engineering/performance-analysis.md)。

## 持续下载吞吐与网络专项

本专项分支在原诊断中加入`download_throughput / throughput_sample`，每5秒低频采样并在已确认终态后总结。任务详情和选定Trace共用“下载吞吐诊断”，显示5s/10s窗口、观测均速/字节、仅下载时长、无增长次数/时长与观测间隔。首次续传字节不当成新传输，新流/重试/取消/后处理独立；缺进度或不足窗口显示“未测量”，没有字节增长也可能是进度缺失，不能据此认定断网。单位为KiB/s/MiB/s。

协议来自真实进度的安全枚举，`ytdlp_default`和`inherited`只说明参数配置，实际出站节点不由UI猜测。阶段查询启用`stageView`，在100/200条分页前保留原阶段/异常和各Trace/attempt最新吞吐；普通事件页和导出仍保留全部低频样本，仍受20,000事件分析窗口限制。自动报告增加“下载吞吐诊断”，采样不进入原阶段耗时分位数。

`npm run diagnose:youtube`由本地执行器通过stdin接收授权URL和现有引擎路径，自动发现/验证Clash本地端口、运行预算/进程清理自检、固定格式比较生产Manager与独立CLI及显式本地代理。单轮<=45秒/目标40MiB，总媒体预算<=300MiB；HTTP403/429停止，不切节点/TUN/系统网络，不更新引擎。`preflightOnly`仅解析，不能当吞吐证据。所有safe结果保留当前worktree的ignored artifacts/network-diagnostics，部分媒体和隔离SQLite在每轮后受控清理。

测试入口`npm run test:network`只运行确定性本地网络/预算/进程夹具；不请求YouTube。隔离原生验收可给显式debug进程设置`FLUXDL_TEST_DEBUG_PORT`（1024–65535，默认9322），避开其他实例；Release不启用。实际YouTube记录、统计与限制见[网络调查](engineering/network-speed-investigation.md)。本专项没有新增产品代理设置，默认下载网络行为保持继承环境。
