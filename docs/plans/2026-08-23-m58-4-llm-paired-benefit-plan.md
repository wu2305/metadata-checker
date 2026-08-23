# M58.4 Phase B：LLM 回答收益配对验证实施计划

> 状态：**approved（用户确认 2026-08-23）**
> 对应设计：[Phase B 设计](../specs/2026-08-23-m58-4-llm-paired-benefit-design.md)
> 前置结果：[deterministic spike](2026-08-23-m58-4-lightweight-graph-retrieval-spike-plan.md)
> 分支：`auto/graph-rag-spike-a7c3`
> 执行位置：CNB 远端开发环境与 CNB Pipeline

## 1. 目标与完成定义

在同一 Pipeline 中运行 6 case × 2 variant × 3 trial 的配对实验，回答 typed PPR
候选是否改善廉价模型最终回答，而不是只改善离线召回指标。

完成需要同时满足：

- baseline 输出契约保持不变，typed treatment 可由实际 transcript 证明已曝光；
- 36 条 agent record 与 36 条 judge record 完整且键唯一；
- 报告含按 variant 固定分母的四项指标和 18 对转换统计；
- token 未进入日志、record、报告或归档；
- 远端聚焦测试、WASM check、Pipeline 校验和 live build 均有记录；
- 结论严格落在 positive signal / no observed gain / inconclusive 三类之一；
- 最新 durable state 通过 `style_smell_reviewer` gate。

## 2. 分支与环境纪律

- 本地只允许查看 git/CNB 状态，不编辑、不编译、不执行 live 请求。
- 源码、文档、`.cnb.yml`、提交与推送都在 CNB 分支完成。
- Rust 编译和测试只在 CNB login shell 的 `/workspace` 执行。
- live 测试只使用 CNB Pipeline 已注入的环境变量；不得读取或输出 token 值。
- 每个可验收工作包提交一次，失败结果也原样保留，不改 case/gold/rubric 追分。

## 3. 工作包

### WP0：治理与基线

文件：

- `docs/specs/2026-08-23-m58-4-llm-paired-benefit-design.md`；
- `docs/plans/2026-08-23-m58-4-llm-paired-benefit-plan.md`；
- `docs/milestones/INDEX.md`；
- `docs/milestones/ai-eval/m58-cheap-model-comprehension-eval.md`。

登记 Phase B 为 active，并冻结 6 × 2 × 3 网格、处理差异、指标和判定，不改已有 Phase A
结果。

### WP1：runtime treatment 接线

文件：

- `src/graph_retrieval.rs`；
- `src/explain.rs`；
- `src/runtime.rs`；
- 相关聚焦测试。

实现：

1. 增加可纯函数解析的 `GraphRetrievalStrategy`，只接受 baseline / typed-ppr；
2. 既有 `build_explain_condition_output_with_intent` 继续走 baseline；
3. 新入口在 typed 模式以 target 为 seed 调用 typed PPR；
4. 非 seed top-ranked 节点只作为 `PathQuery.bridge_anchors`；
5. typed 输出增加自描述的 `details.experimental_graph_retrieval`；
6. runtime 只在 `cli-local` 读取实验变量，非法值返回带上下文错误。

不得改 graph schema、scanner、持久化或 browser-wasm；不得把候选分数当事实。

### WP2：treatment exposure 契约

文件：

- `.cnb.yml` record emitter；
- `tests/support/kimi_smoke_judge.rs`；
- `tests/kimi_harness_judge_tests.rs`。

为 `TrialRecord` 增加必填布尔字段 `retrieval_context_observed`。emitter 只根据真实
tool result 中的 `experimental_graph_retrieval` 标记曝光。baseline 曝光立即判为
实验污染；typed 未曝光保留在固定分母中并作为诊断。

### WP3：variant 与配对报告

扩展 judge 报告：

- baseline / typed_ppr 的 total、answer quality、tool adherence、
  tool-assisted quality、retrieval exposure；
- 以 `(case_id, trial)` 为键的 fail→pass、pass→fail、pass→pass、fail→fail；
- 缺失、重复、未知 variant、污染或 infra 时显式报告 inconclusive。

聚焦测试覆盖完整配对、转换方向、未曝光、污染、缺对和固定分母。

### WP4：CNB 36-trial 网格

在现有 `api_trigger_kimi_harness_smoke` 路径内：

- 一次安装 kimi、一次构建 binary、一次准备 SKILL/真实语料；
- trial 1/3 按 baseline→typed，trial 2 按 typed→baseline；
- baseline 清除实验变量，typed 设置 `typed-ppr`；
- 每个 variant/case 创建独立 kimi context；
- record 精确数量守卫从 6 调整为 36；
- run manifest 记录 `variants=[baseline,typed_ppr]`、`trials=3` 和 treatment 变量名；
- 继续复用既有 transcript/stderr 脱敏与归档逻辑。

不复制第二套 Pipeline，不为 variant 改模型、问题、超时、SKILL 或 judge。

### WP5：远端聚焦验证

在 CNB `/workspace` 执行：

```bash
cargo fmt --check
cargo test --features cli-local --test graph_retrieval_spike_tests
cargo test --features cli-local --test kimi_harness_judge_tests
cargo check --no-default-features --features browser-wasm --target wasm32-unknown-unknown
scripts/cnb-smoke-dry-run.sh
git diff --check
```

对 `.cnb.yml` 额外执行：

- YAML 语法检查；
- CNB 语义检查；
- `cnb-pipeline` Schema validator。

出现具体跨模块回归证据时才扩大测试范围。

### WP6：live paired run 与结论

1. 推送当前 commit；
2. 从该分支触发 `api_trigger_kimi_harness_smoke`；
3. 监控所有 stages，提取 run / records / judge / attachments；
4. 验证 36/36 完整性、18 对唯一性和 token 扫描结果；
5. 将 build SN、commit、模型/fixture/corpus 身份、分组指标、配对转换和限制回填 journal/plan；
6. 提交、推送并完成最终 reviewer gate。

## 4. 提交切片

| 顺序 | commit | 内容 |
|---|---|---|
| 1 | `docs: approve llm graph retrieval paired eval` | Phase B spec / plan / INDEX / journal |
