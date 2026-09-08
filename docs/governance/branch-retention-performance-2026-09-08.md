# 2026-09-08 性能归档改动去向复核

以 PR37 合并后的 `main=db158cb8dc609a0050c214aed788dde459044361` 为源码基线，
核查[前轮报告](branch-retention-patches-2026-09-08.md)留下的 18 项：M51 的 7 项、
M52-performance-optimization 的 11 项。计数单位是「归档分支、路径」组合。

本报告与[机器清单](branch-retention-performance-2026-09-08.json)追加新的核查结果，
不改写前轮 schema 2 清单、历史统计或用户补充的证据形态说明。
所有源码行号均指上述固定 main；本 PR 的文档补注可能使工作区行号后移。
证据是旧 hunk 与当前实现/断言的静态对照，不是当前运行日志或性能验收。

18 项均已记录去向，包括明确未保留的部分；加上前轮 24 项与 PR36 的 1 项，
43 项不再留有未分类路径。128 项精确历史版本证据仍是历史证据，不能升级为当前行为相等。
机器清单的 `unresolved_retention=0` 仅指去向已分类，下面的代码和测试缺口仍开放。

## 不能算作已覆盖的差异

**M51 的跨三阶段邻接缓存没有完整保留。** 旧实现让 entrypoint、edge scan、action flow
共享一份缓存；当前 `src/query/page_logic.rs:1433` 的 edge bundle helper 和 `:1753` 的
外层查询各有自己的缓存。`:1801` 的 action flow 再次向底层读取 helper 已取过的 action
邻接结果。两阶段复用仍在，跨到第三阶段的优化已经丢失；目前只有静态调用链证据，
没有量化性能影响。本轮修正 [M51 文档](../milestones/performance/m51-performance-diagnosis.md)
的完整复用声明及旧 runtime 阶段名，并保留代码回归项。

**native fragment 文件持久化明确延期。** 原型的模块导出、bundle 序列化、store、
runtime 命中/回填，以及两个磁盘测试没有合入。当前内存 warm cache 不提供跨 session
复用，IndexedDB stub 也不能替代这些能力。保留归档 tag，不恢复该原型。

**reload 采样策略和细分观测未合入，不能套用 fragment 的延期结论。** 归档实现用
创建时间、大小、避开 header 的固定位置采样 hash 判定变化，将 mtime 仅用于诊断；
当前 `src/runtime.rs:238` 读取整文件、只 hash 前 4096 字节，`:1361` 仍将 mtime、
size、hash 任一变化作为重载条件。三个 profiled API、`ReloadCheckProfile`、
八个 `runtime_check_reload_*` counter 和对应测试都不存在。
`git log main -S reload_if_changed_profiled -- src/runtime.rs` 无记录，不能说它曾合入后被删除。

本轮修正 [M53 第5节](../milestones/performance/m53-performance-continuation.md)的
“已修复/继续保留稳定采样”错误声明，将 676ms→0ms 明确标为未合入原型的历史测量。
`src/perf_report.rs:991` 的阶段仍叫 `runtime_check_reload_unchanged`，但只统计一次
CheckReload 调用和 diagnostic 数，未断言结果为 Unchanged；阶段名不能充当快路径证据。

[M52 文档](../milestones/performance/m52-performance-optimization.md)增加历史语境说明：
其“当前/下一步”、默认 v1 与性能表均属于旧阶段；M53 已改为优先 v2 hydrate。
旧分支多出的第三轮 warm 采用了后续实现；第四轮 fragment/reload 提案仍须单独标注未合入。

## 测试保留与尚未解决的缺口

M52 的 normal warm 测试仍断言三个 `*_read_model_used == 1`，其中两条只是下移。
旧 `prerequisites_materialize/path_materialize` 阶段名改为完整 warm 查询使用的
`prerequisites/path_summary`，并新增 compact 输出等价用例。
这些测试重排 JSON 数组后比较，不能表述为原始字节或数组顺序等价。

以下缺口仍开放，本轮没有通过文档审计宣称它们已经修复：

| 后续项 | 现有问题 | 需要的证据 |
|---|---|---|
| page-logic 邻接缓存 | helper 与 action flow 各持独立 cache，旧跨三阶段复用退化 | 计数型 GraphReadStore 锁定重复读取；恢复共享生命周期并验证输出与错误传播 |
| reload 检测与观测 | 阶段名不证明 Unchanged；整文件读取仅 hash 前缀；旧采样策略也可能漏检 | CNB 上检查 unchanged、redb 打开扰动、同大小更新、文件替换、读取失败与真实结果计数；先确定可靠契约，再优化 |
| compact 裁剪断言 | `m53_compact_warm_cache_trims_unused_path_categories` 的 `full >= compact` 允许两者同为0 | 固定图先证明 full 对应桶非空，再断言 compact 为0；同时核对保留桶内容 |
| runtime 降级测试 | `test_page_dependency_index_build_failure_falls_back_to_empty_index` 手工复制 fallback，丢弃 build_ms | 通过真实加载入口触发失败，验证降级模型、诊断与可继续查询；不靠毫秒必须非零 |
| 计时/报告断言 | 多处仅验证 stage/counter 存在或耗时大于0 | 验证实际调用路径与计数含义；计时分辨率不应决定行为测试成败 |
| cost model 指标接线 | `perf_report.rs:408` 查找的 prerequisites 三个 counter 名与生产者不符；path 模型还查找未产生的 `path_related_rejected_expansion_ms` | 将报告驱动项与实际 counter 对齐，测试明确要求这些驱动项进入报告；计时 `.max(1)` 后大于0的断言不能证明接线正确 |

`prerequisites.rs:26` 还直接忽略 `budget/for_cache_materialization` 参数，仍物化完整前置条件；
不能把 path 桶的预算裁剪能力泛化到 prerequisites。该现状不构成归档代码丢失证明。
以上后续测试/实现工作需要分别验证契约，历史原型只作为参考。
知识库回答质量与真实语料性能流水线也不在本次静态复核的验收范围内。

## 验证与复算

机器清单保存输入报告 SHA-256、18 项 archive commit、merge base、mode/blob 与完整
base→archive patch SHA-256。复算 patch 的命令为：

```sh
git diff --no-ext-diff --unified=3 BASE ARCHIVE -- PATH
```

对输出原始字节求 SHA-256；tag 使用 `git rev-parse 'archive/NAME^{commit}'` 解引用，
不能把 annotated tag object 当作 commit。新清单与前轮 `status=unresolved` 集合一一对应。
本轮仅改文档和 JSON，验证 JSON 结构、hash、路径/引用、计数及 `git diff --check`，
没有运行本地 Cargo，也没有把源码保留核对当作 Rust 测试通过。
