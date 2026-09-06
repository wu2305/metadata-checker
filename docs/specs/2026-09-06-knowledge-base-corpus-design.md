# 知识库语料与索引设计

> 状态：**draft**（待用户批准后转 `approved`；本文对应实施在 `docs/knowledge/` 与 `.cnb.yml`）

## 背景

Issue #30 讨论是否开启 CNB 知识库。仓库 `docs/` 下 253 个跟踪文件中 `archive/` 46 个、
`ai-eval-runs/` 115 个，默认全量 Markdown 索引 + 默认开启的 Issue 同步会把已归档结论、
评测答案键与当前事实混在一起召回。Codex 复审给出 8 项雷点，本设计是其落地约束。

## 决策

1. **不索引源码。** CNB `knowledge:update` 支持 `.md/.mdx/.docx/.txt/.pdf`，不支持 `.rs`。
   源码知识以「人工撰写、附源码行号与 SHA 的主题文档」形式入库。不使用 CodeWiki
   （其 `knowledge_enabled` 默认 `false`，且不产出源码依据）。
2. **白名单索引。** 首版只索引 `docs/knowledge/**/*.md`、`AGENTS.md`、`SKILL.md`。
   `docs/ai-eval-runs/**`（含评测答案键）、`docs/archive/**`、根目录 24 个迁移 stub 全部排除。
3. **关闭 Issue 同步。** `issueSyncEnabled: false`，避免 Issue 讨论进语料。
4. **处理失败报错。** `ignoreProcessFailures: false`，不让「入库成功」掩盖「部分文档处理失败」。
5. **不无条件 `forceRebuild`。** 保持 `false`；仅在白名单结构变更时手动触发一次。

## 语料规范（写入 `docs/knowledge/` 的文档必须满足）

| 约束 | 理由 |
|------|------|
| 每篇分「当前实现 / 已批准计划 / 已知缺陷 / 历史状态」四节 | 防止把 plan 写成实现、把测试通过写成缺陷已修复 |
| 每条调用链结论带 `src/xxx.rs:NNN` + 分析 SHA | 分块后仍可回溯，避免无据推断 |
| 小节自包含（自带状态、适用版本、限制） | `chunkOverlap` 与分块边界会切断上下文，限定语不能与描述分离 |
| 否定结论写在描述同一小节内 | 同上 |
| 不含真实凭据、内网域名、本机绝对路径 | 仓库为 Public |

## 首版范围

- `docs/knowledge/topic-cli-stdio-to-query.md`：CLI/stdio → `GraphRuntime::query` 调用链
- `docs/knowledge/topic-scan-incremental-persist.md`：扫描 → 增量索引 → 持久化生命周期
- `docs/knowledge/knowledge-acceptance-questions.md`：约 20 条验收问题（含 7 条反误导题）

## 验收

合并并构建完成后，用**空上下文 agent** 逐题查询 `docs/knowledge/knowledge-acceptance-questions.md`：
- 反误导题（⚠C1–C7）**全部答对**才可考虑扩展主题；任一答错先修条目再重测。
- 边界题（D1–D3）必须答「未知」并停止推断；编造答案说明条目边界声明不足。

## 索引配置

`.cnb.yml` 新增 `.knowledge_base_ci`，挂在 `main.push`（`ifModify` 只覆盖
`docs/knowledge/**`、`AGENTS.md`、`SKILL.md`），不在分支 push 与 PR 上运行，
避免往已经很重的 main 链上无条件叠加任务。
