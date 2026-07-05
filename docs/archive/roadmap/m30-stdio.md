# M30：Stdio 性能与容量治理

| milestone | M30 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M30：Stdio 性能与容量治理

### 目标

在 stdio command 扩展后，防止大查询把性能瓶颈从 GraphDB 冷加载转移到 pathfinder、evidence 组装或 JSON 序列化上。

### 非目标

- 不优化 GraphDB 构建速度。
- 不做 Lazy GraphDB。
- 不牺牲主链路正确性换取性能。

### 工作清单

- [x] 所有 stdio 查询记录统一 timing：
  - `graph_load_ms`
  - `query_compute_ms`
  - `serialize_ms`
  - `total_ms`
  - `output_size_bytes`
- [x] 输出预算治理：
  - 默认 compact 或 normal，禁止默认 full。
  - full 必须显式指定。
  - 超预算返回 `OUTPUT_TRUNCATED` diagnostics。
  - 截断不能删除 primary path。
- [x] 建立真实项目性能基线：
  - `explain_condition input3`
  - `context input3 depth=2`
  - `query_model fact_qwSidebar`
  - `query_page_logic 合同协议.spg`
  - 对比 CLI 冷启动与 stdio 第二次请求。
- [x] 热点分析：
  - stdio 第二次查询不得进入 `GraphDB::load_from_db`。
  - 若慢，优先检查 pathfinder、evidence 组装、JSON serialize、大数组排序和 clone。
  - 可选使用 `samply` 对真实项目第二次查询采样。
- [x] 回归门槛：
  - 第二次 stdio 查询 `graph_load_ms == 0`。
  - 小查询 `total_ms < 50ms`。
  - 中等查询 `total_ms < 300ms`。
  - 大查询必须可被 budget 截断，且输出不发生注意力漂移。

### 验收目标

- stdio 扩展后，第二次查询不再冷启动。
- 大输出有预算和截断机制，不依赖 AI 自行忽略噪声。
- 性能基线文档记录可复现命令、commit、graphdb 路径、输出大小和 timing。
- 真实项目主链路验收不因性能截断回退。
