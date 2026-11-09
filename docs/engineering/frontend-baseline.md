# 前端重构前保护测试与基线

测量日期：2026-11-09，原始结果见 [frontend-baseline.json](frontend-baseline.json)。本阶段没有修改生产 App、Store 或 IPC 协议。固定 fixture 与 API mock 只存在于 `src/test/`。

## 可重复命令

在项目根目录执行：

```powershell
npm run typecheck
npm test
$env:FLUXDL_FRONTEND_BASELINE = '1'
npx vitest run src/test/frontend-baseline.test.tsx
Remove-Item Env:FLUXDL_FRONTEND_BASELINE
```

常规 `npm test` 不执行耗时基准；基准需显式环境变量，成功后覆盖 JSON。后续复测前应保留重构前结果，不能把旧结果改写为新协议已经验证。2026-11-09 实际结果：类型检查通过，常规测试20项通过、基准1项按设计跳过；显式基准1项通过。新文件已通过 Prettier。全项目 lint 当前只有原有 `App.tsx` effect 缺 `notify` 依赖的 warning，因为 `--max-warnings 0` 返回失败；本阶段新增测试没有 lint 错误，后续同步 Hook 迁移时需修正。

阶段2补充：上述为生成原始基线时的实际命令。原始JSON现已锁定，harness拒绝覆盖；后续须设置`FLUXDL_FRONTEND_PHASE`与新目标`FLUXDL_FRONTEND_REPORT`，参见[阶段2验收](frontend-stage2.md)。阶段1的lint warning是当时观察，阶段2已修正且全项目lint通过。

## 方法与指标

环境：Windows 11 x64 / 10.0.26200，Node v24.16.0，ReactDOM 开发构建和 jsdom，基准不启用 StrictMode。每组总任务数为100、1000、5000；其中1个正在下载，其余为历史完成项。保持历史页打开，仅改变不可见活动任务的真实进度字段。

- 模拟 JSON 解码产生所有新对象，解码在计时外完成；每个样本依次调用当前生产 Store 的 `setSnapshot`。
- Store通知与序列化各20个样本。React预热1次后采样5次，使用真实 App 和 React Profiler；更新时间包含 React、DOM commit 和 effects。
- History 中每个 TaskRow 恰好渲染一次 MoreHorizontal 图标。基准对该图标使用透明计数包装，并继续渲染原 Lucide 图标，用来观察无关任务行的 React render 次数；未改变生产组件。
- p50/p95 使用 nearest-rank。React仅5个样本，所以 p95 为观察到的最大值。

| 总任务数 | Snapshot字节 | 序列化p50/p95 ms | React actualDuration p50/p95 ms | 更新至effects p50/p95 ms | 每次无关历史行render |
|---:|---:|---:|---:|---:|---:|
| 100 | 67,338 | 0.106 / 0.109 | 17.356 / 65.286 | 24.739 / 73.738 | 99 |
| 1,000 | 670,338 | 1.265 / 1.337 | 241.446 / 532.417 | 305.212 / 661.195 | 999 |
| 5,000 | 3,362,338 | 10.002 / 11.560 | 1,897.639 / 1,932.225 | 2,509.177 / 2,554.889 | 4,999 |

每组20次 Store 更新均通知20次；纯 Store setter p50/p95 分别约0.002/0.010、0.002/0.005、0.011/0.014 ms。小的 setter 时间不包含 React，不能据此判定界面更新成本小。五次 React 样本均只提交1次，但每次会重渲染所有无关历史行，提供了阶段2/4优化的可观察基线。

这些是同一受控夹具的开发环境比较值，不是原生 WebView2 的布局、绘制、IPC传输或FPS测量；Profiler与透明包装有额外开销，jsdom的绝对耗时不能外推到发布版。fixture使用精简媒体元数据和空日志，payload是可重复的下界示例。未采集进程内存占用，也未执行真实下载。

## 新增保护范围

- `store.test.ts`：初始读取、后端任务新增、进度与终态、删除与顺序、刷新错误保留最后数据。
- `pages.test.tsx`：队列三类筛选及后端顺序；历史只显示完成项、按更新时间倒序、标题/来源不区分大小写搜索。
- `ipc-errors.test.tsx`：解析、入队、取消和设置保存失败后用户可见错误；不假称加入成功，保留任务或设置草稿并恢复操作。
- `subscriptions.test.tsx`：StrictMode后每事件只有一个活动订阅；注册晚于卸载时仍解除；focus只在挂载时刷新；退出失败通知可见。

现有全量快照没有版本号，过期响应、乱序与跨会话保护是已记录问题，没有编写“当前已经正确拒绝旧事件”的虚假断言。阶段4应基于实际 envelope 补初始化期间事件、旧请求晚返回、重复/乱序/缺口、跨会话和终态保护测试。现有正常卸载测试不代表注册拒绝已经有错误反馈。
