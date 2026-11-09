# 开发与正式产物流水线验收（2026-11-09）

范围为 `prompt/FluxDL_Codex_Release_Workflow_Optimization.md` 阶段A–E；Windows11 x64、Node24.16.0/npm12.0.1、Rust1.97 MSVC、Tauri CLI2.12.4。起点main/699c75e，初始仅用户提供的新任务书；未修改src/、src-tauri/产品源码、依赖锁文件、UI、引擎、任务状态或IPC。Vite仅新增构建/缓存目录监听排除项。

## 最终实际产物与来源

默认位置均从脚本所在仓库根目录定位，不硬编码盘符。当前工作区的真实结果：

```text
<repo>/release-output/
├── FluxDL/
│   ├── FluxDL.exe                         10143744 B
│   ├── FluxDL_0.2.0_x64-setup.exe           3008448 B
│   ├── LICENSE
│   ├── THIRD_PARTY_NOTICES.md
│   ├── licenses/                          482个文件
│   ├── 使用说明.txt
│   ├── build-info.json
│   └── SHA256SUMS.txt                      488条
└── FluxDL.zip                              7918744 B
```

最终目录489个常规文件，ZIP489项、顶层FluxDL/；全部文件哈希及CRC验证通过。build-info实际来源为干净代码提交 `bd5449c57259a9ce53ee2ec282d7577ef801d36e`，gitDirty=false，构建开始UTC `2026-11-09T08:47:33.775Z`、结束 `08:49:15.584Z`，windows-x64/release、外部引擎。最后重复整理只变更发布ID；不重编译，不把后续文档提交改写为二进制来源。

程序SHA256：`8de6a4ca184beaa8ab9102549b4fb30c4f86e19fcbddd1d719b42c5472d24144`。
NSIS SHA256：`eac9584c493464d2f7e2e43db1ce437d1c734b164aa96d80d6f8d29fe2ddcd95`。
ZIP的最终指纹和全部结构化结果见[验收JSON](release-workflow-validation.json)，不复用上一轮0.2优化的旧安装包指纹。

## 实际执行与结果

| 检查 | 实际结果 |
| --- | --- |
| `npm run release:build` | 最终代码完整执行成功；包含以下各项及真实构建/发布 |
| typecheck / ESLint / Prettier | 全部通过，新发布脚本也纳入lint/格式检查 |
| `npm test` | 90通过，1项性能基准按默认配置跳过 |
| `npm run test:release` | 34通过、0失败、0跳过；独占系统临时fixture |
| cargo check / fmt / 严格Clippy | 全部通过 |
| `npm run test:rust` | 80通过、1项按设计ignored，main/doc tests无失败 |
| 显式 `scripts/test-queue.ps1` | 1通过，5.22秒；未把ignored当作已运行 |
| 许可证收集 | 277个Windows依赖条目，原许可证/依赖锁未产生差异 |
| 前端生产构建 | JS287.71kB、CSS34.28kB；与原产品资源一致 |
| Tauri Release / NSIS | 真实生成x64程序/安装器，最终Rust Release步骤1m35s |
| `npm run release:verify` | 独立执行通过；再次验证前后所有正式文件内容哈希不变 |
| Python zipfile | 独立testzip CRC检查、全量解压489项，与目录逐文件SHA256完全一致 |
| Windows Expand-Archive/.NET | 解压到另一个新建隔离目录，489个文件SHA256全部匹配 |
| 再次 `release:stage` | 实际成功，不重编译；二进制哈希不变、当前安装器唯一、无临时事务积累 |
| `release:clean` | 实际dry-run候选为空、删除0B，发布锁不存在 |
| `release:clean -- --legacy` | 实际识别旧488文件/13427205B；因旧程序仍运行而不具删除资格 |
| Git忽略 | check-ignore确认正式EXE、ZIP和临时区全部忽略；发布模块源码确实被跟踪 |
| Authenticode | 程序和NSIS均NotSigned；未运行安装/卸载向导 |

MSVC链接器会输出“正在创建库”的信息性linker warning；严格Clippy实际通过，没有隐藏编译错误。测试注入故障时出现的“不能写诊断继续回滚”和“未知文件保留备份”是对应安全路径的预期诊断，相关断言均通过。

## 开发模式与用户实例保护

实际运行 `npm run desktop:dev -- --no-watch --config '{"build":{"beforeDevCommand":""}}'`，复用用户原本已经监听1420的本项目Vite，不重复占端口。仅本次debug子进程设置隔离数据/WebView profile，真实工作台渲染、IPC返回空的独立任务库、创建自己的SQLite，request_exit正常退出，CLI退出码0；观察中没有JS异常。正式目录和ZIP在该启动前后逐文件哈希完全不变。

这证明实际Tauri开发启动与现有Vite协作；测试覆盖的是上述复用服务器命令，没有假称另起第二个默认端口实例。默认desktop:dev/dev命令和前端构建语义保留，生命周期钩子未添加发布操作。原Vite PID/启动时间及用户0.1 release应用PID/启动时间均保持，旧应用始终Responding。未打开用户数据库、下载目录，也未停止用户程序或其他项目进程。

## 真实失败与回滚证据

首轮完整构建曾成功生成真实EXE/NSIS、候选目录与ZIP，但提升目录时EPERM，安全失败且无半份正式目录。独立重试同样失败；纯文本目录可移动，完整候选同父目录重命名仍失败。给Vite加入发布/缓存监听排除项后，同一个候选立即可移动，Vite原PID仍服务HTTP200。受控清理该自有诊断候选后重新完整构建，正式提升/替换成功。故障原始记录留在忽略的本机临时区，未把最初成功编译误报为发布成功。

34项fixture中的失败矩阵实际覆盖：

- 缺EXE/安装器/许可证、版本冲突、零字节/伪PE、过期源码/成功收据和输出被替换：均失败，旧目录与ZIP逐文件不变。
- ZIP创建失败、篡改、额外旧安装器、元数据字段错误、空许可证目录：拒绝发布或只读验收失败，不修复/覆盖旧版。
- 备份后/提升后故障：正常回滚，旧目录与ZIP内容完全恢复，并留单份故障记录。
- 替换和回滚同时失败：保留previous-output，错误包含绝对恢复路径；默认clean不删，显式recover验证后恢复。
- 已提升且回滚再次受阻：当前候选和旧备份均保留，显式恢复整体恢复旧版、不合并复制。
- 损坏journal、残留stage、死锁及真实独立进程活锁：分别保守恢复/显式清理/互斥；未知所有权文件不删。
- Windows真实FileShare占用：持有进程未被结束，流程安全失败保留旧版或完成可验证替换。
- 暂存期间源码变动、正式目录新增未知文件、备份新增未知文件、诊断路径变junction：不发布过期输入、不删除未知文件、不因诊断失败跳过回滚。
- Unicode、空格、括号、嵌套路径、2MiB文件、ZIP32单文件边界及junction防越界均实际验证；无SRTTool访问或清理。

## 最终清单与边界

- [x] 项目规则、工程记忆、Git与原构建/输出审计完成。
- [x] dev不发布，desktop:build仅Tauri输出，release:build完整统一入口。
- [x] 复用原stage入口；正式目录和ZIP真实生成、只保留当前一对。
- [x] ZIP顶层/解压内容、版本/Commit/dirty/UTC/架构、全部SHA256和CRC独立验收。
- [x] 重复/不同版本fixture及实际重复发布无历史堆积。
- [x] 构建/输入/ZIP失败保护旧版，替换失败回滚，回滚失败保备份/给恢复路径。
- [x] 并发锁、残留诊断/恢复、只读verify、dry-run clean均已实现并测试。
- [x] 保留npm/Rust缓存；旧release显式白名单策略，运行中实例与未知文件保留。
- [x] 路径/链接/所有权检查，不访问其他项目、用户下载或AppData数据库。
- [x] 正式/临时产物忽略Git；脚本、测试、配置及文档纳入源码范围。
- [x] 外部引擎、WebView2、AppData和安装器/原始EXE的说明准确。
- [x] 自动化检查、真实Windows构建、独立解压和开发启动均有实际证据。
- [x] README、构建说明及工程记忆同步；Git遵循持续授权，最终提交推送/SHA由本次交付结果确认。

没有更改本轮要求以外的产品行为。剩余验证边界：发行EXE没有在真实用户AppData中启动，未执行NSIS完整安装卸载，未独立测试Windows10、签名分发、所有物理DPI/多屏或外部站点。debug隔离启动不能代替发行EXE直接运行；本机结果也不代表远程Actions已通过。原始EXE依赖WebView2及用户配置的外部引擎，不是无依赖便携版。
