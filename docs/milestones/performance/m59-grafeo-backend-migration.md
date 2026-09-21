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

独立 fallback 复审发现 unknown-version 在无绑定检查中误报 required，`fff6017` 修正，
新增 stale/needs_rebuild 断言；旧句柄三个磁盘状态读取也重验绑定。修复后完整 native
再次 1269/0/24；`1714d42` 另补 bound-readonly 不能认领无绑定旧库的断言，8 项定向测试通过。
该只读入口曾被复审误读 tuple 顺序提出 P1，源码与运行反例证伪后已撤回；
独立审查确认无剩余 P0/P1/P2 阻塞项。原始复验日志已追加同一 evidence 文件。

## 2026-09-21 M59-2 来源账本参与建图与持久化

PR42（`codex/m59-2-ownership`）本轮完成 ownership 账本闭环：
1. ownership schema v2 + `ownership_ledgers` 表：每个源文件独立 `FileContributionLedger`
   （Definition/Reference/Edge 贡献）；scanner ownership 路径按“增量解析、整图重建、
   全量持久化”从全部账本确定性重建派生图，账本与图/file states 同事务落盘，重启经
   账本表恢复；坏 TBL 保留旧账本旧图（B3 保护不变）。
2. 节点/边新增 `origin_file` 来源字段并纳入完整事实键；v2 shadow 恢复包含来源。
3. SPG 内嵌 DataFlow/dwtable 模型与字段启用页面局部 ID（`page|local`），物理表保持
   全局；身份转换失败显式报错，不静默回退旧 ID。
4. Definition/Reference 分类修正：仅源文件自身路径或 TBL 主模型/字段为 Definition；
   物理表、被嵌页面等共享目标为 Reference，占位节点在唯一引用者删除后随账本撤销回收。
5. `--project-ref` 显式项目身份接入 CLI 构图/查询/runtime 生命周期/stdio；session 与
   diff-refresh 以 manifest.project_ref 绑定。`project_binding_schema_version` 不冒充
   ownership：未绑定读取完整 ownership 库报 `GRAPH_PROJECT_BINDING_REQUIRED`，
   残缺/未知/旧实验 marker 一律 `GRAPH_OWNERSHIP_SCHEMA_STALE` 拒载，绝不补写覆盖。
6. B5 从“记录已知缺陷”改为严格验收：修改/删除被引用文件、删除唯一引用者、修复恢复、
   重启后全量与增量的节点/边/全部属性/来源字段完全一致（相同事实按 B1 完整事实键
   幂等去重，不同 field_path 的事实逐条保留），并附独立关键事实断言；新增跨页同名
   dwtable 隔离与共享物理表、共享表修改/删除/恢复、坏 TBL、重启账本恢复、
   session→query→diff-refresh 全链路 ownership 贯通等场景。B1 契约保留。

验证（CNB `cnb-beg-1k32624mn`，提交 `2a66483`）：cargo fmt --check 通过；完整 native
1275 passed / 0 failed / 24 ignored（106 组）；benches check、browser-wasm check 通过
（WASM 8 条既有非本批 warning，与本轮前持平）。
[原始日志与 SHA256](../../governance/evidence/2026-09-21-m59-2-ownership-ledger.json.gz)。
中间 workspace `cnb-kr8-1k31bevtu` 的早期定向日志随回收丢失，仅保留提交链
（054ea1f→…→f5b0f6e→46c020e→03c8ebc→413b9fa→…→5e0fb7a→2a66483）。

剩余：Grafeo/M59-3 未提前进入。

### 2026-09-21 冷脸复验修复（P1/P2）

独立只读冷脸验收判定不可验收，P1：embedsuperpage/link 为目标页创建的 stub 被无条件
标为 Definition，重建按 origin_file 字典序择一会把目标页 `origin_file` 写成 embedder
文件，且 B5 独立断言对该字段盲。修复：SPG 的 Page/Component/Condition/Action 改为
`path == 本文件` 才算 Definition（本地 dwtable 模型仍按 modelType 集合判定，其 path
是物理路径），目标页 stub 归 Reference；B5 cross-file 场景新增目标页 origin 独立断言。
P2 一并处置：spec「同 id 不兼容定义必须报告冲突」落地为 `ledger_definition_conflicts`
（与重建择一共用分组语义）+ `GRAPH_OWNERSHIP_CONFLICT` 报告诊断——脏轮由重建返回、
no-op 轮从账本重算，均随 IndexReportWithDiagnostics 透出，prepare 路径注明由后续全量
扫描报告；确定性重建补单测（重复重建一致、冲突择一确定、共享 Reference 不冲突）；
删除无调用方的 `group_ledgers` 与恒假死条件；「重复次数」表述按实现修正。

修复过程中发现并一并修正：`tbl_primary_model` 原从 JSON `name` 取值，而解析器全局
身份规则是**文件 stem**（tbl.rs），导致 TBL 主模型被误判为 Reference（占位降级风险）
且同 stem 冲突无法报告；已改为与身份规则一致。冷脸报告的“TBL 同表名静默择一”前提
据此修正为“同 stem 不同目录”。

验证（CNB `cnb-beg-1k32624mn`，提交 `2ab4fc7`）：cargo fmt --check 通过；完整 native
1279 passed / 0 failed / 24 ignored（106 组）；benches check、browser-wasm check 通过
（WASM 8 条既有非本批 warning，与本轮前持平）。B5 10/10（含目标页 origin 独立断言、
同 stem TBL 冲突报告与 no-op 重扫持续报告）、ownership 单测 5/5（确定性、冲突择一、
共享 Reference 不冲突）。
[原始日志与 SHA256](../../governance/evidence/2026-09-21-m59-2-ownership-ledger.json.gz)
（归档 SHA256 `9c8c14f4189a4f982986dacb86be21d3bb720fa804d1fcc65e07cb9b314d4ce7`，
覆盖 2ab4fc7 最终轮；2a66483 轮日志为同文件先前版本）。

第二轮独立冷脸复验（HEAD `aafc686`）：结论**可验收**，无 P0/P1；7 项修复逐条核实，
B5 新断言经论证可钉死 P1 回归（分类回退必挂）。已声明不阻塞的残留：
1. `GRAPH_OWNERSHIP_CONFLICT` 未登记 `answer_impact` 权威表，按默认 none——后续登记
   为 partial 更符合语义（spec 建议项，不阻塞）；
2. prepare 路径丢弃冲突（PreparedIndexUpdate 无诊断通道），冲突在下次 scan 前不进
   查询侧诊断——journal 已声明，任何后续 scan（含 no-op）都会补报；
3. link（ActionNavigates）目标页 stub 无独立 origin 断言，与 embed 共用同一分类分支，
   整体回退由 embed 钉捕获。
