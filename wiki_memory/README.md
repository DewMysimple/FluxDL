---
type: moc
status: active
updated: 2026-11-09
topic: memory-entry
sources:
  - "用户指令：2026-11-09 使用 wiki-memory 构建标准工程记忆，并持续提交推送 FluxDL 仓库"
  - "wiki_memory/.memory.json"
---

# 工程记忆导航

模式：`standard`，由用户于 2026-11-09 明确选择；按五页状态和六类工作日志建立。使用相对 Markdown 导航，配置见 [.memory.json](.memory.json)。本页只作导航，当前事实放在对应状态页。

日常开发直接遵循项目内协议，不调用 wiki-memory Skill；仅用户明确要求记忆管理时使用 Skill。

- [记忆维护协议](AGENTS.md)
- [工作日志 MOC](日志/MOC_工作日志.md)

修改、恢复接续、架构或交付任务先读 [项目概览](当前状态/项目概览.md)、[当前约束](当前状态/当前约束.md)、[当前待办](当前状态/当前待办.md)，再按范围读取架构、问题与相关主题。明确且独立的只读窄查询可以直达专题和源码。同一上下文已读且未变的内容可复用。

需要来由时再读决策或日志。用 `rg -n '主题词' wiki_memory src src-tauri/src docs` 按主题检索；不必每轮通读全部日志。源码、配置和用户要求优先于记忆，历史测试不代表当前分支已经验证。

导航变化后运行 `python wiki_memory/工具/memory.py index --project . --apply`；记忆修改后运行一次 `python wiki_memory/工具/memory.py check --project .`。初始化或明确完整验收才加 `--require-ready`，替代普通检查。项目内工具独立于个人 Skill 安装路径。

<!-- wiki-memory:auto:start -->

## 当前状态

- [已知问题](当前状态/已知问题.md) — active
- [当前待办](当前状态/当前待办.md) — active
- [当前约束](当前状态/当前约束.md) — active
- [系统架构](当前状态/系统架构.md) — active
- [项目概览](当前状态/项目概览.md) — active

- [工程决策目录](决策/MOC_决策.md) — 4 页，按主题或模块检索。
- [稳定知识目录](知识/MOC_知识.md) — 1 页，按主题或模块检索。

<!-- wiki-memory:auto:end -->
