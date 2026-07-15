# Shadow 文档

当前文档按现行设计、后端设计和历史归档分开维护。

- [总体设计](design.md)：Shadow v2 的核心模型、目录布局、状态机和命令语义。
- [火山引擎 TOS 后端](backends/volcengine-tos.md)：首个存储后端的配置、对象布局和实现计划。
- [旧版设计归档](archive/design-v1.md)：早期方案，仅供追溯。

设计约定：

1. `docs/design.md` 只描述当前准备实现的方案。
2. 后端特有内容放入 `docs/backends/`，不污染核心模型。
3. 已废弃但仍有参考价值的内容移动到 `docs/archive/`。
4. 重要且稳定的设计决策后续可以独立记录为 ADR。
