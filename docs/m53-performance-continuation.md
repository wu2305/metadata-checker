# M53 性能优化落地记录

> 状态：M53 起始基线。M52 已完成 redb v2 graph layout 设计和 shadow build 验证；M53 负责把验证过的 v2 存储体切入实际运行路径。

## 从 M52 带入的已验证优化

### 1. Cost model report

已把 page logic profile 从单纯耗时列表扩展为可解释成本模型：

- `purpose`：热点服务的产品语义。
- `why_expensive`：复杂项目中成本被放大的原因。
- `cost_drivers`：stage / counter 量化来源。
- `space_time_candidates`：空间换时间候选。
- `next_action`：下一步动作。

M53 继续复用这套报告，不另建 profiling 类型。

### 2. Page-local availability batch

`query_page_logic.key_model_availability` 已从逐个 model 重复构造，推进到单次 page logic 内共享 graph read cache / condition collector cache。

真实项目 `sample-count=1` 已验证：

| 场景 | 优化前 | 优化后 | 变化 |
|---|---:|---:|---:|
| `full.key_model_availability` | 5227ms | 3819ms | -26.9% |
| `normal.key_model_availability` | 5040ms | 3508ms | -30.4% |
| `compact.key_model_availability` | 164ms | 124ms | -24.4% |

M53 不再重复做 query-time 临时 fact index。M52 已证明它会变慢。

### 3. Dense snapshot / warm read-model

已验证把 availability / prerequisites / path summary 前移到 long-lived runtime warm 阶段，可以显著降低 query-time。

真实项目 `合同协议.spg` compact 链路：

| 指标 | one-shot | long-lived | long-lived + warm |
|---|---:|---:|---:|
| query dispatch | 916ms | 898ms | 105ms |
| total N=10 | 9758ms | 12292ms | 5195ms |
| total N=50 | 46398ms | 48212ms | 9395ms |

M53 默认接受 init/load 变重，只要 query-time 和多次查询总成本下降。

### 4. Page logic fragment contract

已实现 `cli-local` native 文件后端 spike：

- 持久化 `availability` / `prerequisites` / `paths` / `warm_stages` / `meta`。
- fragment hit 后恢复为现有 `PageLogicAvailabilityCache`。
- miss / schema mismatch / fingerprint mismatch fallback 当前 Rust 路径。

M53 的稳定边界不是文件目录格式，而是 page-logic fragment bundle。native 文件、browser IndexedDB、memory 都只是后端。

### 5. Reload unchanged 快路径

已修复 redb 打开扰动 graphdb `mtime` 导致的无意义 reload。

真实项目结果：

| 指标 | 优化前 | 优化后 |
|---|---:|---:|
| `runtime_check_reload_unchanged` | 676ms | 0ms |
| `runtime_check_reload_reloaded` | 误触发 | 0 |

M53 继续保留稳定内容采样 hash：`mtime_changed` 只做诊断，不触发 reload。

## M53 继续推进的性能问题

1. redb v2 hydrate。
   基于 M52 已落地的 shadow layout（`graph_redb_v2` + `tests/m52_redb_v2_shadow_tests.rs`），让 `GraphDB::open_inner` 优先从 v2 node meta + out/in adjacency 恢复内存图；v2 缺失或校验失败 fallback v1。

2. redb v2 incremental persist。
   用 dirty node / deleted node 更新对应 adjacency 和 file state，不再全量清空并重写 edges / file_states。

3. `key_model_availability` 仍是 full / normal 主导成本。
   下一步应做 build/load-time materialized facts，而不是继续压 JSON shaping。

4. `path_summary` 仍受 candidate search 和 anchor extract 影响。
   下一步优先让 path finder 直接消费 dense/CSR 数据，避免 owned clone 适配器抵消收益。

5. fragment 需要跨平台落地。
   native 文件后端已经证明可行；browser 侧应复用已有 IndexedDB cache/provider，不能假设文件系统。

6. long-lived runtime 的 init/load 成本需要分层观察。
   允许变重，但必须区分 graph load、dense snapshot、read model build、page warm、fragment read。

## M53 不重复做

- 不重跑 per-query fact index 方向。
- 不把 DuckDB / Parquet 当作 M53 前置条件。
- 不设置性能 fail threshold。
- 不做 browser real perf。
- 不改公开 JSON 语义。
