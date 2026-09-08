# PR34 追加修复：C6 召回与 B3/C7/B5/H1 边界

上轮 [35 pass / 2 partial / 3 fail](2026-09-07-boundary-regression.md) 的记录保持不变。
本轮处理上次复审指出的问题：**C6 召回不到正确条目**、**B3/C7 出现无据补充结论**、
**B5/H1 前提不精确**。

## 修复范围（只改被索引语料，不改 Rust / 评测器 / 提示词 / top-k）

| 题 | 根因 | 改动 |
|---|---|---|
| C6 | human 旧路径缺陷写在 §3 正文，该分块与查询串词面距离远，top5 全落在调用链分块；正确条目存在但召不回 | `topic-cli-stdio-to-query.md` 新增 `## 0. 检索速查`，把该问答放在文首（与主题二 §0 同款做法），并保留 §3 原文 |
| B3 | 条目只说了坏 TBL / 坏 SPG 在扫描层的行为，模型却去比较另一篇文档的「失败后怎样」表并虚构冲突 | 主题二 1.6 补显式问答（CLI/stdio 层**未知**）+ 主题一 §5 补「不得跨层外推」规则 |
| C7 | 条目没写 M28 包含哪些缺陷，模型却断言 human 缺陷与 M28 无关 | 主题一 §4 补「M28 与 human 缺陷的关系知识库未记录」，§5 补「不得推断里程碑归属」 |
| B5 | 条目把 full rebuild 置 `Current` 与阈值重建混在同一句，被读成两种「回到 Current」的触发条件 | 主题二 1.3 拆成「迁移 / 初始写入 / 保持」三类并加显式问答 |
| H1 | AGENTS.md 先讲缺 token 失败、后讲预置目录例外，模型首句无条件断言 | AGENTS.md 把「预置目录时不需要 token」提到最前，并单列一条「缺 token 就失败只在未预置时成立」 |

源码核对：`--query-page-logic` 的 `args.is_human()` 分叉在 `src/main.rs:1198-1226`
（human 走 `query::query_page_logic`，non-human 走 `run_cli_runtime_tool`）；
v2 shadow 的阈值重建与 checkpoint-only 保持逻辑在 `src/graph_redb.rs:1163-1200`。

## 冻结题集（提问前冻结，但仍是开发回归）

新增 [八道题](questions-boundaries-2.json)（F1–F8），针对本轮四项修复的不同问法：
F1/F2 检验 B5 的迁移 vs 初始写入 vs 保持；F3/F4 检验 B3 的跨层边界；
F5/F6 检验 C7 的里程碑归属未知；F7/F8 检验 H1 的条件限定。
八题在首次执行前冻结，但由已知失败推导而来，因此**全部标记为 `boundary_regression`**，
连同原 40 题一并只能作为开发回归证据，**不是验收，也不证明泛化能力**。

## 复现

1. 提交并推送文档修改：SHA `c3feafaa3fa53d79fd5bee9e4d9c635e18c2d9d2`。
2. 用一次性 `--config`（[归档](runs/2026-09-07-boundary-regression-2.cnb.yml)）
   触发 `api_trigger_knowledge_acceptance`；归档配置只包含测试和问答，不能据此重放重建步骤。
   本次原始证据证明运行时索引 SHA 匹配，但未在该配置中保留重建操作；随后运行
   16 项脚本测试与 48 题独立问答。日常 `.cnb.yml` 仍为 `forceRebuild: false`。
3. `kb-evaluate.py` 只收 system prompt + 本题 + top-k=5 片段，不收答案键，无历史消息。
   构建：[cnb-60g-1k1udh6o3](https://cnb.cool/wu2305/metadata-checker/-/build/logs/cnb-60g-1k1udh6o3)。
4. 用 `cnb build build-runner-download-log --pipelineId cnb-60g-1k1udh6o3-001` 取完整日志，
   再 `scripts/kb-extract-evidence.py` 校验并解压（stage 详情接口会截断 >100 KiB 日志）。
5. 人工逐题判定，产物：
   [原始证据](runs/2026-09-07-boundary-regression-2.json.gz)（SHA256 `b751a45c…`）、
   [逐题判定](runs/2026-09-07-boundary-regression-2.judgments.json)、
   [完整性清单](runs/2026-09-07-boundary-regression-2.integrity.json)。
   完整性核验覆盖 48 题 / 240 片段 / 48 个双消息请求，索引运行前后均为 `c3feafa`，
   五个语料文件 blob 与报告提交一致，模型返回 `deepseek-v4-flash`。

## 结果

原报告自报 **48 pass / 0 partial / 0 fail**，2026-09-08 源码复审撤回该结论。
更正为 **44 pass / 1 partial / 3 fail**；[首次评分](runs/2026-09-07-boundary-regression-2.initial-judgments.json)
与原始回答保留，更正后的逐题判定仍在原链接。

- **B5/F1 fail**：新增条目错误限定只有阈值重建能使 Stale 回到 Current；回答照搬此错误。
  现有 Stale 库的有效 `delta=None` 全量持久化同样可以置 Current。
- **E3 fail**：回答末段将布尔 `check_reload=true` 错说成进入 runtime 的 CheckReload 命令分支；
  stdio 构造请求时保留原 command，设 check_reload=false（`src/stdio_server.rs:451-459`）。
- **H1 partial**：开头仍无条件说缺 token 失败，后面才加例外；沿用上轮同样的评分标准。

其余原记录中的局部观察：

- **C6**：top1 变为 `topic-cli-stdio-to-query.md` position 0（0.96），即新增的文首速查分块，
  回答给出 human 走 `query::query_page_logic`（`src/main.rs:1200`）、non-human 走 runtime。
- **B3 / F3 / F4**：主结论保留，不再虚构文档冲突，跨层问题答「未知」。
- **C7 / F5 / F6**：M28 未做的结论正确，里程碑归属答未知。
- **F2**：针对 `delta=Some` 的 checkpoint-only 回答保留状态；不能推广到 `delta=None`。
- **F8**：正确在结论中限定缺 token 失败的成立条件；H1 不应跟随它计为通过。

## 结论与限制

- 本轮**不宣布知识库验收通过，也不声称具备泛化能力**。48 题全部参与过语料修正，
  按 [术语口径](README.md) 一律是**开发回归**，更正结果不能外推到未见过的问题。
- 两轮分数差异的原因**不能全部归因于本轮条目改动**：题集、语料与模型输出
  都可能波动，也没有做提示词或模型对照实验。
- 后续若要谈验收，需要**提问前冻结且从未用于修语料**的外部新题、跨模型/提示词对照，
  并保留逐题完整证据；不得继续把固定题调到全绿当作通过。
- 本轮未改 Rust 实现、评测器、system prompt 与日常索引配置；未在本地运行 Cargo。
  远端 16 项脚本测试在 `cnb-60g-1k1udh6o3` 中通过；PR 检查以 CNB Checks 为准，
  检查成功不代表问答质量通过。
