# FluxDL 项目协作规则

## 开发与验证

- 沟通、工程说明和交付使用简体中文。产品需求以 `prompt/FluxDL_Codex_MVP_Prompt.md` 为基线；实现以当前源码和配置为准。
- Windows-first，React / TypeScript / Vite + Tauri 2 / Rust。真实下载、队列、进度与持久化由 Rust 管理；浏览器只作界面预览。
- 修改前检查 `git status --short --branch`、当前 HEAD 与实际远程分支，保留用户和其他任务的改动。
- 按修改范围执行必要检查：前端 `npm run typecheck`、`npm run lint`、`npm run format:check`、`npm test`；后端 `npm run check:rust`、`npm run format:rust`、`npm run lint:rust`、`npm run test:rust`；队列生命周期使用 `./scripts/test-queue.ps1`。仅改记忆或文档时检查相关引用和记忆，不为记忆重跑业务测试。当前验证边界见 `docs/engineering/0.2.0-validation-report.md`，`docs/validation.md`保留0.1历史。

## Git 交付：持续授权

用户于 2026-11-09 明确要求：以后每次完整对话产生的工程修改均提交并推送到 `https://github.com/DewMysimple/FluxDL.git`。该要求适用于本工程后续会话，直至用户修改或撤销。

2026-11-09执行 `prompt/FluxDL_Codex_Engineering_Refinement_0.2.0.md` 有明确例外：该任务第2.2、14.H节要求不自动commit/push、不创建Release，因此这一完整执行保留本地修改并报告工作树，不提交或更改远程。例外不自动扩展到后续新任务；新任务恢复上述持续授权，除非用户再次指定其他要求。

- 远程名 `origin` 指向上述仓库；交付前核对实际 URL 和 upstream。当前主分支为 `main`，其他任务分支遵循其实际 upstream，不悄悄推到其他仓库。
- 完成修改后，先同步有持久价值的工程记忆、执行必要检查、审查差异，再提交本次已授权修改并推送。该范围内的提交和推送已有授权，无需重复确认。
- 纯只读对话或没有文件变化时不制造空提交。暂存前检查变更归属，不将用户无关改动或其他任务成果混入本次提交。首次交付包含此前已完成且尚未提交的 FluxDL MVP。
- 依赖、构建目录、安装包、测试媒体、用户数据库、凭证和本机临时数据不纳入提交；遵循 `.gitignore`。发布产物通过本地 `release/` 或 CI artifacts 交付。
- 不使用强制推送、重写远程历史或丢弃用户改动来完成交付。远程领先时先获取并保全、整合相关变更；推送失败时保留本地提交并如实报告原因及恢复位置。
- 推送后核对远程分支 SHA 与本地 HEAD 一致，并在最终答复中说明提交与推送结果。验证报告只声称实际执行和覆盖的范围。

<!-- wiki-memory:start -->
## 工程记忆

本项目使用工程根目录下的 `wiki_memory/`。新会话读取 `wiki_memory/AGENTS.md` 和 `wiki_memory/README.md`。按协议区分只读窄查询与修改、接续等任务，后者先读取规定的当前状态与全局约束。已读且未变的内容可复用。
在已授权开发任务完成或中断时，按协议同步有证据的状态、验证和恢复下一步；只读任务遵守只读范围。
日常读取、同步和交接直接遵循项目协议，不调用 wiki-memory Skill。仅用户明确要求配置、迁移、体检等记忆管理时加载 Skill；检查频率按项目协议，不把完整验收用于每轮收尾。
<!-- wiki-memory:end -->
