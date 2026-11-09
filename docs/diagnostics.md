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

## 0.2.1：等待、可信预览与代码版本

任务详情进入「性能诊断」会优先定位当前会话尝试，可查看关联解析或上次尝试并「复制本任务诊断摘要」。准备阶段显示实际引擎节点和已等待秒数；30秒提示是提醒，不表示故障。首次管线输出、下载前标记和首次有效协议进度都不是网络首字节；未知/旧会话未闭合不补耗时。

首次预览由Rust缓存180秒、最多128条，命中时省去入队重复提取，路径/格式/引擎仍验证；无引用、过期、变配置或不匹配则完整重新解析。私有引用不持久化/不导出。诊断概览显示缓存决策及实际重提取次数，统计按会话/环境/策略分开；筛选/保留窗口可能只保留部分阶段。

新报告含编译revision/dirty及会话引擎环境；旧记录缺失就显示unknown，不用当前版本回填。Release不运行Git，不提供A/B开关。开发对照复现：`node scripts/diagnostics-smoke.mjs '<本地yt-dlp.exe>' 20 --latency-ab`，程序须先构建debug。实际测试会在finally关闭本轮新开的自有Explorer窗口，保留原有窗口。

[0.2.1验收](engineering/0.2.1-validation-report.md)及[性能复测](engineering/0.2.1-performance-after.md)是当前范围；此前诊断验收保留0.2历史。
