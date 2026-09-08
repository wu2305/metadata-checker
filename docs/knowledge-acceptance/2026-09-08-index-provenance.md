# 2026-09-08 索引生成记录复核

PR36 合并为 `3d595d3d61f1c926af8206ed160e04269bb1b9d5` 后，只读核对两个历史索引
任务和当前 H1 查询。本轮未重建知识库，未修改语料或生产提示词。

## 已确认的事实

| 观察 | 强制重建后的旧运行 | main 合并后的日常更新 |
|---|---|---|
| build / pipeline | `cnb-vud-1k1v5efku-001` | `cnb-saf-1k1v6ngg3-009` |
| 事件 | `api_trigger_knowledge_acceptance` | `push` |
| 配置 | 一次性配置 `forceRebuild: true` | `.cnb.yml` 为 `forceRebuild: false` |
| 索引源 SHA | `58d725be5edf9f908d3381483881e8a7505a47ba` | `f910fdc3eeaaf6439a1ff913a27857829d4264ba` |
| 完成后观测到的索引 ID | `2097114574376218624` | `2097120212368007168` |
| 插件报告处理 | 5 文档、45 chunks，全部成功 | 5 文档、45 chunks，全部成功 |
| AGENTS 顺序 / chunk 数 | 第 5 个文档 / 7 | 第 2 个文档 / 7 |

两次日志中的镜像摘要相同：
`sha256:266426fe8beebec9dedada7ced683127209af1ef4e3022302c18ed28ad4a9af6`。
每篇 chunk 数均为 SKILL 10、AGENTS 7、scan 13、CLI 13、README 2；不能从这些
计数推导每个 chunk 的内容和检索可见性完全相同。

旧任务明确记录 `force_rebuild 参数为 True`，随后删除、创建知识库。main 任务先读取
旧索引信息，随后也记录“完整更新”“重建知识库”“删除知识库”“创建知识库”，再处理
全部文档。因此 **`forceRebuild: false` 在这次运行中没有阻止删除重建**。
官方 [知识库文档](https://docs.cnb.cool/zh/ai/knowledge-base.md) 说明 true 会删除重建，
没有承诺 false 必然增量；日志也没有说明此次选择重建的具体条件。

本轮当前索引仍为上述 main 更新生成的 ID/SHA，PR36 没改入库文件。H1 原问再次请求
top-k=5，top1 为 `AGENTS.md` position 5，score `0.9806919097900391`，包含
预置目录免 token、未预置时缺配置失败及 preflight 的条件规则，与 PR36 的三次结果相同。
本轮没有调用回答模型，不能把这次命中记作 H1 问答通过。

## 尚不能归因的部分

- 原记录中 H1 未召回 AGENTS，而同语料的新索引可以；但两次索引实例、文档上传顺序和
  查询时间均不同，不能单独归因于 `forceRebuild`、异步可见性或检索排序。
- 旧日志确实写了 AGENTS 7 个 chunk 上传成功；没有逐 chunk 的查询就绪时间，
  也没有旧索引在稍后时刻的相同查询，无法排除临时不可见或服务端排序差异。
- 两次源 SHA 不同且经历 squash merge；这只是值得核查的更新路径差异，
  没有插件实现证据证明“非祖先关系触发重建”，不能把猜测写成根因。

下一步需要插件重建分支的代码/服务证据，或在隔离仓库固定文档和镜像后记录上传顺序、
索引 ID、逐 chunk 可见性与延迟查询；不能用删除当前生产索引来补做这个对照。
保留现有语料和提示词，整库最近一次完整回归仍为 44/5/3，尚未通过验收。

## 合并后 CI 的另一个未闭环项

main 构建 `cnb-flo-1k1vcd0tr` 的五条 full-criterion 流水线（004–008）均在
`checkout real project fixture` 阶段失败。逐条日志均已进入 clone，返回
`Repository Not Found` 和 exit 1；不是 `REAL_PROJECT_FIXTURE_DEPLOY_TOKEN is required`
那个缺值分支。后续 preflight/benchmark 被跳过，不计性能通过。

这能证明当前构建凭证未能读取配置的 fixture URL，不能单独证明仓库不存在或精确定位
为某一种权限错误。需要核对该流水线凭证对目标仓库的读取授权及 URL；未读取凭证值，
未改动授权，也未将失败改成跳过。PR36 的 PR 检查成功与 main 性能任务失败是不同范围。

## 证据

[归档](runs/2026-09-08-index-provenance.json.gz) 保存两个知识库 stage 响应、当前索引
元数据、H1 原查询和完整返回。stage 响应以 `...` 开头，本报告只引用其中明确存在的
处理信息，不将其称作完整 runner 日志或完整上传/可见性追踪。

- SHA-256：`485364a3f8a7e79671da9595f4a164be68b24b13fdacbb63fca1aa4636f38c08`。
- 旧、新问答完整产物仍见 [state-paths](2026-09-08-state-paths.md) 与
  [fixed-controls](2026-09-08-fixed-controls.md)，原失败和判定不变。
- fixture 失败只记录不含凭证的错误摘要；完整日志留在 CNB 构建详情中。
