# M58.4 Phase B：LLM 回答收益配对验证设计

> 状态：**approved（用户确认 2026-08-23；live 验证完成 2026-08-23）**
> 类型：live paired evaluation
> 依赖：[M58.4 deterministic spike](2026-08-23-m58-4-lightweight-graph-retrieval-spike-design.md)
> 执行环境：CNB Pipeline；token 只从远端环境变量读取

## 1. 验证问题

Phase A 只证明 typed PPR 在三个确定性小图上能提高同 top-k 的有效密度。它没有回答：

> 廉价模型在真实项目问题中，实际拿到 typed-PPR 增强后的 metadata-checker 证据时，
> 最终回答的 tool-assisted quality 是否比现有 baseline 更高？

本阶段只测这个增量，不重做模型排行榜，也不把一次小样本结果解释为生产收益。

## 2. 因果单位

固定网格为：

- case：当前 `xiaoshouyi_large_real_cases.json` 中 6 个 smoke-enabled case；
- variant：`baseline`、`typed_ppr`；
- trial：每个 case / variant 各 3 次独立 kimi 上下文；
- 总 agent trials：`6 × 2 × 3 = 36`；
- judge：每条 transcript 独立判分，共 36 条。

同一次 Pipeline 内固定：

- repo commit、真实语料 SHA、fixture/gold SHA；
- metadata-checker binary 与 SKILL.md；
- kimi-code 安装版本、agent model、judge model；
- 问题文本、超时、输出格式和判分规则。

唯一处理差异是 metadata-checker cli-local runtime 的实验检索模式。variant 顺序按 trial
交错，避免每轮都由 baseline 固定先跑。

## 3. Treatment 定义

### 3.1 baseline

- 不设置实验检索环境变量；
- 调用现有 `build_explain_condition_output_with_intent`；
- JSON 字段、路径选择和既有测试保持不变。

### 3.2 typed_ppr

Pipeline 设置：

```text
METADATA_CHECKER_EXPERIMENTAL_GRAPH_RETRIEVAL=typed-ppr
```

该开关只在 `cli-local` runtime 读取。ExplainCondition 处理时：

1. 以已解析 target node 为唯一 seed；
2. 复用 `TraversalIntent` 和现有 `GraphReadStore` 执行 typed PPR；
3. top-ranked 非 seed 节点作为 `PathQuery.bridge_anchors`，让既有 path finder 生成可证明路径；
4. 在 `details.experimental_graph_retrieval` 中附加候选排名、收敛信息和
   `evidence_role=candidate_only`；
5. 最终事实仍来自既有 path/evidence 与 answer_facts，PPR 分数不得成为事实。

开关不得影响 browser-wasm、持久化、graph schema 或未使用 ExplainCondition 的命令。

## 4. Treatment 曝光

variant 标签不等于模型真的看到了增强证据。每条 `records.jsonl` 新增：

```json
{"retrieval_context_observed": true}
```

仅当 transcript 的工具结果实际包含 `experimental_graph_retrieval` 时为 true。

约束：

- baseline 出现 true 属于实验污染，Pipeline 判红；
- typed_ppr 为 false 不判 infra，表示模型没有走到受处理的工具输出；
- 报告必须同时给出 typed_ppr 的 treatment exposure，不能把未曝光 trial 算成算法失败。

## 5. 指标

主指标按 variant 固定分母输出：

- `tool_assisted_quality`：`metadata_checker_invocations > 0 && semantic PASS`；
- `answer_quality`：所有非 INFRA trial 的语义通过率；
- `tool_adherence`：调用 metadata-checker 的比例；
- `retrieval_exposure`：实际观察到 typed-PPR 上下文的比例。

成对指标以同一 `(case_id, trial)` 的 baseline/typed_ppr 为一对：

- fail → pass；
- pass → fail；
- pass → pass；
- fail → fail；
- paired answer delta；
- paired tool-assisted delta。

不按是否调用工具或是否曝光筛除分母；曝光只作为归因诊断。

## 6. 判定

本轮只有三种结论：

- **positive signal**：typed_ppr 的 tool-assisted quality 高于 baseline，且
  fail→pass 多于 pass→fail；
- **no observed gain**：主指标不升，或正负转换相抵；
- **inconclusive**：record/judge 不完整、variant 污染、有效配对不足或出现 infra。

即使 positive signal，也只允许得出“这 6 个真实项目问题、当前模型版本、3-trial 网格中
存在 variant assignment 层的弱正向信号”。它不自动等价于 typed-PPR 的因果收益；若
改善 pair 没有实际看到 treatment，上升只能记为随机波动或未归因信号。

## 7. 实测结果

CNB build [`cnb-r2g-1k0mi4oqu`](https://cnb.cool/wu2305/metadata-checker/-/build/logs/cnb-r2g-1k0mi4oqu)
在 commit `4e288994a29db6c5e52531f2028187bca71343f8` 完成 36/36 agent trials、
36/36 judge calls 和 18/18 配对；无 INFRA、缺对或 baseline 污染。固定身份为：

- agent `deepseek-v4-flash`，judge `glm-5.2`，kimi-code `0.38.0`；
- corpus `6920ac514df2d79a3df73a6447cf851abdfc954b`；
- fixture SHA-256 `df86f2f35c170f699c325de556e36de0ed2dae7ce8cd94f98567f9fe49131d10`；
- run tag `20260823T054257Z`；records / run / judge 永久归档，脱敏 transcript 保留 90 天。

| variant | answer quality | tool adherence | tool-assisted quality | retrieval exposure |
|---|---:|---:|---:|---:|
| baseline | 10/18 | 15/18 | 10/18 | 0/18 |
| typed_ppr | 12/18 | 15/18 | 11/18 | 1/18 |

| outcome | fail→pass | pass→fail | pass→pass | fail→fail | paired delta |
|---|---:|---:|---:|---:|---:|
| answer | 2 | 0 | 10 | 6 | +2/18 |
| tool-assisted | 2 | 1 | 9 | 6 | +1/18 |

按第 6 节冻结规则，自动报告的 variant assignment 结论是 **positive signal**。但这不是
typed-PPR 收益成立：typed 组只有 `1/18` transcript 实际包含
`experimental_graph_retrieval`，唯一曝光 pair
`xiaoshouyi_text41_display_conditions/t3` 是 PASS→PASS；两个 answer fail→pass
都发生在未曝光 trial。两组 tool adherence 同为 `15/18`、raw fallback 同为 `12/18`，
也没有显示 treatment 改变了整体工具行为。

因此 M58.4 Phase B 的产品结论是：

- **variant assignment 有弱正向信号**，仅作小样本仪表；
- **typed-PPR 对 LLM 最终回答的因果收益仍未证明**，当前证据为 inconclusive；
- 当前瓶颈先是命令路由 / treatment 曝光，不是继续更换或堆叠图召回算法。

若继续，下一轮应先让需要条件多跳证据的 case 稳定走到受处理的 ExplainCondition 输出，
并在运行前冻结最低曝光门槛；在此之前不把 typed-PPR 接入默认查询路径。
