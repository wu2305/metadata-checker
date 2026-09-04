# M58.3 缺口修复与可观测性打底计划

> 状态：**closed（范围收口，2026-09-05）**
> 上游：[M58.3 spec](../specs/2026-08-24-m58-3-command-surface-gap-fixes-design.md)（approved，
> 五轮独立评审修订；id 文法、诊断信封、迁移机制、事实输出契约的权威定义都在 spec，本文不重复）。
> journal：[m58-cheap-model-comprehension-eval.md](../milestones/ai-eval/m58-cheap-model-comprehension-eval.md)。
> 修订包：[2026-08-24-m58-3-appendix-a-amendment.md](2026-08-24-m58-3-appendix-a-amendment.md)
> （附录 A 回填 + 三处更正 + PR2 三条修订建议，已并入本文与 spec）。
> 治理：多 PR 工作须 approved plan（`docs/governance/planning.md:34`）。

## 收口声明（2026-09-05）

本计划不再以“六个 PR 全部落地”作为当前交付状态。已合入并完成复核的范围是 **PR1
可观测性打底 + PR2 F1/F2/ComponentProperty**；其余 PR3、PR4a、PR4b、PR5、PR6 未在本批次
落地，不能标记为已验收。

已落地证据（代码与测试均已进入当前分支）：

- PR1：`646a4ae` 起的诊断信封、hydrate/scanner 计数、runtime/query 传播与坏行测试；后续
  复核修复收敛于 `1943105`、`fecaef2`、`f711611`、`d81ca07`、`dbfe4b8`、`312c938`。
- PR2：`893c95b`、`e826ce1` 起的形态感知递归、祖先链确定性选父和 ComponentProperty 图契约；
  后续前向引用、解析归一化和快照/基线修复见 `feb5ad8`、`4d85e94`、`7118bf2`、`41c011a`、
  `e27266b`、`51907af`。
- F1 附录 A 已按同一 pin 语料重跑：到达 34,317，漏掉候选 343 全部属于排除列表，
  `343 - 343 == 0`；PR2 真实项目性能基线已回填到
  [performance-baseline.md](../milestones/performance/performance-baseline.md)。

范围收口不是“原始缺口已全部消失”的声明。F3 表达式递归合并、F4 页面局部身份/schema
迁移、F5/F6 事实输出契约和 PR6 全量重建/13 case 重放仍是未完成事项。它们不在本计划中
继续悬挂为“进行中”，后续是否实现由[本地图查询平面与后端深化研究计划](2026-09-04-local-query-plane-and-backend-research-plan.md)
的阶段 0–4 重新按真实查询缺口排序；其中页面局部身份不能被 Grafeo 选型默认带过。

## 背景

原设计为修五个实跑缺口 + 一个测量缺口（F1），spec 三轮评审后定为三阶段六 PR：
可观测性打底（PR1）→ 单点修复（PR2/PR3/PR5）→ 页面局部子图 schema 迁移（PR4a/PR4b）
→ 重建与验收（PR6）。依赖关系：PR3 是 PR5 的硬依赖（calc 的 `${CASE WHEN...}` 提取失败
则 F5 无米下锅）；PR4b 依赖 PR4a 的版本机制；PR6 依赖全部。

## PR 切分

### PR1 — Phase 1 诊断信封与传播（done）

- 写入范围：`src/diagnostics*`（或信封落点模块）、`src/graph_redb.rs`（三类计数 +
  `:178` 折叠路径 + `v2_hydrate_warning` 接入）、`src/runtime.rs`（诊断存入
  GraphRuntime、`RuntimeStatus` 透出）、`src/scanner/`（未识别键/重复 id 计数）、
  `src/query/page_logic/`（回退信号归入 `PAGE_SCOPED_TARGET_FALLBACK`）。
- 验收：spec 验收 1（三处可见、契约测试、pin 语料正常路径诊断 = 0 或白名单基线登记）；
  坏行 hydrate 测试（坏节点行/坏边行/悬挂边 → 计数 + `GRAPH_DB_PARTIAL_HYDRATE`）。
- 测试：`m58_3_command_surface_tests`、`graph_store_tests`、新增坏行 hydrate 测试文件。

### PR2 — F1 形态感知递归 + F2 祖先链 + ComponentProperty 图契约（done）

- 写入范围：`src/superpage/raw_types.rs`（flatten extra map）、`src/superpage/mod.rs`、
  `src/scanner/spg.rs`（裸 Value 递归平行修改、comp→comp Contains、重复 id 诊断、
  `ComponentProperty` 建 comp→comp `DependsOn` 边——spec 说明 D）、
  `src/explain/condition_facts/conditions.rs`（补读节点 meta、放宽前缀、确定性选父）。
- 前置：**已完成**——附录 A 测量回填（`tools/corpus-shape-audit.py` +
  `docs/ai-eval-runs/2026-08-24-m58-3-corpus-shape-audit.json`，修订包
  `2026-08-24-m58-3-appendix-a-amendment.md`）；排除列表初值取四键
  （`effectStyles`/`conditionStyles`/`labelFields`/`stateFields`），`buttons` 人工确认。
- 验收：spec 验收 2；说明 A 测试电池（唯一父/无环/序稳定/page_logic 遍历/增量重建）；
  `parse → 上下文解析 → scanner 建边` 贯通测试（说明 D）；**性能基线义务**：组件数
  预计 +32%（附录 A 推论），按 `docs/milestones/performance/performance-baseline.md`
  的 M50 标准真实项目 runner 采集，记录节点/边数、graphdb 体积、全量构建与加载耗时、
  查询 P50/P95——**实测采集为准，不用推论反推预期值**。
- 后续义务：PR2 合入后重跑 `corpus-shape-audit.py`（新组件类型的属性键分布 +
  `ComponentProperty` 计数都会变，附录 A 与说明 D 据此更新）。**已完成**
  （2026-08-25，口径同步后重跑：到达 34,317、验收判据归 0，见 spec 附录 A
  「重跑确认」与 `docs/ai-eval-runs/2026-08-25-m58-3-corpus-shape-audit-post-pr2.json`）。

### PR3 — F3 表达式递归合并（deferred）

- 写入范围：`src/superpage/expr_ast.rs`（tokenizer `${}` 分支或 `extract_refs_from_ast`
  合并层）。
- 验收：测试矩阵（普通 `${model.field}` 兼容、`${IF(...)}` 多引用、缺右花括号、嵌套
  调用、纯字面量）；垃圾节点 `model:IF(model6` 不再产生。

### PR4a — 图 schema 版本机制（行为不变，deferred）

- 写入范围：`src/graph_redb.rs`（`graph_schema_version` 键、v1/v2 双路校验、
  `GRAPH_SCHEMA_STALE` fail-closed）、`src/scanner/indexer.rs`（版本不匹配强制全量）、
  `src/runtime.rs`（reload 路径）。
- 期望版本 = v1：缺键/旧库正常加载，**无行为变化**；checkpoint 随版本失效机制落地。
- 验收：迁移测试矩阵（v1-only / v2 shadow / graphdb-only / reload / 增量 /
  `--non-human` 失败形态）。

### PR4b — Phase 3 完整局部子图（deferred）

- 写入范围：`src/scanner/spg.rs`（五类局部节点 id 分段 + 边端点迁移 + 写入双写）、
  `src/route.rs`（model/field 前缀禁用 `by_overqualified` 剥离）、`src/model_scope.rs`
  （说明 B 平局替代）、`src/query/` + `src/main.rs`（next_queries 生成点审计，
  `format_next_query(` 57 处/16 文件口径）、
  `src/output/`。期望版本 bump 到 v2。
- 验收：spec 验收 3（歧义契约、精确命中、聚合语义不变、next_queries 粒度、
  `GRAPH_SCHEMA_STALE`、增量隔离）。

### PR5 — F5 + F6 事实输出契约（deferred）

- 写入范围：`src/explain/condition_facts/value_source.rs`、`src/scanner/spg.rs`
  （写边 meta 抄 conditionExp）、`src/explain/`（说明 C 的分组去重/source_field 过滤/
  证据与诊断字段）。
- 依赖：PR3（F3）已合入。
- 验收：说明 C 契约测试；calc case 重放产出非空 reads/lineage。

### PR6 — 重建 + 重放 + 首轮基线（deferred）

- pin 语料全量重建 graphdb；13 case 重放；断言变迁台账（含独立复核）；11-case
  `api_trigger_kimi_harness_smoke` 首轮基线（路由指标与历史固定分母对比，绝对分不比）。
- 可选：PR1 合入后先跑修复前 11-case baseline 形成同语料前后对照。

## 验证矩阵

- 远端（CNB workspace）：`cargo fmt --check` + spec 验收 4 列出的 focused 套件。
- 本地：`scripts/cnb-smoke-dry-run.sh` 五条路径（正常 exit 0 / 66 records；
  `--fault empty|dup|noeat|judgeleak` 各判红）。
- 每个 PR 必须 `git push cnb HEAD` 后在远端跑影响面测试；禁止本地 cargo。

## 风险

- PR4b 爆炸半径最大（15+ 文件）：PR4a 先落机制就是把「重建是否可行」与「schema 是否
  正确」解耦；PR4b 内部可按「scanner 建点 → 查询解析 → next_queries」再切 commit。
- ~~附录 A 测量若与 1049 总数出入大~~（已闭环：回填完成，8,653 候选 / 8,310 带行为
  证据，旧口径 1049 作废，F1 验收基线以附录 A 逐键计数为准）。
- PR2/PR3 关系（修订包 §2.3）：代码层面可并行开发，但 PR3 的语料断言必须在 PR2 合入后
  重测才能关闭——PR2 放开的容器会带 1,054 处新表达式进解析路径。任何断言值变化在
  **改变它的那个 PR 里**就更新并附变迁台账，不允许跨 PR 挂「已知待更新」状态；
  两个 PR 各自独立可验收，各自的语料重建都省不掉。
