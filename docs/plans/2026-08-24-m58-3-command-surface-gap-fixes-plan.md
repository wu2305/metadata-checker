# M58.3 缺口修复与可观测性打底计划

> 状态：**draft**（随 spec 第三轮修订同步起草；spec approved 后本计划一并转 approved）
> 上游：[M58.3 spec](../specs/2026-08-24-m58-3-command-surface-gap-fixes-design.md)（draft，
> 三轮独立评审修订；id 文法、诊断信封、迁移机制、事实输出契约的权威定义都在 spec，本文不重复）。
> journal：[m58-cheap-model-comprehension-eval.md](../milestones/ai-eval/m58-cheap-model-comprehension-eval.md)。
> 治理：多 PR 工作须 approved plan（`docs/governance/planning.md:34`）。

## 背景

M58.3 修五个实跑缺口 + 一个测量缺口（F1），spec 三轮评审后定为三阶段六 PR：
可观测性打底（PR1）→ 单点修复（PR2/PR3/PR5）→ 页面局部子图 schema 迁移（PR4a/PR4b）
→ 重建与验收（PR6）。依赖关系：PR3 是 PR5 的硬依赖（calc 的 `${CASE WHEN...}` 提取失败
则 F5 无米下锅）；PR4b 依赖 PR4a 的版本机制；PR6 依赖全部。

## PR 切分

### PR1 — Phase 1 诊断信封与传播

- 写入范围：`src/diagnostics*`（或信封落点模块）、`src/graph_redb.rs`（三类计数 +
  `:178` 折叠路径 + `v2_hydrate_warning` 接入）、`src/runtime.rs`（诊断存入
  GraphRuntime、`RuntimeStatus` 透出）、`src/scanner/`（未识别键/重复 id 计数）、
  `src/query/page_logic/`（回退信号归入 `PAGE_SCOPED_TARGET_FALLBACK`）。
- 验收：spec 验收 1（三处可见、契约测试、pin 语料正常路径诊断 = 0 或白名单基线登记）；
  坏行 hydrate 测试（坏节点行/坏边行/悬挂边 → 计数 + `GRAPH_DB_PARTIAL_HYDRATE`）。
- 测试：`m58_3_command_surface_tests`、`graph_store_tests`、新增坏行 hydrate 测试文件。

### PR2 — F1 形态感知递归 + F2 祖先链

- 写入范围：`src/superpage/raw_types.rs`（flatten extra map）、`src/superpage/mod.rs`、
  `src/scanner/spg.rs`（裸 Value 递归平行修改、comp→comp Contains、重复 id 诊断）、
  `src/explain/condition_facts/conditions.rs`（补读节点 meta、放宽前缀、确定性选父）。
- 前置：附录 A 逐键逐形态测量回填（测量脚本入 `tools/` 或证据目录）。
- 验收：spec 验收 2；说明 A 测试电池（唯一父/无环/序稳定/page_logic 遍历/增量重建）。

### PR3 — F3 表达式递归合并

- 写入范围：`src/superpage/expr_ast.rs`（tokenizer `${}` 分支或 `extract_refs_from_ast`
  合并层）。
- 验收：测试矩阵（普通 `${model.field}` 兼容、`${IF(...)}` 多引用、缺右花括号、嵌套
  调用、纯字面量）；垃圾节点 `model:IF(model6` 不再产生。

### PR4a — 图 schema 版本机制（行为不变）

- 写入范围：`src/graph_redb.rs`（`graph_schema_version` 键、v1/v2 双路校验、
  `GRAPH_SCHEMA_STALE` fail-closed）、`src/scanner/indexer.rs`（版本不匹配强制全量）、
  `src/runtime.rs`（reload 路径）。
- 期望版本 = v1：缺键/旧库正常加载，**无行为变化**；checkpoint 随版本失效机制落地。
- 验收：迁移测试矩阵（v1-only / v2 shadow / graphdb-only / reload / 增量 /
  `--non-human` 失败形态）。

### PR4b — Phase 3 完整局部子图

- 写入范围：`src/scanner/spg.rs`（五类局部节点 id 分段 + 边端点迁移 + 写入双写）、
  `src/route.rs`（model/field 前缀禁用 `by_overqualified` 剥离）、`src/model_scope.rs`
  （说明 B 平局替代）、`src/query/` + `src/main.rs`（next_queries 31 处生成点审计）、
  `src/output/`。期望版本 bump 到 v2。
- 验收：spec 验收 3（歧义契约、精确命中、聚合语义不变、next_queries 粒度、
  `GRAPH_SCHEMA_STALE`、增量隔离）。

### PR5 — F5 + F6 事实输出契约

- 写入范围：`src/explain/condition_facts/value_source.rs`、`src/scanner/spg.rs`
  （写边 meta 抄 conditionExp）、`src/explain/`（说明 C 的分组去重/source_field 过滤/
  证据与诊断字段）。
- 依赖：PR3（F3）已合入。
- 验收：说明 C 契约测试；calc case 重放产出非空 reads/lineage。

### PR6 — 重建 + 重放 + 首轮基线

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
- 附录 A 测量若与 1049 总数出入大，F1 验收基线以回填后的逐键计数为准并在 spec 登记。
