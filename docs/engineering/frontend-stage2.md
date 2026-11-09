# 阶段2前端模块化验收

先提取原函数体并通过原20项测试，再规范化缓存、收敛订阅和拆分样式。保留原snapshot IPC及产品DOM/class，没有修改Rust、依赖、发布策略或Git。

## 模块入口

- `src/App.tsx`：17行，只选择窗口、应用外观与错误提示。
- `src/windows/WorkspaceWindow.tsx`：布局、导航、抽屉和关闭/后台/退出；`QuickCaptureWindow.tsx`：捕获窗口、高度、滚动和Esc。
- `src/pages/`：Home、Queue、History、Settings；`features/capture/`：输入解析、媒体选项及入队；`features/downloads/`：任务行/详情按ID订阅，TaskRow使用memo。
- `components/ui.tsx`、`overlays.tsx`、`feedback.tsx`：控件、抽屉/对话框、错误提示。
- `hooks/useAppSync.ts`：先注册监听再读初始快照，处理注册错误和卸载；`useWindowAppearance.ts`按主题/密度/动画标量订阅，进度不重装系统主题监听；`useAction.ts`负责操作busy/错误/刷新。
- `stores/app.ts`：可工厂创建，只有settings、engines、tasksById、taskIds；`toSnapshot`按需转换协议视图，不保存第二份任务真相。全量快照保留未变记录引用，重复快照不通知。
- `stores/selectors.ts`：筛选/搜索/排序及每消费者缓存；listingRevision仅作为列表失效标记，普通进度不重算历史列表。
- `lib/api.ts`仍为IPC边界；预设移至`presets.ts`，纯格式化和校验保留在utils。

本地请求序号/generation防止旧请求覆盖订阅期间新状态。线级事件仍没有session/revision，旧事件、缺口和跨会话保护留给阶段4，不能将本阶段描述为完整版本协议。

CSS入口按原序引入tokens、themes、globals、window、workspace、downloads、details、settings、overlays、quick-capture、refinements、responsive。保留晚期Quick/字号覆盖，加入等值圆角/字号/页面间距Token；抽取时先确认分块拼接逐字符等同原文，再替换等值Token。

## 验证与性能

实际通过：`npm run typecheck`、`npm run lint`、`npm run format:check`、`npm test`（29项通过，基准1项默认跳过）、`npm run build`。9项新增覆盖结构共享、列表失效、独立缓存、本地请求逆序、监听优先与注册拒绝；原用户行为覆盖保留。

同一完整snapshot负载结果见[frontend-stage2.json](frontend-stage2.json)，原[frontend-baseline.json](frontend-baseline.json)未覆盖，harness拒绝覆盖该原始文件。

| 任务数 | 原无关行render | 阶段2无关行render | 原更新至effects p50 ms | 阶段2 p50 ms | 阶段2纯setter p50 ms |
|---:|---:|---:|---:|---:|---:|
| 100 | 99 | 0 | 24.739 | 0.356 | 0.274 |
| 1,000 | 999 | 0 | 305.212 | 3.164 | 2.561 |
| 5,000 | 4,999 | 0 | 2,509.177 | 15.645 | 12.674 |

该场景历史页无相关数据变化，故React commit及Profiler actualDuration均为0；**不表示Store处理免费**。结构共享比较使5000任务setter从约0.011ms升至12.674ms；完整snapshot仍为3,362,338字节，阶段4需继续减少全量解析/比较。环境仍是ReactDOM开发构建/jsdom，不是WebView2帧率；其余方法及限制见[基线说明](frontend-baseline.md)。

复测使用同一harness并写新文件：

```powershell
$env:FLUXDL_FRONTEND_BASELINE='1'
$env:FLUXDL_FRONTEND_PHASE='stage-2'
$env:FLUXDL_FRONTEND_REPORT='docs/engineering/frontend-stage2-rerun.json'
npx vitest run src/test/frontend-baseline.test.tsx
Remove-Item Env:FLUXDL_FRONTEND_BASELINE,Env:FLUXDL_FRONTEND_PHASE,Env:FLUXDL_FRONTEND_REPORT
```

CSS证据见[frontend-stage2-visual.json](frontend-stage2-visual.json)：Chromium中同一当前DOM分别使用提取后CSS和Git HEAD原样式，冻结动画/光标比较截图。四页面、760px窄窗、Quick空态各深/浅主题，共12项像素一致、无横向溢出、无JS异常。该证据仅证明样式等值，不代替旧DOM逐字比较、原生双窗口或高DPI验收；桌面流程由总体验收另行执行。

## 可重复样式等值检查

已将上述检查保留为仓库脚本，不依赖本机 ignored 临时脚本：

```powershell
npm ci
node scripts/check-style-equivalence.mjs
```

无需先执行`npm run build`或手动启动开发服务器。脚本使用已有Vite及Playwright依赖，在`127.0.0.1`随机端口启动自己的Vite服务器，另开全新临时profile的无头浏览器，结束后关闭自身资源；不连接CDP、不使用9322、不接管现有FluxDL或浏览器实例。

Windows默认优先使用本机安装的Edge；没有系统Edge时，使用Playwright默认Chromium。若默认浏览器尚未安装，可显式执行`npx playwright install chromium`，或指定已有浏览器：

```powershell
node scripts/check-style-equivalence.mjs --browser-executable 'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'
```

也可通过`FLUXDL_BROWSER_EXECUTABLE`指定路径。不会自动安装浏览器或改依赖。

默认基线固定为`cdc47a33c78e22160e7c4ba75f51adb6abb420f5:src/styles/app.css`，需本地Git历史包含该提交；脚本只读取Git，不fetch、commit或push。另选已存在的ref及新证据目录：

```powershell
node scripts/check-style-equivalence.mjs --baseline-ref cdc47a3 --output-dir artifacts/style-equivalence/my-new-run
```

默认输出为`artifacts/style-equivalence/<时间戳>/`，含12对截图和JSON报告；报告内截图路径相对项目根目录。已有输出目录会被拒绝覆盖，原始before及本阶段证明不会改写。指定ref的相对CSS导入会按原顺序读取；只支持项目`src/styles/`内的直接相对`.css`导入，不作为通用CSS历史资源加载器。

2026-11-09实际执行默认命令：12/12通过、errors为空，新证据位于`artifacts/style-equivalence/2026-11-09T05-08-33-523Z/report.json`。该脚本始终比较**同一当前DOM的两套样式**，不校验旧版DOM或真实Tauri尺寸，不用于Quick原生resize时序诊断。
