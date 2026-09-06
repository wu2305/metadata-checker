# M58.4 轻量图召回对比 Spike 实施计划

> 状态：**done（2026-08-23；deterministic spike）**
> 对应设计：[M58.4 设计](../specs/2026-08-23-m58-4-lightweight-graph-retrieval-spike-design.md)
> 分支：`auto/graph-rag-spike-a7c3`
> 工作位置：CNB 远端 `/tmp/metadata-checker-graph-rag-spike-a7c3`

## 1. 目标

用一轮可撤销、可归因的 spike 回答：

> 在同一批确定性种子和同一张 metadata-checker 类型图上，typed PPR 相比 seed-only 与
> unweighted N-hop 是否提高多跳候选的有效密度，并保持关键终点召回？

交付代码、fixture、确定性对比报告和结论，但不把原型接入公共查询命令。

## 2. 环境与分支纪律

- 本地工作区仅用于读取状态，不编辑、不格式化、不编译、不测试。
- durable change 只写入 CNB 隔离 worktree。
- 每个可验收阶段在远端提交并推送，提交格式遵守 `type: description`。
- 不清理 `/workspace` 中与本轮无关的历史 untracked 文件。
- 编译、测试和基准只在 CNB login shell 中执行。

## 3. 工作包

### WP0：治理与基线

交付 approved spec/plan、M58 journal 与 INDEX 登记；执行 `git diff --check` 后提交
`docs: approve lightweight graph retrieval spike`。

### WP1：typed PPR 内核

文件所有权：

- `src/graph_retrieval.rs`（新增）；
- `src/lib.rs`（只新增 module export）。

实现：

- `GraphRetrievalConfig`：restart、tolerance、max_iterations、top_k；
- `GraphSeed`、`RankedGraphNode`、`GraphRetrievalResult`；
- intent → edge forward/reverse weight 表；
- 稳定 node index 与 transition 构建；
- seed 归一化、dangling 回灌、L1 收敛、稳定排序；
- `anyhow::Result` 与 `.with_context()`。

禁止修改 scanner/parser/graph schema，不新增 crate，不绑定 redb 或具体 memory store，不在库中打印。

### WP2：确定性 fixture 与测试

文件所有权：

- `tests/fixtures/graph_retrieval_spike_cases.json`（新增）；
- `tests/graph_retrieval_spike_tests.rs`（新增）；
- unweighted-hop baseline 仅放 test support。

覆盖：

- writer alias/action 多跳和结构噪声；
- availability DataFlow lineage；
- action/display 与高扇出结构节点；
- dangling mass；
- 并列分数按 node id 稳定排序；
- seed 缺失或全零时报错；
- max-iteration 未收敛显式标记；
- 相同输入重复运行报告一致。

### WP3：三组对比与结果登记

每个 fixture 同时运行 `seed_only`、`unweighted_hop`、`typed_ppr`，记录：

- case / variant / top-k node ids；
- recall、precision、gold terminal recall；
- candidate count；
- PPR iterations / converged。

结果回填本计划与 M58 journal。若 no-go，保留失败 case 和原因，不调换 fixture、seed 或 top-k。

### WP4：聚焦验证

远端按影响面执行：

```bash
cargo fmt --check
cargo test --features cli-local --test graph_retrieval_spike_tests
cargo test --lib graph_retrieval
cargo check --no-default-features --features browser-wasm --target wasm32-unknown-unknown
git diff --check
```

只有出现跨模块回归证据时才扩大到全量 `cargo test`。

### WP5：独立审查与交付

1. 验证后调用精确 `style_smell_reviewer` profile 只读审查；
2. 主线程复核并只修本轮 actionable finding；
3. durable fix 后重跑验证与 reviewer gate；
4. 推送最新提交；
5. 创建相对 `codex/m58-slm-eval-foundation` 的 PR。

## 4. 提交切片

| 顺序 | commit | 内容 |
|---|---|---|
| 1 | `docs: approve lightweight graph retrieval spike` | spec / plan / index / journal |
| 2 | `feat: add typed graph retrieval spike` | PPR 内核、fixture、测试 |
| 3 | `docs: record graph retrieval spike results` | 对比报告、go/no-go 与限制 |

## 5. 决策表

| 结果 | 决策 | 后续 |
|---|---|---|
| gold miss / recall 下降 | no-go | 保留 spike，检查边语义，不接产品链路 |
| recall 持平、precision 提升 | go | 设计 semantic seed provider 的独立 spec |
| 仅靠更大 top-k 才持平 | no-go | 不能声称减少图噪声 |
| 确定性 go、LLM paired 无增益 | 保留底层能力，不接默认查询 | 归因 prompt/tool surface |
| 确定性 go、LLM paired 有稳定增益 | 候选产品化 | 单独批准接入与性能计划 |

## 6. 风险与控制

| 风险 | 控制 |
|---|---|
| fixture 迎合权重 | case 先固定；失败不改 gold/top-k |
| PPR 分数被误当事实 | API 只返回候选；证据仍走现有 path |
| `Contains` 高扇出吞噬概率 | 结构边低权；fixture 放入真实形态噪声 |
| directed edge 查询方向错误 | forward/reverse 独立权重和专项测试 |
| 浮点导致 flaky 顺序 | score 后 node id tie-break |
| 小 fixture 夸大效果 | 结论限于 deterministic sufficiency |
| 新模块增大 WASM/二进制 | 零依赖和 browser-wasm compile check |

## 7. 实测结果

环境与身份：

- CNB workspace：`cnb-vf8-1k0m6rgn7`；
- 分支：`auto/graph-rag-spike-a7c3`；
- 基线：`aeefc45`；实现 commit：`567d904`；
- 正常 case：3（writer、availability/DataFlow、action/condition）；
- 模块单测：4（dangling mass、无有效 seed、迭代上限、Context/Auto 显式权重）；
- 另有稳定序列化和 score tie-break 集成断言。

聚合结果：

| variant | mean relevant recall@k | mean precision@k | mean gold terminal recall@k |
|---|---:|---:|---:|
| `seed_only` | 0.3056 | 1.0000 | 0.0000 |
| `unweighted_hop` | 0.6111 | 0.6111 | 0.0000 |
| `typed_ppr` | 1.0000 | 1.0000 | 1.0000 |

三例 typed PPR 都在 107 轮达到 `1e-10` 阈值；首跑的 100 轮上限不足，因此只把默认上限
调至 200，阈值、fixture、gold、top-k 和权重均未因结果改动。typed PPR 与无权 hop 的
截断前候选池同为 6/7/7；本轮证明的是相同 top-k 下有效密度提高，**没有证明候选池规模
下降**。

远端验证：

- `cargo fmt --check`：通过；
- `cargo test --features cli-local --test graph_retrieval_spike_tests`：3 passed；
- `cargo test --lib graph_retrieval`：4 passed；
- `cargo check --no-default-features --features browser-wasm --target wasm32-unknown-unknown`：通过；
- `git diff --check`：通过。

结论：**deterministic go**。typed PPR 达到设计中的确定性充分条件，但尚未接入产品查询，
也不能据此声称真实项目或 LLM 回答质量改善。后续只有在独立 spec 批准后，才能增加
semantic seed provider，并在 M58.2 使用同模型、同 case、同 trial 的 paired variants
验证 `tool_assisted_quality`。

## 8. 完成定义

- approved spec / plan / INDEX / journal 一致；
- typed PPR 和三组对照有正常、边界测试；
- 远端聚焦验证全部通过；
- 实测结果已记录，结论不越过证据；
- 远端提交已推送；
- 最新 durable state 通过 `style_smell_reviewer` gate；
- PR 只包含本 spike 相对基线的改动。
