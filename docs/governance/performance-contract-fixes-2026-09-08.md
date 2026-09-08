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
  验证 full/normal 保留、compact 删除，并对比有/无 warm 缓存的完整 canonical JSON。
- **确定性**：保底规则原用 HashMap，遍历顺序会改变路径提升说明。
  改为有序映射后，同图 warm/query 不再因规则随机顺序出现内容漂移。
- **runtime 降级**：load 复用独立的读模型构建函数。故障 store 在 dense 构建完成后
  拒绝读取，验证真实 PageDependencyIndex 构建失败时 dense 仍可用、索引为空且状态为
  Partial，并精确检查唯一诊断的 code/severity/count/answer_impact/阶段/位置及计时总和。
  正常分支验证 Full 状态与实际页面映射；注入点在生产构建函数，不是公开 load 的磁盘故障。

完整 JSON 比较中的 canonicalization 会重排数组，不能表述为原始字节或顺序相等。
新 supporting 测试仍保留分类说明字段，未通过删除差异字段绕过不稳定问题。

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

## 最终验证与证据

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

## 影响与剩余边界

Baseline impact：**yes**。减少重复邻接读取，恢复真实毫秒计时，并隔离 unchanged profile
场景；新的耗时与旧的1毫秒下限/混入reload的profile样本不能直接横比。
这不是本轮真实项目的性能收益测量，不重置 Bencher 趋势、不声称真实语料基线已验收。

native fragment 跨 session 持久化仍延期；旧固定采样/忽略 mtime 的 reload 原型未恢复。
prerequisites 的 budget 参数尚未参与裁剪；本轮只保证现有 path 桶契约。
path side-context 仍通过 `.ok().flatten()` 静默降级部分读取错误；本次 edge bundle
错误传播测试不代表全部 GraphReadStore 读取错误均会向外传播。
知识库回答质量、fixture 读取授权与 full-criterion 真实项目验收仍独立开放。
