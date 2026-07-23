# 产品路径 / Init / Warm 性能 Patch 设计

> 状态：**approved**（用户确认 2026-07-13：与 M54–M56 重叠项留差量刷新线；其余作独立 patch）  
> 范围：stdio LongLived 产品路径、`[profile.bench]`、Dense/Facts/Warm/页面元数据缓存、LongLived E2E bench 与内存观测。  
> **不含** META_FILES tick、多 dirty 批量删除、真增量 persist、selective rewarm 编排（见 [2026-07-12-diff-refresh-pipeline-design.md](2026-07-12-diff-refresh-pipeline-design.md)）。

## 背景

M53 已交付 LongLived 读模型与 warm cache，真实项目 warmed query ~81ms。当前堵点转向：

1. 产品路径未强制 LongLived（stdio 曾默认 OneShot）
2. Criterion 曾继承 `release`（`opt-level=z`）
3. LongLived 初始化（Dense ~3s / Facts ~1.8s / PageDep ~0.4s）
4. warm 物化与 `.spg` 重解析成本

## 目标

| 项 | 验收 |
|----|------|
| stdio | `RuntimeMode::LongLived`；`read_model_ready=true` |
| bench profile | `[profile.bench] inherits = "release-fast"` |
| Dense | 单次出边遍历；edge payload 一份；CSR 存 `edge_idx` |
| Facts | 物化索引存 typed `ConditionFact`，输出边界再 JSON |
| Warm path | compact/normal materialization 不 JSON 化无用路径桶 |
| 页面元数据 | 同进程按 path+mtime/size 复用；避免每次 warm 重解析 |
| Bench | `runtime_long_lived_startup` / `warm_pages_{1,10,50}` + RSS 采样钩子 |

## 非目标

- 继续压 warmed query <81ms
- 全量页面启动预热、并行 warm（无 profile）
- fragment 跨 session、换图库
- M54–M56 差量刷新实现

## 与 M54–M56 边界

| 本 patch | M54–M56 |
|----------|---------|
| stdio LongLived、bench profile、init/warm 构建 | 批量删除、增量 persist、META_FILES、selective rewarm、dirty 规模曲线 |
| LongLived startup/warm N 时间与 RSS | `runtime_selective_rewarm_pages_*` |

## 架构要点

```text
stdio run
  → GraphRuntime::load(..., LongLived)
  → RuntimeReadModel { dense, facts(typed), page_dep, warm cache }

warm(page, budget)
  → load_page_file_metadata (mtime cache hit?)
  → build_page_logic_paths(..., for_cache_materialization=true)
       PathMaterializeKeep(budget) 决定 JSON 桶
  → collect_page_prerequisites(..., budget) 按 budget 截断 cache 足迹
```

### DenseGraphSnapshot

- `from_graph`：按节点出边一次遍历；`edge_payloads.push` 一次；`outgoing`/`incoming` 共用 `edge_idx: u32`
- 保留 `DenseNeighborSlice` 零拷贝视图；path finder 仍可走 `DensePathTraversal`（clone adapter），P2 再改 dense-native

### ConditionFact（typed）

```rust
pub struct ConditionFact {
    pub condition_id: String,
    pub condition_type: String,
    pub effect_type: String,
    pub subject_type: String,
    pub owner_type: String,
    pub raw_expr: String,
    pub normalized_expr: String,
    pub json_path: String,
    pub source_file: String,
    pub referenced_symbols: Vec<String>,
}
```

- `MaterializedAvailabilityFactsIndex` 存 `HashMap<String, Vec<ConditionFact>>`
- `ConditionCollectorCache::from_prefilled` 读 typed；`collect_for_node` 返回仍为 JSON（兼容 explain/answer 边界）
- 公开查询 JSON 契约不变；保留 `tests/m53_materialized_availability_tests.rs` 等价性

### Warm path

- `PathMaterializeKeep`：compact/normal materialization 跳过无用路径桶的 JSON
- prerequisites：**不**在 cache 层截断——compact 输出依赖全量 `total_count` / `truncated_array(..., 5)`；截断会破坏字节等价
- 页面元数据：进程内 `path + mtime + size` 缓存（`metadata.rs`）
- **升级路径**（ponytail）：scanner/indexer 物化进 `RuntimeReadModel` / FileState；本轮不改 redb schema

### RSS

- `benches/common/rss.rs`：macOS/Linux 读当前进程 RSS（KB）
- 在 LongLived startup / warm N 场景前后采样，写入 criterion 自定义测量或 stderr 诊断；不设 CI fail 阈值

## 测试

- `tests/stdio_server_tests.rs`：status 含 `runtime_mode=LongLived`、`read_model_ready`
- 既有 `m53_*` dense / facts / warm 等价测
- compact/normal/full warm vs cold 输出等价（已有 T2 测则复用）

## 风险

| 风险 | 缓解 |
|------|------|
| typed fact 漏字段导致分类漂移 | 从 `build_cond_obj` 一一映射；跑 m53 availability 等价测 |
| mtime cache 跨进程失效 | 文档标明 process-local；reload 后新进程自然 miss |
| bench profile 变更影响历史数字 | baseline 文档记 Baseline impact |
