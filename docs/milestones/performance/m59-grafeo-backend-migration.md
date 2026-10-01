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

### 2026-09-22 「可验收」撤回与语义根因修复（用户授权）

上一节第二轮冷脸复验的「可验收」结论被用户**正式撤回**：CNB 实测在基线 `2a65d7b`
上复现三个根因级缺陷，均不是「缺 answer_impact 登记」类的报告缺口，而是行为错误：
1. **身份塌陷**：SPG source `id=orders`、`path=$DATA:/tables/orders.tbl` 建图"成功"，
   但 `model:app/a.spg|orders` 不存在、`model:orders` 出现 DataflowInput 自环。
   根因：scanner 先按旧全局 ID 建临时图、再事后转换页面身份——局部 source 与同名
   物理表在临时图里已塌成一个节点（`path`/`meta` 后写覆盖前写），信息在任何转换
   之前已丢失。
2. **checkpoint 原子性违约**：prepare 把坏 TBL 转成诊断后仍返回成功，diff-refresh
   随后提交 `next_checkpoint`——事件被消费但内容从未入图，违反 approved spec
   「解析失败保留旧贡献并保持脏；失败不得推进 checkpoint」
   （[semantic-integrity-design](../../specs/2026-09-21-semantic-integrity-design.md)
   「M59-2 必须消费的归属契约」）。
3. **冲突被丢弃**：不同目录两个同 stem `a.tbl` 建图时报告 `GRAPH_OWNERSHIP_CONFLICT`，
   但 query diagnostics 与 status.load_diagnostics 均为空；prepare 也丢弃冲突。

修复（`46bc08a..2e0222c`；身份面 `46bc08a..1948885` 为上一轮 worker 实现，本轮
`1e83017..2e0222c` 为失败回归、收口修复与 P2 处置）：
1. **身份**：`PageScope` 在**建节点之前**区分页面局部与物理身份（局部 source 的
   model/field 带 `|<page>` 段；`physical_*_id` 恒全局，即使与 source id 同名）；
   写入边界 `add_identified_node` 只做保留分隔符歧义拒绝、不再二次转换；账本归属
   按 id 页面段判定（`is_page_local`），删除事后 ID 改写。回归：三个反例（同名、
   跨页同名、source 名撞另一 source 物理表名）独立预期断言 + full==incremental
   逐属性快照一致 + 同名场景 Reads/FieldAlias 端点精确四分且无自环。
2. **checkpoint**：`PreparedIndexUpdate.parse_failures` 透出；失败轮
   `commit.checkpoint` 停在旧值（bootstrap 失败轮为 None），deferred 路径
   `merge_pending_state` 同步收 `Option`——修复前 deferred **bootstrap 失败轮**
   会把 pending 水位推进到失败水位，下一轮 poll 空转，事件被静默消费、只能等
   未来新事件偶然触发重试。失败文件不写新 file hash ⇒ 文件保持脏 ⇒ 重试，链条
   闭合；同步/延迟持久化、空 poll、重试、恢复、重启均有动态回归。
3. **冲突**：scan/prepare 双入口透出 `GRAPH_OWNERSHIP_CONFLICT`；runtime 加载与
   install 均从账本重算并同步 `load_diagnostics`（status、每条查询响应、重启一致）；
   修复后随账本重算消失；空 poll/空 bootstrap 轮报告不再硬编码空冲突。

反例修复前后（CNB workspace `cnb-efo-1k335h6jk`，远端用修复前源码直接对照）：
- `deferred_bootstrap_bad_tbl_rebootstraps_until_fixed`：修复前 FAILED
  （轮 2 change_count=0，事件被静默消费）→ 修复后 ok（7/7）。
- `empty_poll_reports_persisting_ownership_conflicts`：修复前 FAILED（报告 `[]`）
  → 修复后 ok。

验证（提交 `2e0222c`）：cargo fmt --check 通过；完整 native（llvm-cov，
`--test-threads=1`）**1294 passed / 0 failed / 24 ignored（107 组）**；benches
check、browser-wasm check 通过（WASM 8 条既有非本批 warning，与本轮前持平）。
[原始日志与 SHA256](../../governance/evidence/2026-09-22-m59-2-semantic-fix.json.gz)
（文件 SHA256 `b59a1b49224d4802e99a075b9b20063960fbec7a16b0e178000ad02bcf0055f2`）。

独立验收：两轮冷脸（第一轮覆盖 `2a65d7b..fee3afe` 全部修复；第二轮复验
`fee3afe..2e0222c` 增量）均结论**可验收**，无 P0/P1。P2 处置：空轮冲突报告、
冲突清除常量化、陈旧注释、bootstrap 空 ChangeSet 前提声明四项已修；
`aa8a44c` 提交消息标 `test:` 实含 orchestrator fix，按不改写历史处理、此处勘误。

剩余与边界（明确区分已实现 / 已验收）：
- full==incremental 快照未含坏文件/冲突 fixture；
- 基线遗留：link-action 跨页 param id 与本页 param id 两种形态并存、
  `submit_meta` 重复 `"source_expr"` 键——本批未动；
- `GRAPH_OWNERSHIP_CONFLICT` 的 `answer_impact` 已在 diagnostics 权威表登记为
  partial（覆盖上一节残留第 1 项）；
- 性能边界保持「增量解析、整图重建、全量持久化」，本批**不宣称性能改善**；
  Baseline impact：新增回归用例增加测试时长，不影响 bench 目标代码路径。

## 2026-09-22 只读复核两个 P1 的根因修复（PR #42，`2aa5f5a` → `0725c80`）

上一节把「旧 target 未接线」记为不计入 M59-2 的后续任务、把「真实 bootstrap 重试」
交给忽略 manifest 的强制重投桩覆盖。本地只读复核指出两项都不成立：approved plan
（[2026-09-06 实施计划](../../plans/2026-09-06-m59-grafeo-implementation-plan.md):25-32）
要求旧 target 显式解析 + 歧义诊断与 A1/A4 **配套发布**；而 `BootstrapReplaySource`
忽略 manifest 强制重投，其绿色不能证明生产重试正确。本轮按「先最小失败回归、再修
根因、按包提交」执行，两个 P1 均已修复；实施与验收由本地主线程完成，远程 NPC 的
两 P1 方案补丁（因 token 无 `repo-code:rw` 未能推送）经审阅后按包顺序本地集成。

### P1-1：真实 bootstrap 源的失败重试（反例 `c71d87d` → 修复 `190814b`）

根因（源码链路）：`orchestrator.rs` 在 prepare **之前** `apply_changeset_to_mirror`
+ `write_manifest`，`mirror.rs` 把本轮拉取内容的 revision/hash 写进 manifest。
TBL 解析失败时 `commit.checkpoint` 保持 `None`（bootstrap 轮），但 manifest 里该
文件的 revision/path 已是最新值。下一轮真实 `BiMetaFilesChangeSource::bootstrap`
按 `file_id` 对照 manifest 的 `revision + source_path` 判 `unchanged` ⇒ 同一批
事件不再投递 ⇒ `change_count == 0` ⇒ 走空 ChangeSet 分支直接
`persist_with_checkpoint`（无 prepare）。**失败文件从未入图，水位却推进了**。
触发条件只需远端快照不变，不需要远端清空文件。

修复（`190814b`）：manifest 区分「镜像已获取」与「图已成功索引」两个状态——
- `RemoteSessionFile.indexed_hash`：最近一次**成功解析并入图**的内容 hash；
  `needs_index_retry()` 是判断是否重投的权威入口（hash 与 indexed_hash 失配即需
  重投，删除墓碑不参与）。
- `bi_meta_files_source` / `fixture_source`：bootstrap 的 `unchanged` 判定增加
  `!entry.needs_index_retry()`。
- `orchestrator`：prepare 之后按本轮**成功解析**的文件推进 `indexed_hash` 并写回
  manifest（`advance_indexed_hashes`）；解析失败文件保持失配 ⇒ 下一轮重投。
- `mirror`：改名时先取走旧记录的 `indexed_hash`（`retain` 会摘掉旧记录，按 index
  再取会越界）。
- 旧 manifest 反序列化 `indexed_hash=None` ⇒ **无 checkpoint 时下一轮 bootstrap
  全量重投一次**后收敛（已有 checkpoint 的升级不触发额外重投）。`190814b` 提交
  信息中「按已索引处理，保留旧行为」的说法与代码相反，`0725c80` 已更正源码注释，
  按不改写历史处理、此处勘误。

### P1-2：旧 target 的生产接线（反例 `92d19ce` → 修复 `4f45a93`）

根因：`resolve_node_target` 无任何生产调用方，`query/model.rs` 与 `explain.rs`
都对 target 做精确 `get_node`。页面局部身份启用后，裸 `model:orders` 在只有局部
节点时 missing；有同名物理模型时**静默命中物理模型**，跨页/跨形态同名的事实被吞
且不报歧义。CLI 路由层 `normalize_target_against_graph` 对精确存在的 id 判 `Exact`
直通，因此 runtime 层才是权威修复点。

修复（`4f45a93`）：`query/model.rs` 新增 `resolve_legacy_model_target`，前置解析
`model:` / `field:` 裸名（含 `|` 的 scoped id 仍走精确查表）——唯一局部命中改用
真实 id 并如实回报 `query_target`；多候选交回全部候选 + `AMBIGUOUS_TARGET` 诊断
（六字段信封）+ 可执行的 `next_queries`；无候选维持 `TARGET_NOT_FOUND`。
`build_query_model_output` 与 `query_model`（human 分支，纵深防御——CLI human
模式实际落 `HUMAN_MODE_NOT_SUPPORTED`，`4f45a93` 提交信息对该入口的「CLI 不绕过」
描述过强，同此勘误）与 `build_explain_output`（`field:` 的生产入口）共用同一套
解析；stdio 经 `GraphRuntime::query` 汇合与 CLI 一致，kind 隔离由解析器保证。
`AMBIGUOUS_TARGET` 与既有 `AMBIGUOUS_TARGET_ANSWERED` 均已在 `answer_effect` 登记。

### 反例修复前后（修复前 FAIL / 修复后 PASS）

| 用例 | 修复前 | 修复后 | 证据 SHA |
|------|--------|--------|----------|
| `real_bootstrap_source_retries_failed_file_without_new_events` | FAILED（轮 2 `change_count=0`） | ok | `c71d87d` → `190814b` |
| `deferred_mode_real_bootstrap_keeps_retryable_until_fixed` | FAILED（第 2 轮 `change_count=0`） | ok | `c71d87d` → `190814b` |
| `restart_after_failed_bootstrap_still_redelivers` | FAILED（`change_count=0`） | ok | `c71d87d` → `190814b` |
| `unchanged_snapshot_after_success_is_not_redelivered` | ok（对照：已入图且快照一致不重投） | ok | `c71d87d` → `190814b` |
| `bare_target_with_single_page_local_model_resolves` | FAILED（`query_target` 仍是 `model:orders`） | ok | `92d19ce` → `4f45a93` |
| `bare_target_with_same_local_name_on_two_pages_is_ambiguous` | FAILED（无 `AMBIGUOUS_TARGET`） | ok | `92d19ce` → `4f45a93` |
| `bare_target_matching_both_local_and_physical_is_ambiguous` | FAILED（静默命中物理模型） | ok | `92d19ce` → `4f45a93` |
| `scoped_target_hits_exactly_without_ambiguity` | ok | ok | `92d19ce` → `4f45a93` |
| `field_and_model_kinds_do_not_cross_match` | —（`4f45a93` 随修复加入） | ok | `4f45a93` |
| `stdio_entry_uses_same_legacy_target_resolution` | —（`4f45a93` 随修复加入） | ok | `4f45a93` |

修复前 FAIL 即「只上反例、不改生产代码」的独立反证（test-only 提交上跑出，等价于
revert 源码保留测试）：P1-1 用**真实** `BiMetaFilesChangeSource` + 可控
transport/provider，不用忽略 manifest 的强制重投桩；P1-2 走生产 `GraphRuntime::query`
与共享 stdio 入口，不直接测 resolver helper。

### 验证（CNB workspace `cnb-6i8-1k345c7ao`，最终源码 `0725c80`）

原始日志（workspace 远端 `/tmp/`）：

- `c71d87d`（包 A 反例，修复前）：`cargo test --features cli-local --test
  m59_2_bootstrap_retry_tests -- --test-threads=1` → **3 FAILED / 1 passed**
  （3 个失败均为轮 2 `change_count` 0≠1；对照用例 PASS），`CARGO_EXIT=101`，
  日志 `m59-A-prefix-fail.log`。
- `190814b`（包 B 修复后）：bootstrap_retry **4/4**；`m54_diff_refresh_orchestrator`
  12、`m54_diff_refresh_fixture` 4、`m55_meta_files_source` 15、`m55_bi_real_fixture`
  5、`m59_2_refresh_checkpoint` 8 全过；`cargo check --tests` 通过；
  `cargo fmt --check` / `cargo check --benches` / browser-wasm check 全过；完整
  native **1300 passed / 0 failed / 24 ignored**。
- `92d19ce`（包 C 反例，修复前）：`m59_2_target_resolution_tests` → **3 FAILED /
  1 passed**（无 `AMBIGUOUS_TARGET`、静默命中物理模型、唯一局部命中 `query_target`
  错；scoped 对照 PASS），`CARGO_EXIT=101`，日志 `m59-C1-prefix-fail.log`。
- `4f45a93`（包 C 修复后）：target_resolution **6/6**；`core_feature` 41、
  `explain` 8、`m58_3_command_surface` 27、`m59_a1_identity_grammar` 10、
  `output_parser` 15（1 ignored）、`stdio_server` 36（1 ignored）全过；
  fmt / browser-wasm check 全过。
- 最终门禁（`0725c80`，注释修正不改行为）：`cargo fmt --check`、`cargo check
  --benches`、`cargo check --no-default-features --features browser-wasm --target
  wasm32-unknown-unknown` 全通过；完整 native `cargo test --features cli-local
  --no-fail-fast -- --test-threads=1` → **1306 passed / 0 failed / 24 ignored**。

环境：CNB 云原生工作区 `cnb-6i8-1k345c7ao`（分支 `codex/m59-2-ownership`，SSH 远程
执行，工作区 SHA 与本地提交逐个精确复现一致），非维护者本机；不复用 `1948885` /
`2e0222c` 绿灯作本轮证据。PR CI 以平台实际状态为准。

### 独立验收

独立只读 reviewer（forensics，冷脸）复核 `2aa5f5a..4f45a93` 全量 diff 与源码链路：
**无 P0**；1 项 P1——`src/session/manifest.rs` 结构体注释「旧 manifest 首次升级按
已索引处理」与代码相反且与同块字段注释互斥——已在 `0725c80` 修正后复核关闭。
P2 处置：崩溃窗口、`severity_for` 未显式登记 `AMBIGUOUS_TARGET`、歧义出口按 surface
并存、SPG 整轮失败×重投无用例、升级轮一次性成本、边端点断言缺口——记入下方边界，
单列 follow-up，不在本包扩大修复。

### 剩余与边界

- manifest 写回先于图持久化：崩溃恰落两写之间且**首次 bootstrap（无 checkpoint）**
  时，durable manifest 已标 indexed 而 durable 图未更新，需远端再变更才重解析
  （有 checkpoint 的会话由水位重投 + hash-dirty 重检自愈）——follow-up 未修；
- `AMBIGUOUS_TARGET` 在 `severity_for` 未显式登记（落默认 Warning 档；兄弟码
  `TARGET_NOT_FOUND`=Error、`AMBIGUOUS_TARGET_ANSWERED`=Info）；六字段信封与
  `answer_effect` 已齐；
- 歧义出口按 surface 并存三种形态：CLI route 层 ≤3 候选逐候选作答
  （`AMBIGUOUS_TARGET_ANSWERED`）、>3 候选 `ambiguous_target_error`、runtime
  resolver `AMBIGUOUS_TARGET`——均登记 `answer_effect`；本轮新用例未起真实二进制
  （main 归一层交互由读代码 + 既有 `m58_3_command_surface` CLI 子套件佐证）；
- SPG 整轮失败 × 重投：prepare `Err` 时 `advance` 不执行、每轮重投每轮响亮报错
  直到远端修复（刻意行为），无专门用例；
- 升级轮成本：旧 manifest 无 checkpoint 时首轮全量重投为一次性远端内容拉取，
  mirror hash 未变不重写、prepare 无脏不重解析，无每轮退化；
- 新回归断言到节点 id / 类型 / `query_target` / candidate 集合与诊断码，未断言边端点；
- full==incremental 快照仍未含「坏文件 + 重试恢复」fixture；sync 模式 bootstrap
  失败轮已由 `real_bootstrap_source_retries_failed_file_without_new_events`
  （one_shot=Synchronous）与 deferred 用例分别覆盖；
- 旧 target 裸名解析覆盖 `model:` / `field:` 两类；`cond`/`comp`/`action`/`param`
  文法恒为 scoped，裸名解析对它们未定义（返回 Missing），与 A1 文法一致；
- 性能边界保持「增量解析、整图重建、全量持久化」，本轮**不宣称性能改善**。

## 2026-09-22 对抗性复验：两 P1 修复的残余缺口（`fac4747` → `a6545cf`）

上一节的修复方向正确，但本轮对 `190814b` / `4f45a93` 做对抗性审查（不信任既有绿灯、
逐条找反例）时发现两条 P1 各自还剩一个**同类**缺口：修复把「已索引」的判据推进了
一步，却都停在「还差最后一步」的位置。两处均先补独立失败反例、再修根因。

### 残余 P1-1：`indexed_hash` 只证明 prepare，不证明 durable

根因：`advance_indexed_hashes` 在 `orchestrator.rs:472` 于 **durable persist
之前**（`484+`）执行并写 manifest。于是 `indexed_hash` 的语义实际是「内存候选图
prepare 成功」，而不是「图已成功索引」——两个状态之间还隔着 persist。

- Synchronous/one-shot（`main.rs:1622` 的生产模式）：persist 失败 ⇒ `refresh_once`
  返回 `Err`，但 manifest 已把该文件记为已索引。下一轮 durable 无 checkpoint ⇒
  bootstrap ⇒ 源判 unchanged ⇒ `change_count == 0` ⇒ 空 ChangeSet 分支写下
  bootstrap 水位 ⇒ **文件从未进入 durable 图，水位却推过了它**。
- Deferred：`install_replacement` 只换内存图，落盘等 pending 阈值；此间重启丢掉
  pending，manifest 已称已索引 ⇒ 同一条永久丢失路径。

反例（`3d4069c`，修复前 2 FAILED / 4 passed）：
- `sync_persist_failure_keeps_file_retryable_until_durable`
  → `持久化失败的文件不得被记为已索引：hash=Some("c3be970033a1fee5") indexed_hash=Some("c3be970033a1fee5")`
- `deferred_restart_before_persist_keeps_file_retryable`
  → `重启后未 durable 的文件必须重投（实际 0）`

修复（`37c5a61`）拆成两阶段，三处落库点全部接线（同步 persist、延迟
`persist_pending`、空 ChangeSet 轮的 pending 落库）：
- `stage_indexed_paths`（prepare 后）：只登记**逻辑路径**到
  `pending_indexed_paths`，不写 manifest；本轮解析失败的路径从登记中**撤下**
  （deferred 下可能是上一轮登记的，而 `manifest.hash` 已指向新内容）。
- `commit_indexed_hashes`（durable 落库成功后）：把已登记路径的 `indexed_hash`
  推进为当前镜像 hash 并写 manifest。

空 ChangeSet 分支的注释从「前提声明」改为可证不变式：空 Changeset 要求全部
active 文件被判 unchanged，而 `needs_index_retry()` 为真的文件必被判 changed，
因此该分支写下的 bootstrap 水位不可能越过未入图文件。

修复后：`m59_2_bootstrap_retry_tests` 6/6；连带回归
`m59_2_refresh_checkpoint_tests` 8/8、`m59_2_identity_and_refresh_tests` 7/7、
`m54_diff_refresh_orchestrator_tests` 12/12、`m54_diff_refresh_fixture_tests` 4/4、
`m55_meta_files_source_tests` 15/15、`m55_bi_real_fixture_tests` 5/5、
`m58_3_pr2_diff_refresh_scanner_persistence_tests` 5/5、
`m59_b3_tbl_parse_failure_tests` 5/5、`m59_b5_full_vs_incremental_diff_tests` 10/10。

### 残余 P1-2：`--explain` 的两条补充调用仍绕过解析

根因：`resolve_legacy_model_target` 只接到 `build_query_model_output` /
`query_model(human)` / `build_explain_output`，但生产 `--explain` 在
`route.rs:120-133` 展开成**三条**调用：

```
Explain（主）+ ExplainCondition（补充 condition_facts）
            +（显式 --depth 时）Context（补充 neighbor_context）
```

后两条仍做精确 `get_node`（`explain.rs:531`、`context.rs:164`）。页面局部身份
启用后，裸 `field:`/`model:` 在这里报 `TARGET_NOT_FOUND`，而 supplement 的
`required=false` 让整条命令照样「成功」——主调用有答案，条件成因与邻居闭包
**静默缺失**，且没有任何可见失败信号。这比原 P1 更隐蔽：原 P1 是整条命令失败，
这里是半条命令静默变空。

反例（`c26334f`，修复前 3 FAILED / 6 passed）：
- `explain_condition_supplement_resolves_bare_legacy_target`
  → `["TARGET_NOT_FOUND", "EVIDENCE_INCOMPLETE"]`
- `context_supplement_resolves_bare_legacy_target`
  → `["TARGET_NOT_FOUND", "EVIDENCE_INCOMPLETE"]`
- `explain_condition_supplement_reports_ambiguity`
  → `["TARGET_NOT_FOUND", "EVIDENCE_INCOMPLETE"]`

修复（`64159b8`）：两处入口前置同一套 `resolve_legacy_model_target`；唯一局部
命中改用真实 id 并如实回报 `query_target`，多候选返回全部候选 +
`AMBIGUOUS_TARGET` + 可执行 `next_queries`，无候选维持 `TARGET_NOT_FOUND`。
歧义响应仍走统一信封诊断，经 runtime 的 `load_diagnostics` / answer effect
合并（`AMBIGUOUS_TARGET` 登记为 Addressing，不降 confidence level）。

修复后：`m59_2_target_resolution_tests` 9/9（新增 3 条）。

### 独立只读复核（forensics，冷脸，`50946ad..64159b8`）

复核确认两条修复主结论**成立**，并逐条验证了本轮关心的反例路径：
- 「本轮成功登记 → 下一轮同文件变更且解析失败」确实被撤下（`retain` 在 staging
  之前执行，且镜像内容变更必然变脏并被重新解析 ⇒ 失败必然进 `parse_failures`）；
- 空 ChangeSet 分支未接线 `commit_indexed_hashes` 是**语义空操作**（该分支可达
  要求全部文件 `hash == indexed_hash`，写入值相同）；
- 未发现 P0。

给出 5 项 P2，本轮处理 3 项（`a6545cf`）：
- **P2-1**：manifest 写失败会中断一次**已经成功**的提交（persist 成功但
  `commit_indexed_hashes` 的 `?` 先于 `install_replacement`/`clear_pending_state`
  传播，tick 循环判其不可重试直接返回 Err，本进程留下旧图配新 checkpoint）。
  改为先 install/clear 再写 manifest：manifest 是「已索引到哪」的缓存，durable
  才是事实来源；失败时磁盘 manifest 保持保守（`indexed_hash != hash` ⇒ 下次启动
  重投，只多解析一次）。
- **P2-3**：`pending_indexed_paths` 由 `Vec` 改 `HashSet`，撤下/合并/求值由
  O(n·m) 降为 O(n+m)，行为不变。
- **P2-4**：`AMBIGUOUS_TARGET` 此前落默认 Warning 档，而同属寻址失败的
  `TARGET_NOT_FOUND` 显式 Error、`answer_effect` 把两者并列登记为 Addressing。
  M59-2 A1 让命令层成为该 code 的首个信封构造方，不一致第一次可观测；登记为
  Error 并在 `m58_3_pr1_severity_tests` 钉住。

**未采纳 2 项，理由如下**（两项均经第二轮独立复核确认处置正确）：
- P2-5「staging 应过滤非 spg/tbl 扩展名」——**不采纳，因为该修法会引入更严重的
  缺陷**。非 spg/tbl 文件永远不会被 scanner 入图；若把它们排除出 staging，
  `needs_index_retry()` 对该记录**永久为真**（`manifest.rs:49-61`），而两个
  bootstrap 源都把它作为 `unchanged` 的必要条件（`fixture_source.rs:154-159`、
  `bi_meta_files_source.rs:421-428`）⇒ 每一轮 bootstrap 都重投同一批文件，造成
  **无限重投**，比被指出的问题更严重。
  被指出的原始风险（把从未入图的内容标成已索引）**在生产路径不可达**：manifest
  的 `hash=Some` 只由 `mirror.rs::apply_active_event` 与 `sync.rs::sync_one_file`
  写入，两者的输入都已过滤不可分析文件（`bi_meta_files_source.rs:314/399` 的
  `is_analyzable`、`remote_sync.rs:260` 的 `is_analyzable_entry`）；scanner 也只
  发现 spg/tbl（`indexer.rs:538-539`）。只有 `FixtureMetaFilesChangeSource`
  （无扩展名过滤）能造出该记录，而它**没有生产调用方**（`main.rs:1614` 与
  `stdio_server.rs:224` 都只用 `BiMetaFilesChangeSource`；fixture 源仅在
  `tests/` 下使用，`mod.rs:30` 只是再导出）。因此保持现状；若将来出现不过滤
  扩展名的新源，需要在**源侧**补过滤，而不是在 staging 侧排除。
- P2-2 `OutputKind::Context` 缺 `find_cmd` 分支（落到 `--find-page` 兜底）：
  `git show 64159b8^:src/context.rs` 证实改动前该路径（旧 168-170 行）已用同一
  兜底，属既有行为、非本次引入；不在本包扩大修复范围，记入下方边界。

### 第二轮独立只读复核（`a6545cf`）

对「3 修 2 不采纳」再做一次冷脸复核（只读、不改），结论：

- **Fix 1（P2-1 顺序调整）HOLDS**：manifest 写失败时，durable 已提交、runtime 已
  持有候选图，二者一致；磁盘 manifest 保守（`indexed_hash != hash`）⇒ 下次启动
  重投。三处 `commit_indexed_hashes` 调用点都仍在 durable persist 之后；
  `persisted`/`persist_report` 先于 `install_replacement` 的既有不变式未被破坏；
  `clear_pending_state` 未清掉后续仍需的状态（`pending_indexed_paths` 是独立字段）。
- **Fix 2（Vec → HashSet）HOLDS 且语义等价**：撤下失败路径、合并登记、commit 求值
  三者集合语义一致；唯一丢失的是 `pending_indexed_paths` 的插入顺序，而它从不被
  观测（唯一消费方按 `manifest.files` 迭代做成员判断）。
- **Fix 3（severity）HOLDS**：`AMBIGUOUS_TARGET` 作为 `TARGET_NOT_FOUND` 同一
  or-pattern 的尾部替代项，未被 `AMBIGUOUS_RESOLUTION` / `AMBIGUOUS_TARGET_ANSWERED`
  两个 Info 分支按字面量精确匹配截获。severity=Error 配 `answer_impact=none` 与
  `TARGET_NOT_FOUND` 的既有组合**完全一致**（两者都经 `answer_effect` 登记为
  Addressing ⇒ 派生 `IMPACT_NONE`），是恢复兄弟码先例而非引入新组合。
- **两项不采纳均确认处置正确**：P2-5 的无限重投机制经 `manifest.rs:49-61` 与两个
  bootstrap 源的 `unchanged` 判定独立验证成立，且生产不可达（fixture 源无生产
  调用方）；P2-2 的「既有行为」分类经 `git show 64159b8^:src/context.rs` 确认。

### 验证环境与 SHA

- 环境：CNB 云原生工作区 `cnb-gf8-1k3515v0e`（分支 `codex/m59-2-ownership`，
  rustc 1.95.0 / cargo 1.95.0，与 CI 镜像同版本工具链），SSH 远程执行；
  **不在维护者本机跑 cargo**。
- 反例与修复均逐 SHA 检出验证：FAIL 证据取自 `50946ad` / `c26334f`（仅测试），
  PASS 证据取自 `37c5a61` / `64159b8`（含修复）。
- **最终全量门禁**（`a6545cf`，`cargo fmt --check` 通过）：
  `cargo test --features cli-local --no-fail-fast -- --test-threads=1`
  → **1311 passed / 0 failed / 24 ignored**，`CARGO_EXIT=0`，110 个测试二进制
  （`gate.log` 2605 行）；新套件 `m59_2_bootstrap_retry_tests` /
  `m59_2_target_resolution_tests` / `m58_3_pr1_severity_tests` 均在本次门禁内运行。
  相对上一轮基线 1306 passed 的 +5 即本轮新增用例。
  `cargo check --no-default-features --features browser-wasm --target
  wasm32-unknown-unknown` → `WASM_EXIT=0`（8 条既有 warning，持平）。
  24 ignored 为真实语料/在线评测，不计验收。
- 工作区 `cnb-pm8-1k33o0ipu`（上一轮 reviewer）已不存在（`get-workspace-detail`
  返回 404），其 `/tmp/m59-review-2aa5f5a` 反例不可复用；本轮新建工作区重做。

### 本轮新增/更新的未关闭边界

- ~~`--context` 的 `TARGET_NOT_FOUND` 兜底仍给 `--find-page` 建议~~：已在
  `fe758b9` 修复（见下一节）；
- ~~裸 `--explain` 一次调用对同一 target 做 3 次整图 `iter_nodes` 扫描~~：已在
  `b810058` 收敛（见下一节）；
- ~~新增回归仍经 `RuntimeQueryRequest` 直达，未覆盖 `main.rs` 的
  `merge_supplement` / `attach_confidence`~~：已补 CLI 子进程用例（见下一节）；
- ~~SPG 整轮失败 × 重投无专门用例~~：已补（见下一节）；
- ~~full==incremental 快照未含「坏文件 + 重试恢复」~~：已补，且把语义钉成显式
  预期（见下一节）；
- 旧 target 裸名解析覆盖 `model:` / `field:` 两类；`cond`/`comp`/`action`/`param`
  文法恒为 scoped，裸名解析对它们未定义（返回 Missing），与 A1 文法一致；
- 性能边界保持「增量解析、整图重建、全量持久化」，本轮**不宣称性能改善**。

## 2026-09-22 未关闭边界收口（`6b0be5a` → `027c4f6`）

上一节把 6 项记为「未关闭边界 / follow-up」。本轮逐项收口，其中 **2 项是真缺陷**
（已修），**4 项是缺测试**（已补）。每项都先测量/取证再动手。

### 1. `TARGET_NOT_FOUND` 的补救建议按错误的节点类型给（真缺陷，`fe758b9`）

根因：`build_target_not_found_output` 由 **`OutputKind`** 推 `--find-*`，而节点种类
其实由 **target 前缀**决定。`OutputKind::Context` 落 `_ => --find-page`，
`OutputKind::Explain` 落 `--find-component`：

```
--context model:definitelyMissing  → 建议 --find-page model:definitelyMissing
```

搜的是 Page 类型，而 target 是 model —— 一条都搜不到。关键词还带前缀：
`find_nodes` 拿关键词匹配 `id`/`name`/`path` 的子串，而 id 形如
`model:app/a.spg|ordersView`，`model:ordersView` 从不是任何 id 的子串，**即使命令
类型对了也搜不到**。

修复：`find_command_for_target` 先按 target 前缀定命令与关键词（去前缀；`field:`
退到所属模型名，因为字段没有独立 find 动词），无前缀（裸名）时才退回按
`OutputKind` 推断。

反例（仅回退 `src/output/schema.rs`、保留新断言）：
```
target_not_found_suggests_find_command_matching_target_kind →
Context 的 TARGET_NOT_FOUND 必须建议 --find-model（按 model: 前缀），
实际 ["--find-page model:definitelyMissing"]     9 passed / 1 failed
```
修复后 11/11。

### 2. 补充调用在主调用已定位失败后仍重复整图解析（真缺陷，`b810058` + 修正 `7d4934c`）

`--explain`/`--relations` 展开的补充调用与主调用拿**同一个** target。主调用已判
`TARGET_NOT_FOUND` / `AMBIGUOUS_TARGET` 时继续发补充调用，只会重复整图解析（裸
target 每次都要 `iter_nodes`）再产出一份同样的壳子，最后被 `dedupe_diagnostics`
丢掉。`b810058` 加了这个跳过。

**但第一版跳过的前提是错的，且由独立复核发现（`7d4934c` 修正）**：`b810058` 假设
「补充调用必然得到同一个定位结果」。这对 `--relations model:X` **不成立**——
主调用 `QueryModel` 已接线 resolver，补充调用 `QueryDataflow` **未接线**、直接精确
`get_node`：

| | 裸 `model:orders`（局部 + 物理同名） |
|---|---|
| 主调用 `QueryModel` | 如实报 `AMBIGUOUS_TARGET` |
| 补充调用 `QueryDataflow` | **静默命中物理模型**，返回它的 DataFlow 子图 |

于是跳过会把那份**有分歧的结果**一起吞掉，而且没有可见痕迹（连
`SUPPLEMENT_UNAVAILABLE` 都不记）。实测反例
（`dataflow_supplement_agrees_with_primary_on_ambiguous_target`）：
```
补充调用必须与主调用同样报 AMBIGUOUS_TARGET，而不是静默命中物理模型；实际 []
```
（空诊断 = 静默命中物理模型。）

两处修正：
1. `build_query_dataflow_output` 前置同一套 `resolve_legacy_model_target`——从根上
   消除「同一个 target、两个子调用给出矛盾定位结论」，这同时修掉了静默挑物理模型
   的行为（与 M59-2 A1 的目标一致），使主/补充结论**真正**一致；
2. 跳过条件加**命令白名单** `shares_legacy_target_resolution`：只跳过已接线同一套
   解析的命令（`Explain`/`ExplainCondition`/`Context`/`QueryModel`/`QueryDataflow`）。
   未接线的命令照常执行——宁可多跑一次，也不静默吞掉一个可能给出不同答案的补充块。
   不再依赖「所有补充调用都同意」这个未经证明的假设。

**暴露面经实测收窄**（`route_layer_normalizes_bare_targets_before_runtime`）：
路由层在 runtime 之前就把大多数裸 target 归一成 scoped id——

```
裸 model:ordersView          → Resolved(model:app/a.spg|ordersView)   ⇒ runtime 零扫描
裸 field:ordersView.order_id → Resolved(field:app/a.spg|ordersView.order_id) ⇒ 零扫描
scoped id                    → Exact                                  ⇒ 零扫描
```

因此 runtime 侧整图解析只在路由层**没能归一**时发生（`Exact` 直通后仍需查同名局部
节点以判歧义；或归一落空）。这也纠正了上一节「3 次扫描」的粗估——它不是每次
`--explain` 的固定成本，而是路由层归一失败时才付。

### 3. CLI 合并层此前无端到端覆盖（缺测试）

`merge_supplement` / `attach_confidence` 只在读代码层面被论证过。新增
`test_ambiguous_target_envelope_survives_cli_merge_layer`：起**真实二进制**跑
`--explain comp:button1`（fixture 里跨两页重名），断言歧义诊断经合并层后仍是六字段
信封、`answer_impact` 与 `severity` 已登记、confidence 块由合并层补上。

过程中修正了一处我自己的错误预期：`AMBIGUOUS_TARGET_ANSWERED` 的
`answer_impact` 是 **partial** 而非 none——工具已逐候选作答，结论只覆盖其中一个
节点，与「纯寻址失败」的 `AMBIGUOUS_TARGET`（none）语义不同。测试改为按 code 分别
断言，把两者的差别钉住而不是抹平。

### 4. 坏 SPG 整轮失败 × 重投（缺测试）

`indexer.rs` 对 `.spg` 解析失败直接 `?` 上抛（整轮 `Err`），只有 `.tbl` 走
`ParseFailure`。新增 `bad_spg_fails_whole_round_and_recovers_after_fix`：失败轮
返回 `Err`、durable 无水位、旧图保留；修好后正常入图并推进。

过程中发现测试基础设施的一个真实约束：整轮失败会**中途 abort**，按队列顺序消费的
`QueuedProvider` 不可靠（第二轮取不到内容）。为此引入按**路径**提供内容的
`PathProvider`（`Rc<RefCell<HashMap>>` 句柄可在轮次间改写），并把
`build_orchestrator` 泛化为接受任意 provider。

### 5. full==incremental 的「坏文件」对照（缺测试，且原设想有误）

上一节记为「快照未含坏文件 fixture」。补测时发现**原设想是错的**：坏文件状态下
增量与从零全量重建**本来就不该相等**——

```
仅在 incremental(bad): field:orders.amount / field:orders.order_id /
                      model:orders（上一次成功解析的物理表内容）
仅在 full(bad):        model:orders（SPG 声明的 PhysicalTable 占位节点）
```

增量有「上一次成功」可保留（B3 语义），从零全量重建没有。把两者断言为相等会写出
一个**错误**的期望。改为把差异钉成显式预期：增量必须保留旧字段、全量必须没有这些
字段且留下 SPG 占位模型、两侧都不得丢掉页面节点。这样「增量保留旧图」与「全量拿
不到」各自可判，而不是含糊地都不检查。

### 验证与边界

- 上述 5 项在 `cnb-gf8-1k3515v0e` 逐项验证，命令与结论见各小节。
- **最终全量门禁**（`87461bd`，`cargo fmt --check` 通过）：
  `cargo test --features cli-local --no-fail-fast -- --test-threads=1`
  → **1315 passed / 0 failed / 24 ignored**，`CARGO_EXIT=0`。
  相对收口前基线 1311 passed 的 **+4** 即本轮新增用例
  （CLI 合并层歧义信封、坏 SPG 整轮失败、路由层归一、全量/增量坏文件语义）。
  24 ignored 为真实语料/在线评测，不计验收。
- 未做真实性能测量：第 2 项减少了重复扫描，但**不宣称性能改善**（无可复现的前后
  测量，仅消除确定性浪费）。
- 仍未关闭：`cond`/`comp`/`action`/`param` 裸名解析未定义（与 A1 文法一致，属设计
  边界）；未进入 Grafeo / M59-3。

## 2026-10-01 阶段 C1：Grafeo 图存储后端接入（`6ef2db9` → HEAD）

### 做了什么

- `Cargo.toml` 新增可选依赖 `grafeo 0.5.43`，feature 只开 `["edge", "storage"]`。
  0.5.43 的 `lpg` 已把 `gql` / `cypher` / `gremlin` / `sql-pgq` 连带拉进来——
  四个查询语言解析器本实现一个都不调用，故不开 `lpg`。
- `src/graph_grafeo.rs`：`GrafeoGraphStore` 实现 `GraphReadStore` + `GraphWriteStore`，
  全程走直写 API（`create_node_with_props` / `create_edge_with_props` /
  `set_node_property` / `get_neighbors_*` / `delete_node` / `delete_edge` /
  `iter_nodes` / `iter_edges`），**不经任何查询语言**。
- feature `grafeo-store` 为单列 feature、**不并入 `default`**（评审后调整：C1 尚无
  生产调用点，默认构建不为未接线引擎付编译代价；C2 接线时再并入）。
  `cargo test --features cli-local` 即「不带 grafeo」的对照口径，
  `--features cli-local,grafeo-store` 才跑 grafeo 实现。
- `AGENTS.md`：按 spec §0 解除 10MB 硬闸门，改为「每次影响依赖/feature 的 PR
  须记录实测体积与变化量」。

### 两个必须自己兜住的语义

1. **应用 ID ≠ Grafeo `NodeId`**。Grafeo 的 `NodeId` 由库分配，应用侧稳定标识是
   字符串 `id`，因此存 `id` 属性 + 建 property index，并在内存维护
   `HashMap<String, NodeId>`；重开库时用 `iter_nodes` 重建该映射。
2. **删节点不保证级联删边**。`remove_nodes_by_ids` 显式先删出边与入边再删节点，
   并清掉涉及已删节点的去重键——否则「删节点后重建同一条边」会被旧去重键吞掉。

去重键与 meta 合并一律复用 `graph_store.rs` 的 `edge_dedup_key` /
`merge_upsert_meta`，不让三个实现各写一份语义。

### 验证（workspace `cnb-9ok-1k3rc24lm`）

- `cargo test --features cli-local --test m59_b1_graph_store_contract_tests`
  → **45 passed / 0 failed**（15 条契约用例 × memory / redb / grafeo 三实现，
  用例体未改一行，这是 spec §2.6 规定的 C1 验收判据）。
- `cargo test --features cli-local --test m59_c1_grafeo_store_tests`
  → **8 passed / 0 failed**（全 `NodeType` / `EdgeType` 往返、嵌套与 Unicode meta、
  `None` vs JSON `null` vs 空对象、重开库恢复去重键、只读打开、缺文件报错、
  删节点后计数一致）。
- `cargo test --features cli-local` 全量 → 111 个 `test result: ok`，无失败项。
- `cargo check --no-default-features --features browser-wasm
  --target wasm32-unknown-unknown` 通过（grafeo 不进 wasm 构建）。
- `cargo fmt --check` 通过。`cargo clippy` 未跑：该 workspace 的 1.95.0 工具链
  未装 `cargo-clippy`。

### 实测体积

评审修复后同工具链复测（本机 cargo 1.97.1，`cargo build --release`）：

| 构建 | 字节 | 相对 |
|---|---|---|
| 默认（`cli-local`，不含 grafeo） | 5,942,264 | 基线 |
| `--features cli-local,grafeo-store` | 5,950,840 | **+8,576 B（+0.14%）** |

**这个 +8.6KB 不代表 grafeo 的真实体积代价**：C2 尚未把 Grafeo 接进 CLI 导入链路，
二进制里没有调用点，链接器把它整体丢掉了（只有 lib/测试目标链接）。
真实增量要等 C2 直写导入落地后重测——spike 测过的 8.86 MiB 是带四个解析器的数，
`edge + storage` 的数现在还没有。

### 边界

- 持久化路径须以 `.grafeo` 结尾：Grafeo 对新建持久路径按扩展名选单文件格式，
  只有该格式支持只读打开。
- redb 仍是默认实现，**本轮不退役**。退役是 D2，前置为 C2/C3/C4/C5 + D1 真实语料
  验收；且 `GraphDB` 目前是 native 路径上唯一的 `IndexStateStore`，C2 之前删掉
  就没有索引提交路径。
- 不宣称查询等价、不宣称性能改善：C3（22 个 `EdgeType` 查询动词）、
  C4（只读表投影 / WHERE / LIMIT / TSV / JSON）、D1（真实语料正确性与性能）均未动。

## 2026-10-01 C1 评审修复（PR #47，NPC CodeBuddy `changes_requested`）

六条行级意见的处置：

1. **`Cargo.lock` 随依赖提交**（阻塞项）：C1 三笔提交漏带 lock，与 `main` 逐字节
   相同、无 grafeo 条目，`--locked/--frozen` 构建会失败。本修复补交完整 lock
   （纯增量 grafeo 依赖树，`cargo check --locked` 验证一致）。
2. **`grafeo-store` 移出 `default`**：C1 尚无生产调用点，默认构建不应为未接线
   引擎付编译代价。改为单列 feature，C2 接线时再并入 `default`。
   测试口径相应改为 `--features cli-local,grafeo-store`。
3. **AGENTS.md 去瞬态引用**：删除对 `codex/m59-3-grafeo-store` 分支名与一次性
   字节读数的引用，改为引用 spec §0 + 「纯 redb 形态（`cli-local`）」这一稳定
   形态，并明说 Grafeo 引入是 spec §0 例外而非对「轻量级 crate」规则的放宽。
4. **`node_from_props` 口径统一**：`id`/`node_type` 缺失报 `Corrupted` 但
   `path`/`name` 用 `unwrap_or_default()` 静默填空——upsert 两条路径都必写
   这两属性，缺失同样是坏数据。四个属性统一 `ok_or_else(Corrupted)`。
5. **`add_edge` 去重键登记时机**：`seen_edges.insert` 原排在可失败的
   `create_edge_with_props` 之前，建边失败后重试会被当重复静默丢弃。
   改为落库成功后登记。
6. **补「不 close 直接 drop」恢复用例**：新增
   `drop_without_close_reopens_with_graph_and_dedup_keys`，覆盖调用方不手工
   `close()` 时 `open()` 的 WAL 回放恢复路径（节点/边/去重键断言）。

验证（本机 cargo 1.97.1）：`cargo test --features cli-local,grafeo-store
--test m59_b1_graph_store_contract_tests --test m59_c1_grafeo_store_tests`
→ 45 + 9 passed / 0 failed；`cargo fmt` 通过；`--locked` check 通过。
