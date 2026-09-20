# M59：Grafeo 后端迁移

> 状态：**active**（2026-09-06；前置工作已在 PR14，后续实施从其合入后的 main 开始）
> Spec：[迁移设计](../../specs/2026-09-05-grafeo-backend-migration-design.md)（approved）
> Plan：[实施与交接计划](../../plans/2026-09-06-m59-grafeo-implementation-plan.md)（approved）

## M59-1 交付（2026-09-20）

A1/A1b/A2 已交付，提交 `03b2336..2ff7c69`（分支 codex/m59-1-identity-grammar）：

- **A1b**：`RefType::ComponentValue` 携带来源文法 `ComponentValueForm`
  （Value / Suffix / Bare），值追溯替换 pattern 与来源文法一致，替换点不再按
  后缀猜测；行为层断言（m59_a1b / m59_b4）原样通过，另增两条原始 token 保留回归。
- **A1**：`graph_identity` 模块提供页面局部 id 文法 `<kind>:<PAGE>|<local>`
  （竖线判据，物理表保持全局）、节点 id 解析与旧 target 显式解析
  （scoped 精确命中；裸名唯一命中直达，多候选交回全部并按 id 升序，不静默挑选）。
- **A2**：`normalize_project_path` / `resolve_relative_reference` 统一路径归一化
  （分隔符 / `.` / `..` / 越界报错），中文路径精确回归。

验证：CNB workspace 全量 native 测试（102 个 test target）通过，
`cargo fmt --check`、`cargo check --benches`、browser-wasm check 通过。

**未启用边界（交接 M59-2）**：在 A4 的 schema 版本键、拒载旧库（`GRAPH_SCHEMA_STALE`）
与强制全量重建就绪前，扫描写入与引用解析保持旧路径，`graph_identity` 的新 id 编码
不得默认写入，避免读写混合 schema。A3 的 origin_file（节点与边）与共享目标 /
占位节点归属同样在 M59-2 落地；启用后按新 schema 复验 B1–B5，B5 的跨文件边丢失与
占位节点残留差异必须清零，不得把缺陷记录当正确性证明。

## 已有基础与限制

- B1–B5 已建立共享 store、分类、坏 TBL、值追溯和全量/增量差分回归。
  A1b 的值追溯半边与枚举元数半边均已实现（见上方 M59-1 交付）。
- `c9b1611` / `df394f2` 补齐 B1 非空 metadata/完整邻接节点更新、B3 完整修复快照、
  B5 保留重复次数的比较。远端 39 项目标测试通过，四类临时故障注入被断言拒绝，
  指定主 style/smell reviewer 对该三文件修改批次 PASS；不代表整个 PR14 或后端等价验收。
- `df394f2` 的 [CNB CI](https://cnb.cool/wu2305/metadata-checker/-/build/logs/cnb-gn8-1k1rhce0o)
  已通过 Rust 测试/覆盖率、format、benches check、browser-wasm check 和浏览器 benchmark。
- B5 仍明确记录跨文件边丢失与占位节点残留；M59-2 要消除差异，不能把缺陷记录当正确性证明。
- [真实语料测量](../../ai-eval-runs/2026-09-06-m59-real-corpus-measurements.md)已建立
  pin `c3c0528fdd28e2600e0b0235040fb349b3c2d446`、501 SPG / 828 TBL 基线；旧后端
  89,178 节点 / 200,028 边，驻留约 3.72 GiB。它是迁移前测量，不是 Grafeo 验收。

## 后续队列与交接

实施包 M59-1 至 M59-5 的写入边界、前置条件、接手者和失败处理见 plan。
M58 转来的 M59-F3、M59-FACTS、M59-REPLAY、M59-BASELINE，以及快照复核保留的
M59-PATH（same_page/物理字段识别，交 M59-4 修复）同样由该 plan 维护，
均不得随换库自动关闭。原 Dashboard/Report 格式支持保留为待排期事项，撤销旧 M59
编号占位，今后排期时另行登记。

## 收口条件

A 身份/schema 定稿、B 按新 schema 复验、C 查询/持久化/WASM 落地、D 真实图产物正确性
与性能测量完成后才能退役 redb。每个合入 PR 在此回填 commit、验证结果和剩余项；
延期评测与事实能力若未完成，明确范围收口并继续留账，不宣称产品理解力目标已达成。
