# 2026-09-08 性能契约集中修复

起点：PR38 合并后的 `main=83798da`。保留 NPC 收紧的源码行引用与前轮归档证据；
[归档去向报告](branch-retention-performance-2026-09-08.md)是固定快照，本次进展追加到这里。
本批对应 M53 的[性能设计](../specs/2026-07-06-m53-performance-design.md)、
[warm-cache 设计](../specs/2026-07-07-m53-warm-cache-scalability-design.md)及
[加载期观测设计](../specs/2026-07-09-m53-init-load-observability-design.md)中的既有契约，
在一个分支内分组提交、统一 PR 验证。

## 实现与测试

- **邻接缓存**：edge bundle 使用查询持有的缓存，action flow 复用已读取的 action 邻接。
  最小图的底层 action 读取从5次降为4次；下一次查询重新读取，缓存不跨请求。
  测试覆盖图闭包与 edge bundle 的读取错误，warm 命中后比较完整页面输出。
- **报告指标**：prerequisites 使用实际 component/action/data-source scan、sort counter；
  path 使用 JSON build 与 side-context counter。测试逐个核对报告 driver 的名称和数值，
  统计汇总验证 min/max/avg，区分缺失观测与明确的零值。
- **unchanged 场景**：实际结果新增 `runtime_check_reload_reloaded` counter。
  检查移到第二次打开 graphdb 之前；若实际重载、失败或 reload_count 不为0，拒绝产生
  名为 unchanged 的样本。产品运行时的 mtime/size/hash 检测策略保持原有契约。
- **计时**：移除 availability/context/index 与 path context 的1毫秒下限。
  毫秒分辨率下合法的0被保留。计时字段存在性与实际缓存/输出计数分别断言。
- **预算**：原 fixture 明确产生非空 candidate/rejected 桶后验证 compact/normal 裁剪，
  同时检查 primary/related 桶保留；四条条件路径的最小图在保底提升后剩两条 supporting，
  验证 full/normal 保留、compact 删除，并对比有/无 warm 缓存的完整序列化 JSON 字节。
- **确定性**：保底规则原用 HashMap，遍历顺序会改变路径提升说明。
  改为显式优先级数组，依次为 action_write、cross_page_writer、data_prerequisite、
  display_gate、target_component、value_source；同图 warm/query 不再因规则随机顺序漂移。
  重叠候选测试精确约束提升后的主路径集合、剩余候选和提升说明。
- **runtime 降级**：load 复用独立的读模型构建函数。故障 store 在 dense 构建完成后
  拒绝读取，验证真实 PageDependencyIndex 构建失败时 dense 仍可用、索引为空且状态为
  Partial，并精确检查唯一诊断的 code/severity/count/answer_impact/阶段/位置及计时总和。
  正常分支验证 Full 状态与实际页面映射；注入点在生产构建函数，不是公开 load 的磁盘故障。

首轮完整 JSON 比较使用 canonicalization，会重排数组，其证据不能表述为原始字节相等。
审查返修已删除这层规范化，直接比较序列化字节，保留所有数组顺序与分类说明字段。

## 验证发现与修复过程

| 提交/探针 | 结果 | 处理 |
|---|---|---|
| `b8e61d2` 定向回归 | M51 page logic 8项通过；报告测试2通过、1失败，`reloaded=1` | 保留失败证据，`2d416e8` 将 unchanged 检查放在再次打开 redb 之前，仍要求实际结果为0 |
| `2d416e8` 定向回归 | 报告3项、M52 profile 5项、dense 1项通过；新增 supporting 用例的分类说明不相等 | `94fdb4c` 固定保底规则顺序，未放宽输出断言 |
| `94fdb4c` 定向回归 | dense 1项、warm-cache 8项、edge-cache 3项通过；真实语料手工 footprint 1项 ignored | ignored 不计通过 |
| 临时回退共享缓存 | action读取断言失败，5次不等于4次 | 恢复源码 |
| 临时改回错误 prerequisites counter 名 | cost model driver精确比较失败 | 恢复源码 |
| 临时让compact保留candidate桶 | compact零桶断言失败 | 恢复源码 |
| 临时删除runtime降级诊断写入 | 生产构建函数测试的诊断数量断言失败 | 恢复源码，重跑2项构建函数测试通过 |

四项回退探针均要求测试实际进入断言并失败，编译失败不算有效回归证据。
探针只在远端临时修改，finally 恢复原文件，不进入提交。

## 首轮统一验证与证据

代码提交：`27cbec70b7133a815e2d451eb044020cc8fe2c3d`。
验证在 CNB workspace `cnb-v1o-1k1vopn9p` 的 `/workspace` 通过 SSH login shell 执行。

- 九个定向集成套件：89项通过，2项真实语料测试 ignored。
- profile 统计、runtime 读模型构建、path 单元测试：分别2、2、1项通过。
- 额外 CLI corpus snapshot：1项通过，未重置快照。
- `cargo fmt --check`、`cargo check --benches`、
  `cargo check --no-default-features --features browser-wasm --target wasm32-unknown-unknown` 通过。
- 合计94项定向/单元测试及额外1项快照通过；ignored 和恢复源码后的重复测试不计入通过数。

[完整日志与执行脚本](evidence/performance-contract-fixes-2026-09-08.json.gz)包含16份日志、
3份执行/回退脚本、每份文件的 SHA-256、代码提交和结果分类。
压缩包 SHA-256：`1d0477b378123d47aa2472a3ba9541249c319da6f45736322a5a009b07047202`。
最终七项统一检查的日志直接记录 COMMIT、COMMAND、EXIT_CODE；较早日志缺失的退出码保留为
null，提交来源标注为会话执行记录，不补写成日志原生证据。回退命令与修改点见所附脚本。

在仓库根目录可解包核对（不执行所附脚本）：

```sh
python3 - <<'PY'
import gzip, hashlib, json
from pathlib import Path
archive = Path('docs/governance/evidence/performance-contract-fixes-2026-09-08.json.gz')
assert hashlib.sha256(archive.read_bytes()).hexdigest() == '1d0477b378123d47aa2472a3ba9541249c319da6f45736322a5a009b07047202'
evidence = json.loads(gzip.decompress(archive.read_bytes()))
for item in evidence['logs'] + evidence['scripts']:
    assert hashlib.sha256(item['text'].encode()).hexdigest() == item['sha256'], item['name']
print(evidence['code_commit'], evidence['results'])
PY
```

## 首批审查返修与验证

首轮 fallback 审查指出：有序映射仍隐式依赖规则名称决定优先级；递归排序数组的测试
会掩盖路径段和证据顺序变化。`5bba1f1` 将优先级改为显式数组，并加入重叠候选测试；
同时删除两处等价测试的全部数组规范化，直接比较 `serde_json::to_vec`。
`8addd0f` 清理随之失效的 import。

首批最终代码提交：`8addd0ff117eea57f33757b2b2a37a5b67e4f8b9`。
同一 CNB workspace 重跑首轮全部定向检查及 CLI snapshot：95项定向/单元测试、额外1项
快照通过，2项真实语料测试 ignored。增加的1项为重叠保底优先级测试；fmt、bench 编译和
WASM 检查均通过，原始数组顺序的字节比较也通过，未重置快照。

[返修完整日志与脚本](evidence/performance-contract-review-fixes-2026-09-08.json.gz)包含8份
检查日志和执行脚本，每份日志都有 COMMIT、COMMAND、EXIT_CODE 0。
压缩包 SHA-256：`ac9b4d9afb4cdcd9ff069623152f84b9b1f951a79f54418b245d8a929790126e`。
可复用上面的解包校验方式，替换压缩包路径与 SHA-256。
首轮证据包及四项回退探针仍保留原提交标识，不把它们改标成最终代码上的新运行。

## PR39 追加：前置条件预算与旁路读取错误

用户要求继续填充同一个 PR；本段从 `a0c055c` 后追加，不改写上面固定提交的证据。

- `d70fdbe`：compact warm cache 的 display/data/action prerequisites 各保留最多5条
  已排序 JSON，释放多余 Vec 容量；normal/full 保留全量。完整计数和 display/data 中模型
  首次出现的顺序单独保留，保证 summary、截断信封以及尾部条件引用的模型不因裁剪丢失。
  冷查询与 warm 共用模型合并逻辑，缓存投影不再为检查命中重复深拷贝 prerequisites。
- `1cdd034`：side-context 邻接读取失败经路径构建函数传播到冷查询与 warm builder，
  错误链包含页面、组件和原始原因；缺失邻接仍为空结果，正常跨页去重与同页排除保持原行为。
- `800766d`：修正冷查询故障注入点。首轮第5次读取仍在 prerequisites 阶段，虽然查询报错，
  side-context 上下文断言失败；实测第6次读取才进入目标阶段。保留失败日志，未删除上下文断言。

预算测试覆盖每组0、3、5、7条及三种 budget，精确断言缓存条目数、总数、剩余数、排序、
profile 计数和完整输出字节；模型只出现在尾部条件，另测 compact 缓存不能误用于 normal/full。
旁路测试覆盖正常去重、同页排除、缺失邻接，以及实际冷查询/warm builder 的读取失败传播。

临时回退探针：取消5条裁剪、把 summary 总数误用缓存长度、在裁剪后才收集模型引用，
分别触发预算测试失败；恢复 side-context 的 `.ok().flatten()` 后，冷查询与 warm 两项
错误传播测试同时失败。全部为编译成功后的测试失败；探针结束恢复源码并核对 git diff。

追加批代码提交：`800766d214ce23255ff14dc5ff5b43effe68ae9e`。
CNB workspace `cnb-ce2-1k1vtmjj3` 的最终统一检查：116项定向/单元测试、额外1项 CLI
快照通过；3项 ignored 分别为真实项目 runtime、手工 warm footprint 和交互 REPL 测试，
不计通过。fmt、bench 编译及 WASM 检查通过；完整命令见归档执行脚本。

[追加批完整日志](evidence/performance-contract-budget-errors-2026-09-08.json.gz)包含16份
日志及3份执行/回退脚本，包括错误注入阶段不匹配的原始失败。最终统一检查和四项回退
探针的日志均内嵌 COMMIT、COMMAND、EXIT_CODE；早期定向日志未记录的命令/退出码保留为 null。
压缩包 SHA-256：`e9cc6869e832ecf8fbae343a45c96414bbe3229aefac632365145b6ba9d2f721`。
解包核对方法同上，替换路径和 SHA-256。

## PR39 NPC 复审追加：顺序、实际构建与缓存元数据

本批从 `bfc462e` 继续，针对 NPC 指出的测试覆盖缺口补充：

- `9d72374` 删除 M51 profile、M52 profile、M53 dense path、M53 materialized
  availability 四个套件中的递归数组排序，六处比较直接使用完整序列化字节。
  dense/materialized 等价性覆盖 compact、normal、full，不重排路径段或证据。
- `7edf1da` 增加仅由 prerequisites 引入的模型，包装实际 GraphReadStore 读取，
  在读取时注入10毫秒延迟。cold 必须读到模型且 index build 计时包含延迟；warm
  必须零次模型读取，明确记录命中与零构建时间，并保持完整输出字节相同。
  另检查 build/projection 计时不超过所在阶段，context 与 index build 别名相等。
- `f5e7c22` 修正测试模型 ID：最初使用页面限定 ID，实际 resolver 读取全局存储 ID，
  cold 的“必须读到模型”断言失败。保留原始失败，改用真实存储身份，未放宽断言。
- `2a10f29` 增加7项真实 warm cache 比较测试：三类总数、同长度模型 ID 替换、
  模型顺序、Some/None prerequisites，以及完全相等的克隆。前五项保持 retained
  JSON 不变，双向要求精确返回 `prerequisites mismatch` 并拒绝等价。

三项临时回退探针均编译成功并触发目标断言：cold build 计时硬编码为0、warm 命中时
重复构建 availability、仅比较 prerequisites 的三组 JSON 而忽略总数和模型引用。
前两项分别使实际工作测试失败；最后一项使5项元数据差异测试失败，另2项仍通过。
探针在 finally 恢复源码，核对远端 git diff 为空后运行统一验证。

最终代码提交：`2a10f29675136d8e3210bb61f5165645d9305bde`。
CNB workspace `cnb-r0g-1k20ra3ov`：124项定向/单元测试及额外1项 CLI snapshot
通过，3项 ignored 保留原分类；fmt、bench 编译和 WASM 检查通过。
本地仅对[新口径采样说明](../milestones/performance/performance-baseline.md#采集新口径报告)
中的命令执行 `sh -n` 语法检查，没有运行真实语料 profile 或重测 Criterion/Bencher。

[本批完整日志与脚本](evidence/performance-contract-npc-tests-2026-09-08.json.gz)包含15份
日志及4份脚本，包括初次模型 ID 不匹配的失败。最终9项检查和3项回退探针均内嵌提交、
命令和退出码；早期日志缺失的字段保留为 null，提交来自会话执行记录并单独标注。
压缩包 SHA-256：`c09855b5a644157f45db3bd976e91ed1ee85145501c79959deedb53312346b6d`。
校验方式同前，替换文件名和 SHA-256；历史证据包保持原样。

## 影响与剩余边界

Baseline impact：**yes**。减少重复邻接读取，恢复真实毫秒计时，并隔离 unchanged profile
场景；新的耗时与旧的1毫秒下限/混入reload的profile样本不能直接横比。
这不是本轮真实项目的性能收益测量，不重置 Bencher 趋势、不声称真实语料基线已验收。

native fragment 跨 session 持久化仍延期；旧固定采样/忽略 mtime 的 reload 原型未恢复。
prerequisites 的 compact 常驻 JSON 已裁剪；构建仍先完整收集与排序，且完整模型引用列表
随唯一模型数量增长，不宣称峰值内存有界，也不把条目减少等同于真实项目耗时改善。
side-context 吞错已修复；PathFinder 和字段路径构建中的其他静默降级仍属独立问题，
本轮不能据此宣称全部 GraphReadStore 错误均会向外传播。
M54/M57 增量刷新测试仍递归排序数组；M54 的语义视图还移除了路径及派生 summary 字段。
这些比较不能证明完整输出顺序或路径等价，需另行核验，未纳入本批 M51–M53 的收紧范围。
受控延迟验证的是 cold index build 计时；projection 目前只检查字段存在、阶段上界，
没有独立的受控耗时探针，不把本批测试表述为所有计时字段均已验证准确性。
知识库回答质量、fixture 读取授权与 full-criterion 真实项目验收仍独立开放。
