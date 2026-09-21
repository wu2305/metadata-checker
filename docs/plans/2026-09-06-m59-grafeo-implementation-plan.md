# M59 Grafeo 迁移实施与 M58 交接计划

> 状态：**approved**（2026-09-06 用户同意完成 M58 收口、PR14 合并准备与 M59 启动事项；本文细化已批准迁移设计的执行顺序）
> Spec：[Grafeo 迁移设计](../specs/2026-09-05-grafeo-backend-migration-design.md)（approved）
> Journal：[M59](../milestones/performance/m59-grafeo-backend-migration.md)
> 上游：[M58 范围收口](../milestones/ai-eval/m58-cheap-model-comprehension-eval.md)、[M58.3 原计划](2026-08-24-m58-3-command-surface-gap-fixes-plan.md)。

## 当前边界

PR14 交付评测基础设施、M58.3 PR1/PR2 和 M59 的 B1–B5/A1b 部分前置工作。
本计划从 PR14 合入后的 main 建分支执行，不继续向 PR14 追加迁移实现。
Grafeo 选型与本次迁移的 10 MB 体积预算豁免沿用已批准 spec，不重开选型。

完成 B5 指的是差分测试已建立。目前两项测试明确记录增量丢跨文件边和删除后残留占位
节点；M59-2 必须把它们改成全量与增量完全相等的回归测试。不得保留缺陷差额后宣称迁移正确。

## 执行单元

下表编号是本计划内的任务包，不是 CNB PR 号。每包单独 PR，经影响面测试与独立复核后
合入 main，下一包从更新后的 main 开始。主线程负责集成和验收账本；执行者只修改领到的
模块，独立验收者只读。表中失败处理不允许降低断言或更新快照掩盖差异。

| 包 | 现在能否做 / 前置 | 写入责任与结果 | 验收与交接 | 失败处理 |
|----|------------------|----------------|------------|----------|
| M59-1：A1/A1b/A2 身份与路径 | PR14 合入后可做 | graph/scanner/superpage/dependency 与 target 解析：统一项目、源文件、局部 id 身份；保留原始引用 token；共用路径归一化 | 同名跨页隔离；旧 target 显式解析，歧义诊断；中文、分隔符、点段、越界、引用 token 精确回归。身份编码与下包 A4 的版本切换共同发布，交接 M59-2 | 若新身份无法与旧持久化安全隔离，先保留旧默认路径，将身份编码放在未启用的新路径；不得读写混合 schema |
| M59-2：A3/A4 归属与版本切换 | 依赖 M59-1；A1 新编码在本包启用 | indexer/scanner/graph/store/persistence：节点和边的 origin_file、共享目标归属处理、版本键、拒绝不兼容库、强制重建 | 删除/修改被引用文件、删除唯一引用者、重启、修复后，全量和增量节点/边/全部属性及重复次数相等；B3 坏 TBL 保留旧图；B1 双后端契约通过。交接 M59-3 | 保持旧库可恢复，诊断说明重建原因；归属差分未过不得进入 C |
| M59-3：C1/C2 Grafeo store 与导入 | A 完成，按新 schema 复验 B1–B5 | GrafeoGraphStore、Cargo 特性、导入与持久化接线：原生 bulk/store API、自持 NodeId、索引 | 同一 B1 套件覆盖 memory/redb/Grafeo；重复写入、重启、删除、导入结果及错误语义一致；不把逐条 Cypher 导入当正式路径。交接 M59-4 | 保留 redb 默认实现；C1 受阻时按 spec 评估 with_read_store 退路，并记录差异，不静默替换方案 |
| M59-4：C3/C4/C5 查询、投影与 WASM | 依赖 M59-3 | query/runtime/output/visualization 与 browser glue：21 类链路边、只读表投影、TSV/JSON、WASM 接线 | 每类边有精确投影测试；修复 M59-PATH；query 不绑定 redb；保留原始文件读取与证据定位；浏览器 WASM 构建和接入回归。新用户契约超出 spec 时先补批准设计。交接 M59-5 | 不支持的能力给稳定诊断；不宣称功能等价，不提前退役 redb |
| M59-5：D1/D2 真实验收与退役 | 依赖 C 全部完成 | 真实语料测量、baseline、redb/v2 shadow 退役与 docs | 固定源码/语料/hash，比较全量及增量产物、重启与查询；测冷启动至首查询、RSS/峰值和产物大小；通过后删除 redb 并重跑影响面测试。交接 M59 journal 收口 | 正确性或兼容性未过保留旧后端；性能未达目标按测量定位，不能仅凭合成图速度宣布成功 |

身份与 schema 的发布约束：M59-1 可独立合入未启用的编码/helper，但不得在旧 schema
版本下默认写入新 id。若无法拆开，应将 M59-1/M59-2 合成同一个 PR，先完成 A4 的
拒绝旧库与重建机制，再启用新身份；这只是交付边界合并，不改变 A 先于 C 的依赖。

## 2026-09-21 语义完整性修复补充

用户授权直接处理对抗性审核发现的根因，执行
[语义完整性修复 spec](../specs/2026-09-21-semantic-integrity-design.md)。
本批修复引用区间、追溯状态、属性路径、身份输入校验与事实证据丢失；事实存储版本独立于
M59-2 的实体归属版本。M59-2 必须消费该 spec 的实体/事实/来源撤销契约，不得把本批
fact 版本门槛当成 A3/A4 或 B5 已完成。Grafeo 仍按 M59-3 排序。

## M58 未完成事项的承接账本

“deferred”表示仍未完成。迁移实现不会自动完成表达式/事实契约，也不会证明模型的业务理解力。

| 来源 / 跟踪键 | 承接责任与阶段 | 状态 | 关闭条件 |
|---------------|----------------|------|----------|
| F4 / PR4b：页面局部身份 | M59-1 执行者，M59-2 集成 | queued | 跨页同名隔离、兼容 target 歧义、源范围身份与 schema 迁移回归通过 |
| PR4a：版本与重建 | M59-2 执行者 | queued | 不兼容库拒绝加载，不静默混用旧数据，完整重建后可恢复 |
| F3 / M59-F3：递归表达式提取 | M59 journal 查询事实后续队列；主线程维护，parser 执行包承接 | deferred，不阻塞纯后端迁移 | 依据原 M58.3 F3 设计补全嵌套表达式、CASE WHEN 与中文精确测试，CLI 能暴露源文件内对应表达式；实现前单列 PR |
| F5/F6 / M59-FACTS：calc 与写入条件事实 | M59 journal 查询事实后续队列；query 执行包承接，依赖 M59-F3 | deferred，不计入 C3/C4 已完成条件 | 依原 M58.3 事实契约证明 calc 值来源和 writer conditionExp，正常/降级/缺证据输出精确一致，源定位可复核；实现前单列 PR |
| PR6 / M59-REPLAY：全量重建与 13-case 重放 | M59-5 执行者与独立验收者 | queued | pin 语料与工具版本，全量重建后逐 case 保存命令、输出、断言与 gold 差异；若受 F3/FACTS 阻塞，逐 case 记原因，不放宽 gold，未完成部分继续留账 |
| PR6 / M59-BASELINE：11-case 首轮基线 | M59-5 后评测执行包；主线程负责排期 | deferred | 走 api_trigger_kimi_harness_smoke，同 pipeline generate/judge，归档 run/records/judge/transcripts、fixture hash/kimi 版本与三个率。可记录失败基线，不要求模型全答对，不把旧 6-case 分数代用 |
| PR6：断言变迁台账 | 本次 PR14 收口独立复核；未来变更由各包验收者复核 | 当前条目复核见链接；持续义务 | [快照台账](../ai-eval-runs/2026-08-26-m58-3-snapshot-change-ledger.md)逐项源证据与决定；新断言变化随 PR 登记，不以测试绿代替语义判定 |
| 路径特征 / M59-PATH | M59-4 query 执行者 | queued，迁移查询验收前修复 | 整条路径的 same_page 与真实物理字段来源判定正确；同页、跨页、跨 SPG/TBL 及普通 name 字段精确回归，独立复核快照 |
| 建图 wall +41% 未归因 | M59-5 性能执行者 | observation | 记录同 pin、独占环境下建图/加载测量，区分环境差异与代码影响；不能声称迁移必然消除此差异 |

M59 的后端迁移可以在上述 F3/FACTS/BASELINE 仍 deferred 时按范围验收，但必须在 journal
保留承接键和未完成原因；不得将这些项改为 done。真实重建和图产物正确性属于 D1 必需项。

## 验证方式

Rust 编译/测试只在 CNB 环境的 `/workspace` login shell 执行。任务包按实际改动选择套件：

```bash
cargo test --features cli-local --test m59_b1_graph_store_contract_tests
cargo test --features cli-local --test m59_b3_tbl_parse_failure_tests
cargo test --features cli-local --test m59_b5_full_vs_incremental_diff_tests
cargo test --features cli-local --test core_feature_tests --test corpus_snapshot_tests
cargo check --no-default-features --features browser-wasm --target wasm32-unknown-unknown
```

身份、schema 和后端切换跨核心模块，除上述精确回归外需要完整 native 测试；D1 使用真实
语料单独测量。LLM 基线仍走 api_trigger 流水线，不以 dev workspace 手跑替代。
文档提交验证链接、表格状态与 `git diff --check`；所有交付批次按 AGENTS.md 运行指定
style/smell 审查。PR 中分别写语义验收、测试与性能的证据范围。


## 2026-09-21 项目绑定准备阶段的收口约束

见 [绑定门槛修正 spec](../specs/2026-09-21-project-binding-gate-design.md)。
`project_binding_schema_version=1` 仅表示项目绑定已初始化，不能当作 ownership 完成。
当前不写 `ownership_schema_version`；早期实验库中的该 marker 一律拒载，在新路径从源重建。
session 仍使用已发布扫描栈，不启用准备格式。重新接线必须同时交付来源账本、读写入口的
项目绑定传递及 B5 独立预期/全量增量验收；不得仅因准备 API 测试通过就启用新页面 ID。
