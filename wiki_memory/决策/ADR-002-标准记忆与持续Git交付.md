---
type: decision
status: active
updated: 2026-11-09
topic: project-memory-git-delivery
sources:
  - "用户指令：2026-11-09 明确要求标准工程记忆，并在以后每次完整对话修改后提交推送 https://github.com/DewMysimple/FluxDL.git"
  - "AGENTS.md"
  - "wiki_memory/.memory.json"
  - ".gitignore"
---

# ADR-002 标准记忆与持续 Git 交付

## 背景与选择者

用户于 2026-11-09 明确选择 standard 工程记忆，并指定 FluxDL 的 GitHub 远程和持续提交推送要求。Agent 将要求落实到根 `AGENTS.md` 与项目自包含协议，不依赖个人 Skill 自动触发或全局自动化。

## 选择与实施

唯一记忆根 `wiki_memory/`：README 导航、AGENTS 协议、五页当前状态、必要决策与知识、一份日志 MOC 和六类日志目录。采用 schema 2、工具 3.4.0、相对 Markdown 导航，工具副本放本仓库。

Git `origin` 指向 `https://github.com/DewMysimple/FluxDL.git`，主分支 `main`。每次完整对话有工程修改时，先核实来源、检查与同步记忆，再提交并推送，最后核对实际远程 SHA。纯只读或无变化时不造空提交。持续授权可被用户撤销或改变。

## 取舍与边界

记忆随源码转移而独立可用；需要维护成本，因此只同步有持久价值的变化，不复刻对话、不保存凭证与个人信息。原始本机报告和产物忽略，精选截图、许可及脱敏验证证据跟踪。

Git 规则是项目协作协议，不是后台自动推送服务或强制推送钩子。遵守它的后续会话已有提交推送授权；遇认证、远程分叉等真实问题需保全修改并如实记录，不通过改写历史掩盖失败。

## 核实范围

2026-11-09 实际完成 init 预览与应用，核实根接入块、配置和远程地址；此页记录用户选择与规则，不替代 Git 提交、push 输出或远程引用的实时证据。初始化验收和交付过程见对应维护日志。
