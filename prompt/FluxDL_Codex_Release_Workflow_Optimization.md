# FluxDL · Codex 开发环境与正式构建产物管理优化任务书

> **任务性质：** 在现有仓库上执行工程改造，而不是仅输出设计建议。  
> **仓库：** https://github.com/DewMysimple/FluxDL  
> **目标平台：** Windows 11 / Windows 10 x64，Tauri 2 + Rust + React + Vite + TypeScript  
> **核心目标：** 开发过程轻量、正式产物始终只保留最新一版、安全替换、验证后压缩、构建失败不破坏上一版。  
> **交付语言：** 简体中文。  
> **任务优先级：** 本轮只改开发环境、构建流水线、产物管理、验证和文档；**不改产品功能或 Midnight Glass UI**。

---

## 0. 最重要的用户需求（不可偏离）

用户在另一个软件 SRTTool 中使用如下约定：

```text
E:\Software Development\SRTTool\release-output\SRTTools\
E:\Software Development\SRTTool\release-output\SRTTools.zip
```

其中，`SRTTools/` 保存**最新版本的最终产物**，`SRTTools.zip` 保存**该最新版本的压缩包**。每次迭代成功后只保留最新版本，不希望开发目录累积大量旧版产物。

现在请将**同一理念**应用到 FluxDL，而**不要修改、清理或访问 SRTTool 的任何文件**。

FluxDL 的默认根目录按仓库实际位置自动探测。当前项目文档中的 Windows 开发示例为：

```text
E:\Software Development\FluxDL
```

但是**绝不能把该绝对路径硬编码到脚本里**。所有路径以正在执行脚本的 FluxDL 仓库根目录为基准，这样其他磁盘、包含空格或中文的路径也能使用。

最终交付路径必须是：

```text
<FluxDL仓库根目录>\release-output\FluxDL\
<FluxDL仓库根目录>\release-output\FluxDL.zip
```

正式构建成功后，`FluxDL/` 是当前版本最终交付文件夹，`FluxDL.zip` 是同一内容的 ZIP 压缩包。**本地不应继续积累旧版最终产物。**

### 必须遵守的四条原则

1. **开发与发布分离：** 日常开发命令不更新正式产物；只有显式正式构建/整理命令才更新 `release-output`。
2. **安全发布：** 先构建、先暂存、先验证，全部成功之后才替换旧版；任何失败不得破坏上一份可用产物。
3. **最新一版：** 发布成功后对外可见的正式产物只有一个 `FluxDL/` 和一个 `FluxDL.zip`。不要以版本号无限堆积文件夹和 ZIP。
4. **缓存按需清理：** 保留 `node_modules`、Rust `target` 等有用增量构建缓存；及时清除可以安全识别的旧发布产物和临时垃圾，但绝不每次构建全量清空编译缓存。

---

## 1. 首先审计当前仓库，复用已有能力

开始修改之前，必须先阅读以下文件（不存在则记录并跳过）：

- `AGENTS.md` 以及 `wiki_memory/AGENTS.md`、`wiki_memory/README.md` 和当前工程记忆规范要求的必要文件。
- `README.md`、`docs/validation.md`、`docs/engineering/` 中与构建、测试和发布相关的记录。
- `package.json`、`package-lock.json`。
- `src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、相关 Tauri bundle/resource 配置。
- `scripts/stage-release.mjs` 和 `scripts/` 下其他构建、许可证、验证脚本。
- `.gitignore`、`.github/workflows/windows.yml` 或仓库实际使用的工作流。
- `release/`、`release-output/`、`src-tauri/target/release/bundle/`（如果存在，只读检查，不先删除）。

执行并记录：

```powershell
git status --short --branch
git rev-parse HEAD
git remote -v
```

### 仓库现状线索（仍须用本地实际代码确认）

公开版本中已存在：

- `npm run desktop:dev`：Tauri 开发模式。
- `npm run desktop:build`：Tauri 2 Windows NSIS 构建。
- `npm run release:stage`：运行 `node scripts/stage-release.mjs`。
- 旧的 `stage-release.mjs` 把安装程序、`fluxdl.exe`、许可证、说明和哈希整理到 `release/`。
- 当前 Tauri 使用 NSIS 安装包，原始程序来自 `src-tauri/target/release/fluxdl.exe`。
- 当前 FluxDL 是**外部 yt-dlp / FFmpeg 引擎版**；不随程序自动附带这些工具。

**优先增强现有 `stage-release.mjs` 与 npm scripts，不允许无理由另外新建一套并行的发布系统。** 如果当前仓库已在此基础上更新，则以最新代码为准，按需修改。

在 `docs/build-and-release.md` 中先简要记录现状、原有命令、现有产物，以及本轮需要解决的问题。然后继续实际实施，不要停在文档阶段。

---

## 2. 最终目录契约：一个最新交付目录 + 一个 ZIP

建议目标结构如下；文件名与实际构建配置以项目真实值为准：

```text
FluxDL/
├── src/
├── src-tauri/
├── scripts/
├── docs/
├── package.json
├── package-lock.json
├── ...
└── release-output/
    ├── FluxDL/
    │   ├── FluxDL.exe                        # Tauri 主程序原始 EXE
    │   ├── FluxDL_<version>_x64-setup.exe    # NSIS 安装程序
    │   ├── LICENSE
    │   ├── THIRD_PARTY_NOTICES.md
    │   ├── licenses/                         # 项目已有的第三方许可证
    │   ├── 使用说明.txt
    │   ├── build-info.json
    │   └── SHA256SUMS.txt
    └── FluxDL.zip
```

### 具体要求

- `release-output/FluxDL/` 的内容来自**同一次成功的 Release 构建**，不得混入不同版本的安装器、旧 EXE 或旧许可证。
- `release-output/FluxDL.zip` 直接压缩验证过的最新 `FluxDL/`，ZIP 内以顶层目录 `FluxDL/` 开始。解压后得到相同目录结构。
- ZIP 内不能再次嵌入 ZIP 自身，也不能包含 `.git`、`node_modules`、`target`、调试数据、下载视频、AppData、用户数据库、测试媒体或开发日志。
- 最终交付目录中不保留上一个版本的 `FluxDL_旧版本_x64-setup.exe`。
- 许可证、第三方声明和外部引擎使用说明不可遗漏。
- **不能把 NSIS 安装器误称为便携版，也不能把原始 EXE 宣称为完全无依赖绿色单文件。** 原始 EXE 的实际启动要求须通过测试或文档说明，包括 WebView2、用户本地引擎配置和 AppData 持久化。
- 本轮**不加入** yt-dlp、FFmpeg、FFprobe、EJS、JavaScript Runtime 的自动下载和内置打包；保持现有外部引擎方案。
- 不动 `%APPDATA%` 中的 FluxDL 用户设置和 SQLite 数据；发布构建与清理永远不能涉及用户下载文件目录。

### 关于固定命名

- `FluxDL/` 和 `FluxDL.zip` 使用固定名称，覆盖时无需产生版本编号目录。
- 安装器内部可保留现有的版本号文件名，方便识别；但在最终交付文件夹中始终只能存在当前版本的那个安装器。
- `build-info.json` 记录真实版本和 Git 提交，而不是用文件夹名称表达历史版本。
- 不需要额外的 `FluxDL-0.2.1.zip`、`FluxDL-20261109.zip` 等历史归档。

---

## 3. 明确区分日常开发、构建、发布整理和清理

在保持现有命令兼容的前提下，为 `package.json` 提供一个清晰统一的命令入口。

建议命令语义：

| 命令 | 目标 | 是否更新 `release-output` |
|---|---|---|
| `npm run desktop:dev` | 正常 Tauri 开发、热更新 | 否 |
| `npm run build` | 前端生产构建/验证 | 否 |
| `npm run desktop:build` | 现有 Tauri 生产二进制与 NSIS 构建 | 否；仅落入 Tauri 构建目录 |
| `npm run release:build` | 完整正式流水线：检查、构建、暂存、验证、发布 | **是，仅全部成功后** |
| `npm run release:stage` | 复用已有构建结果，整理/验证并发布 | **是，仅全部成功后** |
| `npm run release:verify` | 只读验证最新文件夹和 ZIP | 否 |
| `npm run release:clean` | 受控清理临时和旧的构建/发布文件 | 根据显式参数，默认只预览 |

以上命令名是推荐接口。若当前已存在同名命令，**保留名称并兼容现有用途**。对于 `release:stage`，不应偷偷自动重新构建整个项目；应报告缺失或过期的构建输入。完整构建统一走 `release:build`。

### 3.1 `release:build` 必须具备的行为

1. 从**仓库根目录**解析实际版本与构建配置；检查 Node、npm、Rust、Tauri 和 Windows 目标环境。
2. 校验相关版本信息的一致性：例如 `package.json` 与 Tauri 配置的版本。如果有多处版本源，不一致时直接失败，禁止悄悄输出带错误版本号的文件。
3. 运行项目适用的工程检查（默认至少前端 typecheck / tests、Rust 关键 tests，具体从 `package.json` 和仓库现有规范读取）；注意不要在无关改动时引入过多重复构建。
4. 使用现有 `desktop:build` 生成 Windows Release EXE 与 NSIS 安装器。
5. 仅在确认**本次构建成功**后调用 staging；不能直接拿不明来源的旧二进制冒充本次构建。
6. 构建和文件整理完成后生成 ZIP 并校验。
7. 所有检查成功后才安全替换正式输出。
8. 如果任一阶段失败，返回非零退出码、给出明确错误、保留原来的 `release-output`，并清理本次安全临时文件。

如果项目现有测试存在明确的 Windows-only 或需要真实网络的可选测试，遵守既有验证边界，不能虚称执行过。

### 3.2 `release:stage` 的行为

允许开发者先自行执行 `npm run desktop:build`，然后通过 `release:stage` 整理。必须验证：

- 构建输入确实存在且非空。
- EXE 来源、架构、版本与安装器名称符合当前配置。
- 构建输入可被识别为预期可执行文件，而不是测试替身或错误文件（测试模式允许明确传入 fixtures）。
- 对已有构建产物的复用给出来源/新鲜度说明；如无法确认来源与当前源码相符，应报告风险或要求显式确认，不能静默声称与当前 HEAD 完全一致。
- 严禁把上次遗留的安装器误认为最新版本。

### 3.3 保持平时 Vibe Coding 高效

- 普通 `dev` 和 `test` 不触发 Release 全量构建。
- 不在 `predev`、`postbuild` 等生命周期钩子里偷偷重建安装包。
- 不每次执行 `npm ci`、删除 `node_modules` 或 `cargo clean`。
- 不为追求所谓「开发目录干净」而反复从零编译所有 Rust 依赖。

---

## 4. 采用安全的 staging → verify → promote 流水线

这是本轮最重要的实现要求。

**绝对禁止：正式构建开始就删除现有 `release-output/FluxDL/` 或 `FluxDL.zip`。**

要求实现：

```text
当前已存在的最新正式产物
          │
          │ 不动
          ▼
完整 Release 构建 → 临时 staging 目录
          │
          ▼
验证 EXE、安装包、许可证、元数据、哈希
          │
          ▼
从暂存目录生成 FluxDL.zip
          │
          ▼
解包/逐项校验 ZIP 和源文件夹完全一致
          │
          ▼
在受控发布锁保护下执行备份与替换
          │
   ┌──────┴───────┐
   ▼              ▼
成功             失败
   │              │
   ▼              ▼
只保留最新版      回滚至旧版 + 报错
```

### 4.1 暂存区域

- 在与最终输出相同的文件系统/卷上创建一次性 staging 目录，降低跨盘移动导致的失败风险。
- staging 不应在最终 `FluxDL/` 目录里面，否则容易被打包进去。
- staging 目录必须使用唯一、安全的名称，进程退出时可识别为脚本管理的产物。
- 不使用与别的项目重名的全局临时目录，不覆盖其他项目文件。
- 路径中有空格、中文、括号等字符时必须正常工作。

### 4.2 发布原子性与 Windows 限制

文件夹与 ZIP 是两个独立对象，在普通文件系统中不能简单保证两者一起原子替换。请实现**尽可能原子、可回滚、可恢复的两产物发布**，而不是声称完全事务化。

要求：

- 通过专用锁或等效互斥机制，禁止两个 Release 发布任务同时覆盖产物。
- 先完整构建 staging 的目录与 ZIP，**不要边复制边覆盖正式文件**。
- 发布替换时，保留短暂的旧版备份用于回滚；成功后清除安全范围内的备份，不累积历史版本。
- 如果移动、重命名、访问权限、杀毒软件锁定等使发布失败，尝试恢复旧版。
- 如果回滚也失败，**必须保留可恢复的旧版备份、输出绝对恢复路径和手动恢复步骤，不得继续清理该备份**。
- 发布完成时检查目录和 ZIP 对应同一份 `build-info.json` 与文件哈希。
- 可以采用 `release-output` 整目录受控替换，也可以采用受控的双对象替换；选择对 Windows 最可靠、实现最简单且易于测试的方案。
- 不能自动结束正在运行的 FluxDL 进程。若文件锁阻止替换，应友好失败并保留旧版，而不是强制杀进程。
- 能处理之前异常退出残留的 staging、backup 或 lock：明确检测、诊断和恢复，不允许盲目删除可能是唯一可用副本的备份。

### 4.3 校验失败时的策略

任何一项关键校验失败，**禁止发布新版本**，原正式目录和 ZIP 均保持不变（如发布过程中失败则按回滚机制恢复）。

例如：

- 缺少 NSIS 安装器。
- 缺少主程序 EXE。
- EXE 为零字节或明显异常。
- 版本号不一致。
- 许可证缺失。
- `build-info.json` 不完整。
- ZIP 无法打开、CRC 校验失败或内容与目录不一致。
- 无法写入正式目录。
- 发布锁已由另一构建任务持有。

不要把这些错误转为 warning 然后继续覆盖旧版。

---

## 5. 构建元数据和哈希一致性

在最终 `FluxDL/` 中提供可读的 `build-info.json`，至少包含：

```json
{
  "product": "FluxDL",
  "version": "<从真实项目配置获取>",
  "gitCommit": "<实际 HEAD 短哈希>",
  "gitDirty": false,
  "buildTimeUtc": "<真实 UTC ISO8601 时间>",
  "target": "windows-x64",
  "profile": "release",
  "packaging": "tauri2-nsis-and-raw-exe",
  "engineDistribution": "external"
}
```

这只是字段格式示例；值必须由代码计算，不能硬编码示例版本或时间。如果工作区有未提交变更，**不得写成干净构建**，应准确标记 dirty；不能凭 Commit 哈希冒充产物与远端一致。

### 哈希清单

- 生成 `SHA256SUMS.txt`，覆盖最终 `FluxDL/` 内所有需要校验的常规文件，包括 EXE、安装器、许可证、说明和 `build-info.json`；**清单文件自身除外**。
- 哈希校验按**相对路径**记录，确保在另一台 Windows 机器上可复验。
- 路径排序稳定、编码一致，注意中文文件名。
- ZIP 构建完成后验证文件清单和内容哈希与 `FluxDL/` 一致，禁止只比较 ZIP 文件大小。
- ZIP 解压后的顶层结构应是 `FluxDL/...`；不得意外多一层 `release-output/` 或 staging UUID 目录。
- 最终可使用 `release:verify` 独立重复校验，即使不重新编译也能验证交付目录。
- `release:verify` 必须是只读的，不得修复、替换或重新生成产物。

### 额外要求

- 不要把完整绝对开发路径、机器用户名、密码、Token、Cookies、用户历史下载文件等写入元数据。
- 不要把源码 `.env` 或本机机密配置打进 ZIP。
- 版本来源应避免多个文件静默分叉。必要时新增版本一致性检查，不在本轮引入新的版本发布服务。

---

## 6. ZIP 生成与验证的技术要求

- 优先使用项目已经拥有的 Node.js 脚本能力和 Windows 标准工具，**避免为压缩 ZIP 引入体积较大的新依赖**。
- 可以复用 Node ESM 进行编排，通过 Windows 自带的 .NET ZIP 或其它可靠的本地 ZIP 方案生成归档；选择方案时测试中文路径、空格路径、目录嵌套和不同大小文件。
- 注意 `Compress-Archive` 等工具在大文件上的限制。若采用它，明确约束并提供实际校验；不要假设所有版本没有文件大小限制。
- ZIP 内的路径分隔符与目录结构要兼容常见 Windows 解压工具。
- ZIP 内容应准确对应暂存目录，不应读正在被替换的正式目录来打包。
- ZIP 中包含 `build-info.json` 和哈希文件，方便独立校验。
- 如果归档内包含系统符号链接或意外文件，应进行显式处理或报错，不随意跟随链接读取仓库外部内容。

---

## 7. 开发环境与旧产物的清理策略

用户的意图是**不要保留多份旧版最终构建产物**，而不是每次迭代都重装依赖、重新编译一切。

请实现一个可审计、可预览、安全的清理机制。

### 7.1 允许自动清理的对象

仅在新版本**已经成功发布**后，允许自动清理脚本自己创建、且可确认不再需要的：

- 本次成功发布用过的 staging。
- 本次发布的旧版临时 backup（仅在新产物验证成功、回滚不再需要后）。
- 本脚本明确所有权的临时 ZIP、临时清单和中间文件。
- 在已确认安全的情况下，旧版 NSIS 安装包等**明确命名、明确限定目录**的多版本发布产物。

### 7.2 不允许自动删除的对象

- `node_modules/`。
- `src-tauri/target/debug/` 或全部 Rust 增量编译缓存。
- 用户实际下载文件、AppData 数据库、配置文件。
- 非 FluxDL 的任意目录，包括 `E:\Software Development\SRTTool\...`。
- 没有脚本所有权标记的未知文件或目录。
- `src/`、`src-tauri/src/`、`docs/`、`wiki_memory/`、`.git/`。
- 构建失败时的上一份可用正式产物。
- 回滚失败时仅存的一份 backup。

### 7.3 旧 `release/` 迁移与清理

当前版本可能遗留旧的 `release/` 目录，`scripts/stage-release.mjs` 曾经写入它。不要直接无条件递归删除它。

- 新流水线统一只写 `release-output/`。
- `release/` 不再作为公开交付目录。
- 首次检测到旧 `release/` 时，报告它存在，检查是否完全属于历史自动生成文件。
- 如果有未知文件，不清理；告诉用户如何处理。
- 建议通过 `npm run release:clean` 的显式执行选项（如 `-- --apply --legacy`）删除**经过预览、白名单确认**的历史版本目录。
- 清理前做路径边界校验，禁止超出仓库根目录，也禁止删除符号链接指向的外部目录。

### 7.4 `release:clean` 的默认行为

- 默认是 **dry-run**：只列出候选文件、路径、大小以及为何能删除。
- 需要 `--apply` 或等效显式参数才执行实际删除。
- 可提供 `--deep` 等可选模式，但必须清楚提示其会删除 Rust 或 Node 缓存，**不能作为普通发布默认步骤**。
- 对所有删除路径执行归一化、项目根目录边界、目录所有权及符号链接检查。
- 检查输出路径不为空、不是磁盘根目录、不是仓库根目录、不是用户目录。
- 清理命令应尽可能幂等。
- 报告清理前后空间大小（如果测量成本合理）。

**特别强调：** 不要使用 `git clean -fdx`、`cargo clean`、`Remove-Item -Recurse -Force` 对不受控通配路径执行危险清理；更不能删除整个 `E:\Software Development`。

---

## 8. `release-output` 必须忽略 Git，源码脚本可提交

检查并更新 `.gitignore`：

```gitignore
# Local final delivery only
/release-output/
```

现有 `release/`、`dist/`、`node_modules/`、`src-tauri/target/` 等忽略规则应保留。

使用 `git check-ignore` 验证文件确实被忽略。

**不要将以下内容加入 Git：**

- `release-output/FluxDL.exe`。
- 任何 NSIS `.exe` 安装器。
- `release-output/FluxDL.zip`。
- staging / backup / lock 临时产物。
- 编译缓存、日志、用户 SQLite、个人配置、下载媒体。

**应该纳入版本管理：**

- 修改后的构建/暂存/验证/清理脚本。
- `package.json` 脚本入口及必要配置。
- 测试脚本和测试夹具代码（不包括巨型二进制产物）。
- `docs/build-and-release.md`、`README.md` 的相关说明。
- `.gitignore` 变更。
- 如仓库规则要求，更新 `AGENTS.md` 和工程记忆中具有持久价值的构建约定。

Git 提交与推送**遵循本仓库当前 `AGENTS.md` 中已记录的用户授权和交付规则**：先检查 origin / HEAD / 分支与工作区，避免混入他人改动，不强制推送；如果该授权仍有效，按其规范提交和推送**脚本、配置、测试和文档**，绝不提交本机二进制产物。若用户有更新授权，遵从较新的明确要求。

---

## 9. 测试场景：不仅要能成功构建，更要能安全失败

请为流水线增加自动化测试，尽可能使用**临时项目 fixture 和假二进制文件**来验证 staging / ZIP / rollback，不要每次测试都完整编译 Tauri。

测试模式与生产模式必须清楚隔离：生产模式不能接受测试 fixture 产物冒充真实 Release。

### 9.1 功能测试矩阵

| 场景 | 预期 |
|---|---|
| 首次发布，原目录不存在 | 创建 `release-output/FluxDL/` 和 `FluxDL.zip` |
| 第二次发布同版本新构建 | 覆盖为最新内容，不残留旧文件 |
| 第二次发布不同版本 | 只保留新版本的安装器和 ZIP |
| 重复执行 stage（相同输入） | 安全幂等、不积累临时目录 |
| 主程序 EXE 缺失 | 明确失败，旧产物不变 |
| NSIS 安装器缺失 | 明确失败，旧产物不变 |
| 许可证缺失 | 明确失败，旧产物不变 |
| 版本号不一致 | 明确失败，旧产物不变 |
| ZIP 生成失败 | 明确失败，旧产物不变 |
| ZIP 内容被篡改 | verify 失败，且只读不修复 |
| 构建中断 / staging 残留 | 可诊断、安全清理，不误删旧产物 |
| 发布移动/重命名失败 | 回滚旧版并保留故障证据 |
| 回滚失败 | 旧版备份不删除，报告恢复路径 |
| 两个发布命令同时执行 | 互斥，防止互相覆盖 |
| `release:clean` 无参数 | 只展示候选项，不删除 |
| 有未知文件在旧 `release/` | 不删除未知文件 |
| 仓库路径包含空格/中文 | 完整流程可运行 |
| 运行中的 FluxDL 占用 EXE | 不强制关进程，安全报错或按可验证方式完成 |
| ZIP 解压结构检查 | 顶层为 `FluxDL/`，内容与交付文件夹一致 |
| `git status` 检查 | 构建产物不被 Git 跟踪 |

### 9.2 测试中的路径保护

测试不得真实删除或修改用户的正式 `release-output`，除非是在明确的真实发布验收阶段。

- 测试使用系统临时目录下独立的 fixture 项目根目录。
- 测试创建的临时路径必须由测试独占，退出时仅删除其自身所有路径。
- 不模拟磁盘根目录为输出路径。
- 对清理路径的防越界检查做独立单元测试。
- 对符号链接、异常路径、非预期文件和 Windows 文件锁尽量增加测试。

### 9.3 发布后独立验收

`npm run release:verify` 至少输出：

- 产品名与构建版本。
- Git Commit、dirty 标识和构建时间。
- 主程序 EXE 与安装包的文件名和大小。
- 文件数量、目录 SHA256 校验结果。
- ZIP 是否存在、能否打开、是否有遗漏/多余文件。
- 解压对照的文件清单/哈希一致性。
- 最终绝对路径。

不能只用“文件存在”作为验收通过条件。

---

## 10. Windows 实机验收

如果当前环境是 Windows 且具备实际 Tauri 编译工具链，请**真实执行一次完整正式构建**，然后进行验证。

建议步骤：

1. 确认 `npm run desktop:dev` 和现有开发工作流不被改动破坏。
2. 运行仓库要求的静态检查与自动化测试。
3. 执行 `npm run release:build`。
4. 执行 `npm run release:verify`。
5. 解压 `release-output/FluxDL.zip` 到新建临时目录，比较全部文件与原目录 SHA256。
6. 在条件允许时执行必要的程序启动 Smoke Test；注意这不代表已完成所有安装、卸载或便携版测试。
7. 再运行一次正式整理，确认不会积累旧文件。
8. 在隔离测试目录中模拟失败和回滚，确认旧产物受保护。
9. 检查 `git status`、`git check-ignore`，确保最终 EXE 和 ZIP 未进入版本管理。

如运行环境不是 Windows 或缺少 Rust/MSVC/Tauri 打包依赖：

- 完成所有可运行的静态检查、Node 脚本测试、目录/ZIP 逻辑测试。
- 清楚记录没有实际构建成功的项目。
- 列出用户在 Windows 上可执行的精确命令。
- 不得假称已生成 Windows EXE 或真实安装包。

---

## 11. 兼容性、安全与工程范围

### 本轮不允许变更

- 不修改 Quick Capture / Workspace 的 UI 布局。
- 不改 Midnight Glass 配色、主题、动效。
- 不修改 yt-dlp 下载引擎、任务调度、SQLite 状态模型和 IPC 协议。
- 不增加首次启动引导、自动更新或任何新增下载功能。
- 不为了打包改动系统用户数据目录。
- 不引入账号、云端、遥测。
- 不把外部引擎突然打包到发行物中。
- 不重构大量与构建无关的前端或 Rust 业务文件。

### 若发现现有设计缺陷

仅修复完成本任务必需的问题，其余列入“后续建议”。不要顺手扩大范围。

### 资源与依赖处理

- 使用实际 Tauri 构建产物，不硬编码不存在的资源文件。
- 保留现有许可证/声明与第三方资源的分发要求。
- 不打包 `target`、`node_modules`、源码、测试数据。
- 若认为“直接运行的 `FluxDL.exe`”缺少运行所需资源，应先检查 Tauri 资源嵌入、外部文件及运行时依赖，再决定交付文件夹应包含哪些文件，不能只凭文件名推测。
- 对 Windows WebView2 的需求如实说明；本轮不进行自动引擎安装。

---

## 12. 文档与命令示例交付

更新 `README.md` 的本地开发与构建部分，新增或完善 `docs/build-and-release.md`，并在其中准确写出：

### 普通开发

```powershell
cd 'E:\Software Development\FluxDL'
npm run desktop:dev
```

日常开发不会触碰 `release-output`，不会清空 Rust 和 npm 缓存。

### 完整正式构建

```powershell
cd 'E:\Software Development\FluxDL'
npm run release:build
npm run release:verify
```

成功后最新交付在：

```text
E:\Software Development\FluxDL\release-output\FluxDL\
E:\Software Development\FluxDL\release-output\FluxDL.zip
```

文档必须提醒这些只是**该目录位置下的示例**，构建脚本本身基于仓库根目录自动定位。

### 使用已有 Tauri 构建产物重新整理

```powershell
npm run desktop:build
npm run release:stage
npm run release:verify
```

### 清理预览和明确清理

```powershell
npm run release:clean
# 若脚本支持相应选项，确认候选路径后才运行：
npm run release:clean -- --apply
```

如果最终命令有所不同，请以实际实现更新文档，不能留下不可运行的示例。

文档还应解释：

- `desktop:build` 与 `release:build` 的区别。
- 正式目录与 ZIP 的一致性保证。
- 构建失败时为什么旧版不会丢失。
- staging / backup 残留时的恢复步骤。
- 外部 yt-dlp、FFmpeg 和 WebView2 运行要求。
- 为什么日常开发缓存会保留。
- 怎样按需深度清理，而不误删用户数据。
- “最新本地包”不是 GitHub Release，也不意味着已经完成签名或分发适配。

---

## 13. 推荐实现步骤（必须依次验证）

### 阶段 A：审计与方案

- 检查当前仓库、构建脚本、已有输出与 Git 状态。
- 明确真正的 Tauri 输出位置和版本来源。
- 定义 `release-output` 的准确内容。
- 检查旧 `release/` 的归属与风险。
- 选择尽可能简单、Windows 可靠的替换与 ZIP 方案。

**验收：** 方案与实际仓库吻合，不存在盲目清理路径。

### 阶段 B：构建与暂存

- 改造现有 `stage-release.mjs`，复用已有整理逻辑。
- 增加必要的独立验证模块与完整 `release:build` 入口。
- 创建 `build-info.json`、扩展 SHA256 校验。
- 在 staging 中生成完整目录与 ZIP。

**验收：** 不动旧版也能生成、验证一组候选产物。

### 阶段 C：安全替换与清理

- 加入并发保护、替换前备份、失败回滚。
- 确保最终仅 `FluxDL/` + `FluxDL.zip`。
- 增加只读 verify 和 dry-run clean。
- 处理旧 `release/` 的迁移提示，不强制删除未知文件。

**验收：** 成功覆盖最新，失败保留旧版，清理不越界。

### 阶段 D：自动化测试与实机验证

- 实现不依赖真实 Tauri 编译的 fixture 测试。
- 运行现有静态检查/测试。
- Windows 环境中完成一次真实 release build。
- 验证 ZIP 解压一致、错误路径、回滚和 Git 忽略。

**验收：** 测试具有真实输出和可复现命令，不只写“应通过”。

### 阶段 E：文档、工程记忆与 Git 交付

- 更新 README、构建发布说明和长期工程约定。
- 检查 diff，避免无关文件修改。
- 遵守当前 AGENTS.md 的用户授权与推送流程，仅提交源码、脚本、文档与测试，不提交产物。
- 提供最终交付报告。

---

## 14. 最终验收清单

每项请基于实际执行结果填写，不要预先全部标记通过：

- [ ] 已阅读并遵循 `AGENTS.md` 与工程记忆协议。
- [ ] `desktop:dev` 仍正常开发，不修改正式产物。
- [ ] `desktop:build` 语义保留，`release:build` 为完整统一入口。
- [ ] `release:stage` 复用现有构建结果，没有另造冲突流水线。
- [ ] 构建成功后生成 `release-output/FluxDL/`。
- [ ] 构建成功后生成 `release-output/FluxDL.zip`。
- [ ] ZIP 解压后顶层为 `FluxDL/`，全部文件与最终目录一致。
- [ ] `build-info.json` 版本、Commit、dirty、时间和架构准确。
- [ ] SHA256 清单正确，验证命令可独立复跑。
- [ ] 第二次发布能覆盖旧产物，不堆积旧版本。
- [ ] 构建失败、缺失文件、ZIP 失败时旧产物不受损。
- [ ] 替换失败能回滚；回滚失败有明确备份路径。
- [ ] 并发发布互斥有效。
- [ ] 普通开发保留编译缓存，清理命令默认 dry-run。
- [ ] 旧 `release/` 有安全迁移或显式清理策略。
- [ ] 不删除 SRTTool、用户下载、SQLite/AppData 或仓库外文件。
- [ ] `.gitignore` 正确忽略 `release-output/` 和相关临时产物。
- [ ] 不误称原始 EXE 为无需 WebView2 / yt-dlp 的完全自包含便携版。
- [ ] 相关自动化测试真实运行并记录结果。
- [ ] Windows 环境下执行了真实 Release 构建（如环境允许）。
- [ ] README、构建文档和工程记忆保持一致。
- [ ] 符合仓库的 Git 提交/推送授权与范围约束。

---

## 15. 最终答复必须交付的内容

任务完成后，以中文给出以下结构化结果：

1. **改造总结：** 修改了哪些命令、脚本与文件。
2. **当前最终目录树：** 展示 `release-output/FluxDL/` 和 `FluxDL.zip` 实际内容及路径；如果不能实际构建，明确说明没有生成，不能伪造目录树。
3. **一键正式构建命令：** 给出已经验证的精确命令。
4. **ZIP 一致性结果：** 文件数量、哈希对照、验证命令和结果。
5. **失败回滚测试：** 列出至少一个模拟失败场景、旧产物是否保持不变。
6. **开发缓存策略：** 哪些默认保留，哪些可以受控清理。
7. **旧 `release/` 处理：** 是否存在、已迁移还是保留、原因。
8. **验证结果：** typecheck、tests、Rust 检查、真实 Windows 构建、verify 等真实执行结果。
9. **未完成事项与风险：** Windows 环境限制、WebView2/外部引擎依赖、原始 EXE 是否完成直运行验证。
10. **Git 交付：** 实际提交与推送状态（遵循 AGENTS.md），确保未上传二进制和临时文件。

**严禁报告未执行过的测试为“通过”，不得把仅存在脚本文件解释为流水线已经实机可用。**

---

## 16. 立即执行指令

你现在位于 FluxDL 项目工作区。请直接执行以上工程改造，而非仅生成计划。

**执行顺序：**

1. 审计 Git、项目约定和当前脚本。
2. 确定当前构建输出，检查已有 `release/` 和历史文件。
3. 在现有 `stage-release.mjs` 基础上实现统一的 Release 流水线。
4. 实现 staging、完整性验证、ZIP、备份、替换和回滚。
5. 实现只读 verify、受控 dry-run clean 和自动化测试。
6. 验证日常开发路径不会改变正式产物。
7. 在 Windows 环境中实际执行 `release:build` 与 `release:verify`。
8. 更新文档、必要的工程记忆并按仓库已授权规则交付代码。
9. 输出第 15 节要求的最终结果。

如果实际仓库已经存在更完善的实现，请保留已有可靠机制，只补齐缺失部分。**简单、可审计、可测试、可回滚，比为了“自动化”引入复杂架构更重要。**

最终目标只有一句话：

> **平时开发不臃肿；正式构建一条命令；最新产物一个文件夹加一个 ZIP；成功才覆盖，失败可恢复；永远不碰用户数据。**
