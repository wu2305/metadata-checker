# M59-2 项目绑定准备阶段修正

状态：approved。2026-09-21 用户在审查后授权“你来直接调整”。

1. 项目准备格式使用 project_binding_schema_version，不能写 ownership_schema_version。
   旧实验 ownership marker 没有来源账本证明，拒绝加载，要求在新路径从源重建。
   将来启用真实账本必须使用能区别于旧实验 ownership=1 的新版本；准备库也须从源重建，
   不得仅补账本 marker 或把既有派生图当成可撤销贡献。
2. 只有准备版本与绑定都缺失、无 ownership marker 且事实库确实为空时才能初始化。
   单键缺失、未知版本和不匹配绑定一律拒绝，不修补、不重置。
3. 打开时在同一图锁内校验并 hydrate，句柄保存绑定；提交在写事务前及事务中复核。
   未绑定 open、readonly、scanner、shadow 读写不能访问绑定库；旧句柄不能在库被绑定后提交。
4. session 暂回已发布扫描栈。当前项目准备 API 仅供明确调用者使用；真实来源账本、
   按来源撤销、B5 和所有生产查询入口配套之前不得再次启用。

验收：标记状态矩阵、所有公开图入口、旧句柄提交、绑定重启与只读、准备阶段无 ownership marker。
测试只在 CNB 执行。Baseline impact: no performance claim；增加每次提交项目状态校验，真实性能未测。
