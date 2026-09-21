# M59 语义完整性验收记录

- PR：[41](https://cnb.cool/wu2305/metadata-checker/-/pulls/41)，基线 `b2c56ee`。
- 全量代码：`2e2a40f` + CNB cargo fmt；格式提交 `c4bb9d6`。
- 最后测试加强：`c4bb9d6` 的身份往返断言；远端 6 项通过，格式提交 `ceb81c5`。
- 环境：CNB Linux 工作区 `cnb-c9g-1k30leog8`，已停止。
- [原始日志及 SHA256](../governance/evidence/2026-09-21-m59-semantic-integrity.json.gz)。

| 验证 | 结果 |
|---|---|
| `cargo test --features cli-local --no-fail-fast` | 104 test executables + 1 doctest group；1258 passed / 0 failed / 24 ignored |
| `cargo test --test m59_identity_validation_tests` | 最后加强后 6 passed |
| `cargo check --benches` | 通过；未执行性能测量 |
| `cargo check --no-default-features --features browser-wasm --target wasm32-unknown-unknown` | 通过；8 条 warning，位于本批未修改的声明/导入 |
| `cargo fmt --check` | 通过 |
| 原有 corpus 快照 | 未改动且通过；不是真实语料验收 |

没有执行 ignored 的真实项目测试、在线 LLM 评测和性能诊断；不能据此声称这些场景通过。
曾观察到实验性 JSON-only 边键使重启邻接顺序变化；生产编码改为关系排序前缀 + NUL +
完整事实 JSON。调试生成的旧 fixture 库隔离后从源重建，原快照通过，未修改快照期望。

独立审查发现的八项问题均已补代码和反例：不完整 AST 判定、CLI stale 回退、绝对路径、
全局分隔符写入、shadow 读取门槛、TBL 失败前置校验、全局构造器 kind 约束、只有 v2 shadow 的旧库被自动补版本。
最后一项修复为 `e88a980`：缺 marker 且存在任何 v2_ 表即拒载，回归也验证拒载后 marker 未写入。

范围：修复引用/事实信息丢失和错误完整性声明；不关闭 M59-2 origin 撤销、共享目标生命周期、
项目绑定、B5，也不关闭完整 F3/F5/F6、REPLAY/BASELINE、PATH、Grafeo。

最终 shadow 门槛在 CNB `cnb-b5n-1k30n19tj` 定向复验：
`cargo test --features cli-local --test m59_fact_identity_tests --test m52_redb_v2_shadow_tests --test m53_redb_v2_hydrate_tests --test m58_3_pr1_refix_graph_redb_tests`
通过；`cargo fmt --check` 通过。原始日志已追加同一证据文件。

独立复审：主 style/smell 模型在当前账户不可用，按规则使用 fallback 验收者；
最终对 `e88a980` 给出 `FALLBACK_PASS`，此前八项均闭合，无剩余 P0/P1/P2 阻塞项。
验收者只读静态复核，CNB 执行证据由主线程提供，不声称独立重复运行。
