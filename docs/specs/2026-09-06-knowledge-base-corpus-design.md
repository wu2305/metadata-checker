# 知识库语料与索引设计

> 状态：**approved**（2026-09-07 用户授权“修复这些问题，在 PR33 中一并处理”；范围为现有知识库修复与验收）

## 背景

Issue #30 讨论是否开启 CNB 知识库。仓库 `docs/` 下 253 个跟踪文件中 `archive/` 46 个、
`ai-eval-runs/` 115 个，默认全量 Markdown 索引 + 默认开启的 Issue 同步会把已归档结论、
评测答案键与当前事实混在一起召回。Codex 复审给出 8 项雷点，本设计是其落地约束。

## 决策

1. **不索引源码。** CNB `knowledge:update` 支持 `.md/.mdx/.docx/.txt/.pdf`，不支持 `.rs`。
   源码知识以「人工撰写、附源码行号与 SHA 的主题文档」形式入库。不使用 CodeWiki
   （其 `knowledge_enabled` 默认 `false`，且不产出源码依据）。
2. **白名单索引。** 首版只索引 `docs/knowledge/**/*.md`、`AGENTS.md`、`SKILL.md`。
   `docs/ai-eval-runs/**`（含评测答案键）、`docs/archive/**`、`docs/knowledge-acceptance/**`
   （验收问题与答案键）、根目录 24 个迁移 stub 全部排除。
3. **关闭 Issue 同步。** `issueSyncEnabled: false`，避免 Issue 讨论进语料。
4. **处理失败报错。** `ignoreProcessFailures: false`，不让「入库成功」掩盖「部分文档处理失败」。
5. **不无条件 `forceRebuild`。** 保持 `false`；白名单结构变更或确认索引陈旧时，固定 SHA 手动触发一次。
   临时触发方式（不落库到 `.cnb.yml`）：用 `--config` 传一份只含 `knowledge-base`
   流水线的临时 YAML，走 `api_trigger_*` 事件触发，例如
   `cnb build start-build --repo <slug> --branch <b> --event api_trigger_kb_force_rebuild \
     --config "$(cat /tmp/kb_rebuild.yml)"`。
   2026-09-06 第 2 轮验收即用此法做了 4 次重建。

## 语料规范（写入 `docs/knowledge/` 的文档必须满足）

| 约束 | 理由 |
|------|------|
| 每篇分「当前实现 / 已批准计划 / 已知缺陷 / 历史状态」四节 | 防止把 plan 写成实现、把测试通过写成缺陷已修复 |
| 每条调用链结论带 `src/xxx.rs:NNN` + 分析 SHA | 分块后仍可回溯，避免无据推断 |
| 小节自包含（自带状态、适用版本、限制） | `chunkOverlap` 与分块边界会切断上下文，限定语不能与描述分离 |
| 否定结论写在描述同一小节内 | 同上 |
| 关键结论另配**显式问答**（问 X 吗？答：不是/没有） | 第 2 轮实测：限定语散在描述句里时，空上下文 agent 会答「未知」而不敢下结论 |
| 高频问题在**文首**放一块「检索速查」 | 文首分块最容易被 top-k 命中；放正文中间会被 `chunkSize` 切断 |
| 表格列数压到最少（原表 ≥5 列时合并列） | 第 2 轮实测：五列六行表格被切成两半，表头与后半行分离 |
| 不含真实凭据、内网域名、本机绝对路径 | 仓库为 Public |

## 首版范围

- `docs/knowledge/topic-cli-stdio-to-query.md`：CLI/stdio → `GraphRuntime::query` 调用链
- `docs/knowledge/topic-scan-incremental-persist.md`：扫描 → 增量索引 → 持久化生命周期
- `docs/knowledge-acceptance/knowledge-acceptance-questions.md`：24 条验收问题（含 8 条反误导题）

**验收集不入库。** 初版把问题集放在被索引的 `docs/knowledge/` 内，导致 top1 直接命中
答案表（B 类题目尤其明显）——检索命中被当成了 agent 回答正确。2026-09-06 复审（Issue #30）
后把它移到 `docs/knowledge-acceptance/`，并在 `.knowledge_base_ci` 的 `exclude` 里加兜底。

## 验收

用**空上下文 agent** 逐题查询，查询串用**原始问法**，不得并进「期望答案要点」的原词：
- 每题必须记录 top chunk（文件 / position / 分数）**与 agent 原话回答**；只记命中不算通过。
- 反误导题（⚠C1–C8）**全部答对**才可考虑扩展主题；任一答错先修条目再重测。
- 边界题（D1–D3）必须答「未知 / 未在知识库中」并停止推断；编造答案说明条目边界声明不足。

第 1 轮结果（构建 `23feea9`）**已作废**：6 chunk / 71913 B 与白名单字节数一致属实，
但问题集当时在索引内，且只记录了召回片段、没有 agent 原话，无法证明「无误导」「能停止推断」。
另：记录称 20 题，实际表格 23 题（现为 24 题，新增 ⚠C8 覆盖 v1 hydrate 部分缺失）。
D3 的本机路径风险已补声明；历史回答仍含“唯一注入方式”的错误，PR33 本次另行纠正。

### 第 2 轮历史自报（2026-09-06，构建 `b024815`）：**待独立核验**

入库核对：count=5 / 71833 B，`last_commit_sha=b024815…`，`include` 不含验收集目录。
原执行者报告使用空上下文 `deepseek-v4-flash`，留存了 24 题回答摘录但缺完整请求与响应。以下仅保留其自报结果：

- A 类 6/6 对、B 类 7/7 对、⚠C 类 **8/8 对**、D 类 3/3 正确停止推断
- 复验通过 D3（`AGENTS.md` 的「不得假设路径可用」声明生效）与 ⚠C8（PR #33 新增条目）

**原执行者归纳的三类条目写法问题**（未独立排除检索和模型因素）：

1. **链路被分块切断**：A1/A3 只答出 `run_surface`，答不出统一终点 `GraphRuntime::query`
   ——因为 §1.2 与 §1.4 分属不同 chunk。修法：主题一文首加「§1.0 完整链路速查」自包含小节。
2. **表格被 `chunkSize:1200` 劈开**：B1 只答出六阶段的前两个。修法：六阶段速查上移到
   文首「## 0. 检索速查」，并把原五列表格压成四列。
3. **限定语散在描述句里，模型不敢据此下结论**：⚠C5 / ⚠C6 / ⚠C7 出现「片段明明有内容，
   但 agent 答未知」。修法：把缺陷/历史状态条目改成**显式问答**（问 X 吗？答：不是/没有）。

历史运行曾观察到 top-k 改变召回集合；本次固定 top-k=5 便于比较。
这不是平台行为的完整实验，不能断言所有 top-k=1 漏召回均是假阴性。

## 索引配置

`.cnb.yml` 新增 `.knowledge_base_ci`，挂在 `main.push`（`ifModify` 只覆盖
`docs/knowledge/**`、`AGENTS.md`、`SKILL.md`），不在分支 push 与 PR 上运行，
避免往已经很重的 main 链上无条件叠加任务。

## PR33 补充验收约束（2026-09-07）

历史第 2 轮是按固定题反复修文档后的自报结果，未保存完整请求与响应，
不能作为独立 24/24 或泛化验收。现增加 24 道原题和 8 道预先冻结的改写/新题，
通过 `api_trigger_knowledge_acceptance` 保存 top-k 原文、完整消息、流式响应、
模型身份、索引前后状态与题集 hash；逐题人工核验，模型请求不包含答案键。
本次还纠正 CI 缺 token 的知识、将缓存探测改为非破坏性检查，并覆盖失败退出路径。
