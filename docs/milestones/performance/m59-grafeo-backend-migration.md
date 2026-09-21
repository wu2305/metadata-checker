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

### 冷脸验收处理（2026-09-21，deepseek-v4.1-flash）

首轮验收结论「不通过：1 P0 / 4 P1 / 1 P2」，逐条处理（修复提交 `4764299`，
fmt/清理 `335f7af` / `31ae621`；远端全量 native 102 个 test target 复验通过）：

- **P0「新文件不在分支」**：复核为评审环境假阴性。`git show 119db87:src/graph_identity.rs`
  与 `git show 119db87:tests/m59_a1_identity_grammar_tests.rs` 均存在（评审者自引的
  `git diff --stat 539f984..119db87` 也列出新建）；PR CI（rust-ci 全量 llvm-cov，在
  `2ff7c69` 上通过，其后仅差 journal 文档）已把新测试 target 纳入 CI 可见范围。
  证据已回帖，待评审复核。
- **P1-2 field 半边缺失**：`resolve_model_target` 泛化为 `resolve_node_target`，
  scoped 精确查表支持任意 kind，裸名解析覆盖 model 与 field（跨 kind 同名互不干扰），
  新增跨页字段歧义、物理字段裸名、跨 kind 隔离回归。
- **P1-3 非 value/step 后缀静默**：核对 `83798da` 旧实现——非 value/step 后缀
  当时同样归 `RefType::Other`，非本批回归。新增分类层回归钉住该边界；诊断产物
  登记为 M59-2/query 层欠账，不在本批扩容。
- **P1-4 文法不变式**：模块文档写明「kind/页面/局部段都不得含 `|`，竖线是
  scoped 唯一判据」，补含 `|` 全局名的解析回归；`is_page_scoped_id` 未启用、无消费方。
- **P2-5 路径语义**：`normalize_project_path` 文档写明输入须根锚定、`..` 越根是
  越界诊断；评审提到的 `./app/a.spg` 实际返回 `Ok("app/a.spg")`（`.` 段直接消解），
  已有单元测试覆盖，文档同步澄清。

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

## 2026-09-21 结构性审核后的语义修复

分支 `codex/m59-semantic-integrity`，基线 `b2c56ee`，实现与复审修复截至 `2e2a40f`，
格式与加强测试 `c4bb9d6`。批准范围见 [补充 spec](../../specs/2026-09-21-semantic-integrity-design.md)。

- 引用出现位置与依赖集合分离；逐原始字节区间替换，递归栈仅检测当前链，保留表达式优先级。
- `.step` / 嵌套属性保留完整路径；不支持、未闭合、缺失、循环和深度截断输出部分完成，
  human 与 JSON 均可见，JSON 顶层 `TRACE_INCOMPLETE` 对应 partial answer effect。
- 同关系不同证据保留为不同事实；memory/redb/dense 与已有 v2 完整事实语义对齐。
  `fact_schema_version=2` 拒绝旧库，主入口、readonly、shadow 读取与 CLI 状态检查均覆盖。
- ID 构造拒绝保留分隔符与非法全局 kind，路径拒绝绝对/盘符输入；scanner 启用全局
  model/field 分隔符验证，TBL 写入前完成身份校验。**新页面局部 ID 仍未启用**。
- CNB `cnb-c9g-1k30leog8`：全量 native 1258 通过 / 0 失败 / 24 ignored；bench 与 WASM
  check 通过，WASM 有 8 条非本批修改位置的 warning；原 corpus 快照未改且通过。
  证据见 [原始日志](../../governance/evidence/2026-09-21-m59-semantic-integrity.json.gz)。

此修复不关闭 M59-2：按来源撤销、共享目标生命周期、项目绑定、跨文件 B5 仍未交付。
版本键只覆盖事实表示，不得拿它启用新 ID；旧库必须从源在新路径重建，不能改 marker 冒充迁移。
F3 目前只做到“不支持则部分完成”，完整语言与 F5/F6、REPLAY/BASELINE、PATH、Grafeo 继续留账。

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


## 2026-09-21 项目绑定门槛纠偏（M59-2 准备阶段）

PR41 已正常合并为 `224d4e1`，文件树与最终交付 `1168f11` 一致，双 CI 成功。
后续 `a70ace0` 的准备实现存在未绑定入口绕过、残缺 marker 被覆盖及 ownership 过早标记。
本轮按用户授权修复：绑定留在句柄并在提交前/事务内重验，未绑定 open/readonly/scanner/shadow
入口拒绝绑定库，打开和绑定校验在同一锁内；残缺/未知/旧实验状态拒绝而不修补。

版本改名为 `project_binding_schema_version`，session 提前接线撤回；来源账本结构保留用于
后续实现，但没有参与建图与持久化，M59-2、B5 仍未完成。禁止将本批宣称为完整 ownership。
独立测试涵盖所有入口、旧句柄提交、marker 矩阵不变性、绑定重启与 readonly、反序列化校验。

验证：CNB `cnb-6mg-1k30r3t8e` 在 `ba011b1` + cargo fmt（格式提交 `0b5f766`）
执行完整 native：1269 passed / 0 failed / 24 ignored（106 组）；benches check、
browser-wasm check、fmt check 全通过。WASM 保留 8 条非本批声明/导入位置的 warning。
[原始日志与 SHA256](../../governance/evidence/2026-09-21-project-binding-gate.json.gz)。
ignored 的真实语料/在线评测不计通过，B5 仍只是既有缺陷回归。
