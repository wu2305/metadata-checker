# M19-FIX：可扩展主链路计算框架

| milestone | M19-FIX |
| status | done |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M19-FIX：可扩展主链路计算框架

**状态：主体实现已收敛，真实项目回归测试待验证。**

### 背景

M19 已接入页面级 prerequisites、`primary_paths`、`key_primary_paths` 与 `related_context`，但当前 `primary_paths` 仍偏向“高置信图边列表”，不是围绕页面数据可用性目标的因果路径。典型症状：

- `key_primary_paths` 容易被 `$user.id`、`model19.brand`、普通模型读取边占据。
- `合同协议.spg` 的关键链路 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- 潜客信息跟进.button1.action1/action4` 没有稳定进入 summary Top-N。
- 单一打分/排序容易掩盖问题，需要预留后续 PRA、Personalized PageRank、Steiner Tree 或学习型 selector 接口。

### 目标

把 `primary_paths` 从“边集合”重构为“候选路径 + 分类 + 可替换选择器”的主链路计算框架。当前阶段不把分数写死为唯一依据，而是输出可解释的路径候选、分类理由、选择理由和诊断信息。

### 任务清单

- 定义路径领域模型：`PathNodeRef`、`PathEdgeRef`、`PathSegment`、`PathCandidate`、`PathTerminal`、`PathQuery`、`PathSelectionResult`。
- 每条 `PathCandidate` 至少包含 `path_id`、`purpose`、`terminals`、`segments`、`evidence`、`selection_reason`、`rank_features`、`classification`、`confidence`、`diagnostics`。
- 区分路径分类：`primary_path`、`candidate_path`、`supporting_path`、`related_context`、`rejected_path`。
- 拆分路径发现与路径选择接口：
  - `PathFinder::find_candidates(&PathQuery) -> Vec<PathCandidate>`
  - `PathSelector::select(&PathQuery, Vec<PathCandidate>) -> PathSelectionResult`
- 先实现 `BoundedCausalPathFinder`，限制深度、边类型和页面作用域，避免全图扩散。
- 先实现 `RuleBasedPathSelector`，但预留 `WeightedPathSelector`、`NoScorePathSelector`、`DebugAllPathSelector`、`FutureLearningPathSelector`。
- 实现 `AnchorExtractor`，从页面条件、隐藏计算字段、物理表读取字段、跨页面 writer 中提取 `target_anchors`、`source_anchors`、`sink_anchors`、`bridge_anchors`、`excluded_anchors`。
- 将字段级物理表归并纳入路径计算，候选路径必须能表达页面局部模型字段、物理表字段、跨页写入 action 和原始 `json_path`。
- 禁止直接把所有 `Reads` / `ActionWrites` 塞进 `primary_paths`；候选路径必须至少连接两个 anchor，或连接 condition、组件、模型过滤、`totalRowCount__`、物理表 writer 等因果对象。
- 输出 `rank_features` 作为特征，不作为唯一真伪判断；至少包含 `contains_target_component`、`contains_physical_field`、`contains_cross_page_writer`、`field_name_match`、`same_page`、`has_condition_node`、`has_json_path`、`edge_type_sequence`、`path_length`、`source_file_count`、`contains_only_structural_edges`、`contains_dataflow_side_branch`。
- `key_primary_paths` 采用“分组保底”而不是简单 Top-K：
  - 至少保留 1 条 value source path。
  - 至少保留 1 条 data prerequisite path。
  - 至少保留 1 条 display gate path。
  - 至少保留 1 条 action/write source path。
  - 如果存在跨页 writer，必须保留至少 1 条 writer path。
  - 如果存在目标组件，例如 `input3`，必须优先保留包含该组件的路径。
- `related_context` 只承载相关但非必要的旁路关系；同名局部模型的其他页面引用、DataFlow 旁路、结构边和字段不匹配跨页边不得进入 `key_primary_paths`。
- 修复 `related_context_summary` 统计时机，保证 `summary.related_context_count == details.related_context_summary.total_count`。
- 支持调试输出层级：
  - compact：只输出 `key_primary_paths`、Top-N prerequisites 和计数。
  - normal：输出 `primary_paths`、`supporting_paths`、`related_context_summary`。
  - full：输出 `candidate_paths`、`rejected_paths`、`path_selection_diagnostics`。
- 更新 `docs/schema.md`，定义 `PathCandidate`、`PathSegment`、`PathTerminal`、`rank_features`、`classification`、`selection_reason` 和 `PathSelectionResult`。
- 更新 `SKILL.md`，要求条件/页面数据可用性问题优先读取 `summary.key_primary_paths`，只有诊断提示时再看 `candidate_paths` 或 `related_context`。

### 测试清单

- 增加 fixture：简单读链 `component -> local model -> physical model`。
- 增加 fixture：跨页 writer `other page action -> same physical field`。
- 增加 fixture：同名局部模型噪声，其他页面 `model22` 不同物理表或不同字段。
- 增加 fixture：DataFlow 旁路相关但不应进入 primary。
- 增加 fixture：`condition -> component -> model.filter -> totalRowCount__`。
- 增加真实项目回归：`合同协议.spg` 的 `key_primary_paths` 必须包含 `input3`、`model22.phoneNumber`、`fact_qwSidebar.phoneNumber`、`潜客信息跟进.spg|button1|action1` 或 `action4`。
- 增加负例断言：`key_primary_paths[0..5]` 不得全部被 `$user.id`、`model19.brand` 等普通读边占据。
- 增加负例断言：外页同名局部模型不得进入 `primary_path`，除非它写入同一物理字段。
- 增加一致性断言：`related_context_count` 与 `related_context_summary.total_count` 必须一致。
- 增加输出稳定性断言：compact 不展开大数组，normal 输出候选分类，full 输出 rejected paths。
- 全量验证必须通过 `cargo test`，且不得新增 Rust warning。

### 验收目标

- `--query-page-logic 'page:app/销售.app/销售/合同协议.spg' --budget compact` 的 `summary.key_primary_paths` 能优先呈现 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- 潜客信息跟进.button1.action1/action4`。
- 小模型只读 summary 能复述主链路，并能区分主链路、补充路径和相关但非必要的旁路关系。
- 路径选择策略可替换，不依赖写死的单一分数；后续可以接入 PRA、Personalized PageRank、Steiner Tree 或学习型 selector。
- `related_context` 不再污染 `primary_paths`，且其统计、诊断和升级建议一致。
