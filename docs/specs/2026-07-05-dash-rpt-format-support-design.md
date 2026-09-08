# Dashboard（`.dash`）/ Report（`.rpt`）格式支持设计

> 状态：**draft，未排期**。对应 [milestones/INDEX.md](../milestones/INDEX.md) 的「待排期 | Dashboard / Report 格式支持」一行。
> 原始记录日期：2026-07-05（brainstorming 阶段产出，从未开始实现）。
> 归档日期：2026-09-08。

## 本文来历与修订说明

原稿写于 2026-07-05，标题为「M56/M57 Dashboard 与 Report 元数据理解设计」，
存放在分支 `codex/m52-redb-v2-and-ci` 上，**从未合入 `main`**。该分支已于 2026-09-08
清理删除，原始提交保留在本地 tag `archive/m52-redb-v2-and-ci`（未推送远端）。

初次归档时做了以下三处修改；2026-09-08 复审另修正查询适配范围，见「解析方案」：

1. **去掉 M56 / M57 编号。** 原稿把 `.dash` 排为 M56、`.rpt` 排为 M57，但这两个号后来
   被改派给 diff-refresh 线（M56 = Diff refresh persist，M57 = Diff refresh 产品化，
   均已 done）。原稿的阶段划分改称「阶段一 / 阶段二」，不占用任何里程碑编号。
   INDEX 中该条目已明确「不再占用 M59；旧计划中的 M59 rpt/dash 是历史编号」。
2. **补一次事实复核。** 见下节；原稿对源码的判断在 `main` @ `be23369` 上逐条重验过。
3. **移除本机绝对路径。** 原稿引用了一份本地 BI 前端源码目录，按语料脱敏规则改为相对描述。

本文只作为**缺口记录**保留，不构成已批准计划。真要开工需另立里程碑、另写 plan。

## 缺口复核（`main` @ `be23369`，2026-09-08）

原稿声称的缺口今天**依然完全成立**，逐条核过：

| 原稿断言 | 复核结果 |
|---|---|
| `SourceKind` 只有 `Spg` / `Tbl` / `Unknown` | ✅ `src/source_id.rs:75-79`，三个变体，无 `Dash` / `Rpt` |
| scanner 目录遍历只收 `.spg` / `.tbl` | ✅ `src/scanner/indexer.rs:339`：`e == "spg" \|\| e == "tbl"` |
| 解析分支只认这两种 | ✅ `src/scanner/indexer.rs:453`（spg）/ `:465`（tbl），无第三分支 |

以下符号与目录在该版本存在；存在性核对不证明能直接适配新格式，复用前仍须验证输入语义：

- `normalize_dataflow_path`（`src/model_scope.rs:41`）
- `parse_expression_refs`（`src/superpage/expr_ast.rs:621`，纯函数）
- `process_tbl_file_from_string`（`src/scanner/tbl.rs:8`）
- `NodeType`（`src/graph.rs:11`）
- `explain/handlers/`（现有 `component_action` / `condition` / `model_field` / `page_dataflow`）
- `tests/fixtures/corpus/` 的 `manifest.json` / `selection.json` / `samples` 流程

## Summary

低代码平台实际有 **四种**元数据格式，但 metadata-checker 目前只理解两种：

| 格式 | 语义 | 当前支持 |
|---|---|---|
| `.tbl` | 数据存储/加工逻辑（物理表、DataFlow） | ✅ 已支持 |
| `.spg` | 数据生产/修改页面（表单、动作） | ✅ 已支持 |
| `.dash` | Dashboard，管理层/中层读取分析数据的看板 | ❌ 完全不支持 |
| `.rpt` | Report，结构化报表 | ❌ 完全不支持 |

`.dash`/`.rpt` 对 scanner 而言完全不可见（`SourceKind` 只有 `Spg`/`Tbl`/`Unknown`，
`.dash`/`.rpt` 落在 `Unknown`，目录遍历也只收集 `.spg`/`.tbl`）。这意味着当前"跨文件图数据库"
给出的任何"这个字段被谁读取/谁写入"的结论，**天然缺失了分析层（管理层看板/报表）这一环**，
是一个真实的、有具体证据支撑的理解缺口，而不是模糊的担忧。

目标：把数据全生命周期（`tbl` 存储 → `spg` 生产/修改 → `dash`/`rpt` 呈现/分析）在同一张图里
打通，让 AI 能回答"如果我改这个字段，哪些看板/报表会受影响"这类端到端问题。

## 背景判断（证据）

对比了 BI 前端源码（`com.succez.bi` 的 `web/static-file/ana/dashboard/*.ts`、
`types/report-meta-types.d.ts`）与真实样本文件（销售易项目下的 `ana/*.dash`、`ana/*.rpt`），
确认两种格式在结构上是 `.spg`/`.tbl` 的近亲，而不是全新的语义体系：

- **`.rpt`**：
  - `sources[].path` 使用与现有代码完全相同的 `$DATA:/.../*.tbl` 路径模式，
    `model_scope.rs` 的 `normalize_dataflow_path` 已经通用化处理这个前缀，无需新写解析逻辑。
  - `paramsPanel.children[]` 是组件树，字段形如
    `{ id, type, field: "model1.carModelDetail", value: "=param1" }`，
    与 `SpgComponent` 的字段绑定结构一一对应。
  - `sheets[].rows[].cells[]` 里的取值形如 `"value": "=model1.dimDepart"`、
    `"value": "=AVG(model1.MSRP)"`——与 SuperPage 完全相同的 `=` 前缀表达式语法。
- **`.dash`**：
  - `sources[]` 既可以是引用式（同 `.rpt` 的 `$DATA:` path），也可以内嵌一个**完整的
    DataFlow 模型**（`dataFlow.nodes`、`type: DbTable`、`filter.clauses`、`fields[]`），
    与 `scanner::tbl::process_tbl_file_from_string` 已经处理的内嵌 DataFlow 结构**完全同构**。
  - `components[]`（widget 树）每个 widget 有稳定 `id`（如 `kpi1`）和
    `dataDefinitions.measure[].exp`（如 `"model1.$ROWCOUNT"`），
    与 `SpgComponent` 的 `exp` 字段绑定语义一致。

结论：现有的表达式引用解析（`superpage::expr_ast::parse_expression_refs`）、内嵌 DataFlow
图构建逻辑、`$DATA:` 路径解析（`model_scope.rs`）都是可以直接复用或轻量抽取复用的底层能力，
不需要为 `.dash`/`.rpt` 从零实现语义理解。

## Goals

- `.dash`、`.rpt` 能被 scanner 识别、解析，并把节点/边写入与 `.spg`/`.tbl` 相同的项目图。
- `query_model`/`explain model:X` 能在"读者"列表里看到 dashboard/report；
  复用 `Reads` 遍历，并适配容器归属、输出与后续查询建议。
- 复用现有底层能力，不重写已验证的 `.spg`/`.tbl` 解析管线。
- 真实样本（一份 `.dash`、一份 `.rpt`）进入 `tests/fixtures/corpus/`，
  走与 M9 相同的 manifest/selection/snapshot 流程。

## Non-Goals（本阶段明确不做）

- 不做元数据的创建/修改能力（这是后续独立里程碑，依赖本阶段先把"理解"打牢）。
- 不做单文件优先的分阶段验收（`.spg`/`.tbl` 历史上是先单文件后建图，但此次目标已明确是
  全生命周期图查询，因此直接做图集成，不单独产出一版"仅能读单个 dash/rpt 文件"的中间态 CLI 体验）。
- 不为 report 表格单元格（cell）建立独立图节点——cell 只有 `si`/行列位置，没有稳定用户态 id，
  强行赋 id 会在重新排版后产生虚假的节点身份漂移。cell 级别的字段引用改为聚合去重后挂在
  `Report` 节点的 `Reads` 边上。
- 不引入统一的跨格式 IR（详见下方方案对比的方案 C），避免为了架构纯粹性而重构已有
  150+ 测试覆盖的 `.spg`/`.tbl` 管线。
- 不在本阶段深化"真实场景 LLM 使用效果验证"（这是独立缺口），本阶段的 `ai_eval` 扩展
  仅覆盖新增的 dash/rpt 查询能力本身。

## 架构方案对比

| 方案 | 描述 | 优点 | 缺点 |
|---|---|---|---|
| A. 强行复用 SuperPage | 把 dash/rpt 的组件树适配成 `RawComponent`，直接跑 `superpage.rs` | 代码量最小 | widget（图表配置）和 report 网格（行列结构）本质不是组件树，硬套会污染 `superpage.rs`，违反 AGENTS.md 模块边界 |
| **B. 并列模块 + 抽取共享原语（采用）** | 新增 `dashboard.rs`/`report.rs`，各自独立解析，但复用抽取出的表达式解析、内嵌 DataFlow 图构建、`$DATA:` 路径解析 | 符合现有模块边界；复用的都是已验证的确定性逻辑；改动面可控 | 比方案 A 多一点新代码 |
| C. 统一跨格式 IR | 四种格式统一归一化到一个抽象绑定/引用表示，再喂给单一图构建器 | 长期最"干净" | 需要重构已上线的 spg/tbl 管线，风险高、收益不确定，违反"不为架构纯粹性重构可用代码"的原则 |

选择方案 B。

## 数据模型

### 新增节点类型

`graph.rs` 的 `NodeType` 新增 `Dashboard`、`Report`（与 `Page` 并列，不复用 `Page` 加 meta
判别字段，以保持 ID 前缀自解释：`dashboard:`/`report:`，与现有
`page:`/`model:`/`component:`/`action:` 约定一致，`--explain` 可直接按前缀分发）。

### 节点复用

- `.dash` 的 widget、`.rpt` 的 `paramsPanel.children[]` 复用现有 `Component` 节点类型，
  通过 `Contains` 边挂在 `Dashboard`/`Report` 节点下——它们都有稳定 id，语义上就是
  "一个带字段/表达式绑定的 UI 元素"。
- `.rpt` 表格 cell **不建独立节点**（见 Non-Goals）。
- `.dash` 内嵌 DataFlow 模型复用现有"页面局部 model"模式（`model:PAGE|MODEL`，
  `model_scope.rs` 已实现），把 Dashboard 当作一种可以拥有局部 model 的"页面"。
- `.rpt` 的 `sources[].path`（`$DATA:/...`）直接解析到图中已存在（或新建）的物理表
  `Model` 节点，复用 `model_scope.rs` 的路径归一化。

### 边复用

不新增边类型。`Reads`、`Contains`、`DataflowInternal`、`DataflowInput` 语义已经覆盖
"看板/报表读取字段"的场景。

## 解析方案

1. `source_id.rs`：`SourceKind` 新增 `Dash`、`Rpt`。
2. `scanner/indexer.rs`：扩展目录发现、类型分派与失败处理，支持 `.dash`/`.rpt`。
3. 从 `scanner::tbl::process_tbl_file_from_string` 中抽取内部函数
   `build_dataflow_model_graph(graph, model_id, source_path, value)`，让顶层 `.tbl`
   （`model_id` 从文件名派生）和 `.dash` 内嵌 source（`model_id` 为
   `model:{dash_path}|{source_id}`）共用同一套 DataFlow 图构建逻辑。
4. `superpage::expr_ast::parse_expression_refs(expr: &str)` 已经是纯函数，
   `dashboard.rs`/`report.rs` 直接调用，不需要额外抽取。
5. 新增 `src/dashboard.rs`、`src/report.rs`：各自定义 `RawXxx`/`XxxMetadata` 类型和
   组件/绑定提取逻辑（镜像 `superpage/mod.rs`、`tbl_single.rs` 的结构，不共享结构体）。
6. 新增 `src/scanner/dashboard.rs`、`src/scanner/report.rs`：图构建入口
   （镜像 `scanner/spg.rs`、`scanner/tbl.rs`），在 `scanner/mod.rs` 中注册。
7. `explain/handlers/` 新增 `dashboard`/`report` 处理器，复用 `explain_page_graph` 的
   只读变体（有 `data_sources`，没有 `entrypoints`/`write_targets`，因为看板/报表不提交数据）。
8. `graph::find_readers`（`src/graph.rs:119`）按 `Reads` 边过滤，可复用这一遍历原语；
   **查询层仍需适配新节点类型**。`context::generate_next_queries`（`src/context.rs:22`）
   穷尽匹配 `NodeType`，新增变体须同步处理。`query_model`（`src/query/model.rs:31`）
   经 `find_parent_page`（`src/query.rs:48`）补充读者归属，后者当前只识别 `Page`。
   实施时须明确 dashboard/report 本身与其子组件的归属表示，检查 `related_pages`、
   explain/route 的类型分派及 next_queries，不能以出现一条 Reads 边代替完整契约验证。
   本节源码位置按 `be23369` 核对；目标格式适配尚未实现。

## 阶段拆分（不占用里程碑编号）

- **阶段一：`.dash` 解析与图集成**
  - `Dashboard` 节点类型、widget → `Component` 节点、内嵌 DataFlow 局部 model。
  - `explain dashboard:X`、`query_model` 验证读者列表包含 dashboard。
  - 一份真实 `.dash` 样本进 `tests/fixtures/corpus/`。
- **阶段二：`.rpt` 解析与图集成**
  - `Report` 节点类型、`paramsPanel.children[]` → `Component` 节点、
    `sources[].path` → 物理表 model 解析、cell 级引用聚合。
  - `explain report:X`、`query_model` 验证读者列表包含 report。
  - 一份真实 `.rpt` 样本进 `tests/fixtures/corpus/`。

阶段一先行，因为内嵌 DataFlow 模型的复用（`build_dataflow_model_graph` 抽取）是两者共用的
基础设施，且 `.dash` 结构更复杂，提前踩坑能降低阶段二的不确定性。

## 测试与验收

- 沿用现有分层：parser 单元测试（`tests/dashboard_tests.rs`、`tests/report_tests.rs`）、
  scanner 建图测试、`query_model`/`explain` 集成测试。
- 查询测试须覆盖根节点和子组件的读者归属、context 分类、可执行的 next_queries，
  以及现有 SPG/TBL 输出兼容性；新增枚举变体的编译通过不等于这些行为已正确。
- 真实语料：两个真实样本按 M9 流程（`manifest.json` 登记、`selection.json` 精选、
  `snapshots/` 契约快照）纳入 `tests/fixtures/corpus/`。
- `ai_eval` 扩展：新增覆盖 dash/rpt 的问答用例，验证"这个字段被哪些看板读取"这类端到端问题的
  evidence/diagnostics 与现有格式同等严谨。
- 验收边界：不引入新边类型；不改变现有 `.spg`/`.tbl` 的图语义或输出 schema；
  `NodeType` 枚举变更需同步 `graph_store`/`graph_redb`/序列化逻辑（AGENTS.md 已知改动面）。

## 后续队列（本次不做，但已识别）

1. **真实场景 LLM 使用效果的深化验证**——现有 `ai_eval` 基于精选 fixture，
   还不足以证明真实可靠性，需要独立里程碑设计（如何采样真实使用场景、
   如何评估"保守回答"是否合适等）。
2. **元数据创建/修改能力**——本次 brainstorming 的原始出发点，明确要在"理解"
   （含本文补齐的四种格式）站稳之后再设计，因为写入能力的正确性验证依赖于
   先有可靠的读取/图查询能力作为校验基准。
