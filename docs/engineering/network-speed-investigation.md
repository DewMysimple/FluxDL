# YouTube 网络速度专项诊断与验证

2026-11-09，当前临时 worktree / `codex/youtube-network-speed`；起点 `ffa76eb6fafb8d53566caa314fbc17e5aa097a1d`，当时实际 origin/main 同 SHA，工作树干净但 detached，因此只在原 worktree 创建专项分支。没有合并或推送 main，没有重新发布主线发行包。原诊断、SQLite、双窗与下载安全边界保留。

## 最终判定与证据等级

**本轮未稳定重现持续约100 KB/s，不能断言根因。** 最有证据的是下载前存在较长的引擎提取/二次复核/首次进度等待；本次有效下载窗口均速约1.1–1.3 MiB/s，但重复轮次多次未在预算内形成窗口，不能排除间歇性慢速。不能据此宣称已修复网络或证明远端节点/CDN限速。

- 已实测确认：A、B、C 的自有引擎进程均曾建立至本机7897的连接，最大采样计数分别至少2/4/3。默认配置已经经过本地代理入口，不能把“没有--proxy”解释为直连。Mihomo监听、系统代理指向及HTTP nonce隧道/SOCKS5无认证握手交叉验证一致。
- 已实测确认：最后A复核保留了单调计时阶段：首次解析、二次复核、排队、spawn、首个有效进度；下载阶段和等待分开计时，用户选择格式的停留不计入。
- 间接支持：本次没有应用参数限速，生产Manager和独立CLI的有效窗口处于同一数量级；显式代理没有稳定收益证据。因此未增加默认代理设置、分片并发、HTTP chunk、缓存或第三方下载器。
- 尚未测量/未确认：本地代理后的实际节点、匹配规则、物理出口、CDN位置及引擎内部各子等待；CPU/内存/磁盘吞吐没有采样，不能据此排除所有本地负载影响。没有读取控制器密钥/订阅/私密配置，也没有切节点或修改系统网络。

## 环境与代理事实

Windows11 x64（build26200），yt-dlp当前用户指定版本2026.10.26，FFmpeg/FFprobe8.0.1，实际检测Node24.16.0。未安装、更新或替换用户引擎。先前自动找到的另一个旧引擎2026.07.04作为独立辅助记录，不混入当前版本对照；用户仅补充了引擎路径，无需手动跑测试。

只读确认Mihomo拥有127.0.0.1:7897监听；系统代理启用且指向该端口，PAC未配置，当前执行器HTTP_PROXY/HTTPS_PROXY/ALL_PROXY/NO_PROXY均未设置。另一个GUI服务端口未当作代理。TUN候选适配器Up，有相应IPv4默认路由；IPv6默认路由未见。两个公开基础域名DNS有限查询各返回一个fake-IP范围IPv4（28ms/1ms），没有返回IPv6；仅保存类别/计数，不保存地址，不把DNS结果当作节点路由证据。其他VPN冲突未获得足够证据，不能仅凭虚拟网卡推断。

参数审计确认`--ignore-config`不读取yt-dlp配置文件，未设置limit-rate、显式proxy、chunk或高并发。yt-dlp未配置proxy时使用urllib获取代理；Windows urllib可回退系统注册代理，因此环境变量为空也可能使用系统代理。[yt-dlp源实现](https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/yt_dlp/YoutubeDL.py)、[CPython实现](https://github.com/python/cpython/blob/3.13/Lib/urllib/request.py)。这与本次实际loopback TCP观测相符，但显式代理和TUN未必是不同物理出口。

## 实验方法与可比性

授权来源是用户主动提供的YouTube单视频；完整URL只由内存stdin传递，不写入报告/源码/完整命令展示。先执行metadata-only预检查，再固定真实format137：1080p视频轨、HTTPS、非DASH/HLS；生产选择器保持`137+bestaudio`，没有降画质或丢音轨。音轨ID未单独锁定，预算内均为部分下载，不声称文件或合并后处理完成。所有组使用相同现有引擎/运行时/预设、原安全参数与默认缓存状态，不清引擎缓存。

A是真实生产Manager/SQLite/二次复核与下载管线，使用100ms任务快照而不渲染WebView；B是独立CLI进程和独立双管道消费，只复用生产argv构建、协议解析与底层Job安全工具，排除了共享Manager/Process消费路径的假对照；C同B且解析/下载均显式指定已验证的本地HTTP代理。逐项argv等价与HTTP/SOCKS参数校验有测试，完整参数/路径/URL只留进程内存。

先ABC，再CBA，各两轮；之后仅补一次A，验证修正后的截止前阶段证据保留，未再重复整个矩阵。每轮总墙钟约45秒（退出有约0.2–0.4秒清理开销），提取和等待挤占媒体观测时长时保留未测量，不延长截止。真实byte delta计算均速与5s/10s窗口，停滞阈值5秒，仅表示“未观察到字节增长”，不证明断网。A的100ms快照可能遗漏极短流，初始续传字节与首次进度前未知传输不算稳态字节，文件大小另作预算兜底。

## 当前引擎的速度记录

所有速度以B/s计算、用二进制MiB/s展示；每组只有1个有效均速窗口，**样本不足**，不能算因果提升比例、稳定p95或普遍性能。0字节/短窗口是没有测到有效下载速率，不当作0带宽。

| 组 | 尝试数 / 有效均速样本 | 有效均速中位数 | 有效5秒窗口中位数 |
| --- | ---: | ---: | ---: |
| A | 3 / 1 | 1.179 MiB/s | 0.942 MiB/s |
| B | 2 / 1 | 1.102 MiB/s | 0.842 MiB/s |
| C | 2 / 1 | 1.236 MiB/s | 0.330 MiB/s |

A共3次包含最后单独复核；B/C各2次，其中只有一轮有效。完整轮次如下：

| 组 | 观测增量B | 有效时长s | 均速 | 5s窗口中位数 | 最后10s | spawn至首进度s | 停滞次数/观测时长 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| A1 | 0 | 0.000 | 未测量 | 未测量 | 未测量 | 未测量 | 0 / 0.000s |
| B1 | 20678085 | 17.902 | 1.102 MiB/s | 0.842 MiB/s | 0.982 MiB/s | 未测量 | 1 / 9.389s |
| C1 | 14598130 | 11.262 | 1.236 MiB/s | 0.330 MiB/s | 0.992 MiB/s | 未测量 | 1 / 5.171s |
| C2 | 0 | 0.000 | 未测量 | 未测量 | 未测量 | 未测量 | 0 / 0.000s |
| B2 | 63488 | 0.141 | 未测量 | 未测量 | 未测量 | 未测量 | 0 / 0.000s |
| A2 | 0 | 0.000 | 未测量 | 未测量 | 未测量 | 未测量 | 0 / 0.000s |
| A复核 | 24661796 | 19.942 | 1.179 MiB/s | 0.942 MiB/s | 1.272 MiB/s | 12.523 | 0 / 0.000s |

这些当前版本样本都由预算结束，终态为runner_cutoff，清理均passed；没有记录403/429或引擎失败，但被截断的未报告错误仍是未知，不能写成绝对“无任何错误”。早期六轮的部分精确首进度/阶段因只等待final而丢失，明确显示未测量；已改成实时发送已有阶段和首进度，不从管线墙钟补造。最后A复核已实际验证修复。

最后A的阶段为：首次解析5.819s，二次安全复核5.440s，队列3.587ms，下载spawn4.278ms，spawn至首次有效进度12.523s，入队至首次有效进度17.974s。首次解析与入队分Trace、总阶段与子阶段嵌套，不能简单全部相加。实际等待主要在两次引擎调用与spawn后的资源准备，不能仅凭这些跨度继续拆出网络、JS或CDN时间。

旧2026.07.04引擎辅助组：A有效9.229s/均速1.064MiB/s；B有效5.775s/均速1.671MiB/s后出现HTTP403，立即停止该旧引擎整组，没有继续C或反复重试。随后用户给出真实当前引擎，按正确版本完成上述矩阵；不是切客户端、抓Cookie或规避限制。不得把旧/新不同引擎直接合并计算加速。

累计9轮真实部分下载的媒体预算计数80,468,570B（76.74MiB），低于整次300MiB预算；新执行器的总预算逐次扣除前次实际观测/文件大小。单轮目标40MiB，允许有限在途数据；这些不是网卡wire计数，不包括提取、TLS/HTTP开销或无法观察的重传，不能宣称逐字节硬截断。元数据预检查不是吞吐测试。Blender公网控制组未执行（本轮已有授权YouTube与真实字节），本地fixture结果从未替代YouTube速度。

当前格式为普通HTTPS，所以按协议跳过-N4；没有曲线/重复收益证据，不试验http-chunk-size或启用其他默认优化。[yt-dlp官方分片选项](https://github.com/yt-dlp/yt-dlp#download-options)只适用于DASH/HLS分片，不能作为单连接HTTPS加速器。

## 实际代码改动与安全

- diagnostics/throughput.rs、event/mod/writer/report、Manager：复用schema1的可选typed吞吐字段，真实delta/Instant，5s低频样本+确认终态后的总结，33流/128点有界；finished/后处理/取消/旧attempt与退休stream边界独立。原队列、durable ack、SQLite路径及设置模型未变。
- progress模板加入info.protocol，仅映射安全枚举；记录ytdlp_default/inherited配置事实，不冒称TUN或出站节点。报告增加“下载吞吐诊断”，阶段分位数不混入无duration采样。
- TaskThroughput与现有任务详情/性能页：显示5s/10s、观测均速/字节/时长、停滞与间隔，缺失窗口未测量；新stageView查询在分页前保留普通阶段/异常和每trace/attempt最新吞吐，640样本回归证明长任务不淹没原时间线，普通事件页/导出仍全量。
- network-diagnostics/core、只读环境/TCP脚本及独立Rust例子：stdin输入、严格本地无凭据代理、条件/预算校验、现有生产argv、挂起创建后分配Job，截止或父先退出也清理后代。强杀前先保存已发安全阶段，最多128条，不依赖final。
- lib.rs/diagnostics-smoke仅在原显式隔离debug gate增加可选测试端口；默认9322，实测其被其他工作占用时拒绝附着，改用本worktree独立9433。Release无此入口，不扩大capabilities。

未新增产品代理设置或改变Clash/TUN/节点/系统代理/路由/DNS缓存/防火墙/证书，没有安装下载器或更新引擎。A按原生产模型短暂在本次隔离SQLite保存任务URL；每轮结束已删除本轮媒体和隔离数据库，用户AppData/历史/下载目录不写入。新JSONL/Markdown/ZIP及safe报告不含目标URL、标题、签名媒体URL、认证、私有路径或原始stderr。raw stderr/元数据只在进程内消费后丢弃。正常业务下载参数只增加安全protocol进度字段，没有隐形限速/提速开关。

## 自动验证与复现

统一入口npm run diagnose:youtube已由Codex自动实际执行；通过stdin提供授权URL/现有引擎路径，可选formatId、seconds（<=45）、repetitions（1/2）、totalBudgetBytes（<=300MiB）、conditions（A/B/C）、preflightOnly。脚本每次先跑预算/代理/隐私/真实本地传输和后代回收自检，再增量构建本worktree生产harness；无URL只出环境/未授权原因，不擅自选视频。URL不放命令行参数，实际复现可继续交给Codex完成，无需用户拼终端命令。

已执行：typecheck/lint/format:check；前端106通过/1既有benchmark跳过；check:rust、format:rust、严格all-targets Clippy；Rust111通过/1原worker ignored后显式worker通过4.88s；network_bench例子9通过；网络基础15+真实loopback7共22通过；test:diagnostics生产管线5次/组与异常场景、benchmark:diagnostics 20次/组通过。fixture覆盖慢传、断流重试、真实M3U8、未知总量/流切换、时长/字节/总预算、Nan/无效条件、HTTP404/407误判、父先退出/pipe继承后代和主动截断、私密伪值不透传；不把fixture写成互联网结果。

npm run build与真实debug/no-bundle已成功，原生诊断专项23通过/0 JS异常，包含真实WebView对同task/Trace/attempt最终样本逐值核对、报告与ZIP字节一致、HTTP/inherited/不足窗口及权限/双窗/重启/目录定位。stageView最终源码已重新构建并重跑23项原生检查通过，报告位于本机ignored artifacts/diagnostics/desktop-1794233542581/report.json。曾有新测试选择器拼写/嵌套td定位失败，修正测试后通过，失败证据保留ignored。新worktree因Git检出CRLF与既有Prettier LF不符曾format失败，按原规范归一化未改文件行尾后检查通过；新增.gitattributes只固定formatter管理源码的LF，防止后续Windows worktree再次出现该问题，没有产生无关业务内容差异。

本轮未重建或覆盖release-output，未安装/卸载NSIS；Win10/物理DPI/多屏/物理托盘点击、完整公网长传与远程CI未新增验收。自动全链路原生测试仅控制自有debug进程，用户发行实例和Clash原PID/创建时间保留，9322占用者未连接或终止。

## Worktree交付与下一步

仅当前专项功能分支提交推送，不合并main，不force push。源码/测试/脱敏结论/工程记忆可交付，dev-logs/artifacts/所有媒体/SQLite/诊断包被ignore，不上传；主线旧产物及其他worktree不作为本轮输出目录。

P0：已验收范围内无已知遗留。P1：重复有效样本不足、45s预算可能先耗在提取/等待，无法断言持续100KB/s的根因；节点/CDN/本地资源缺少归因证据；诊断保留/20,000分析窗口有界、永久I/O失败不保证零丢失。P2：上述平台人工范围与远程CI未覆盖，后续在出现慢速的实际任务用低频吞吐和Trace直接定位，再决定是否追加同条件实验；本轮不需要用户切节点或重新手工跑测试。
