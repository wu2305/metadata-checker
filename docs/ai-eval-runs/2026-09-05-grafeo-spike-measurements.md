# Grafeo 落地实测记录（2026-09-05）

> 性质：`docs/plans/2026-09-05-graph-backend-migration-plan-review.md` §4 第 0 步
> 「一次性判决实验」的实测数据。主体已完成，结论见 §0。
> 环境：CNB workspace `cnb-g1g-1k1np634j`，8 core / 16 GB，rust 1.95.0，
> grafeo 0.5.42（crates.io，2026-05-04 发布）。
> 语料：**合成图**（见 §3）。真实 xiaoshouyi 语料 workspace 拉不到——workspace 的
> `CNB_TOKEN` 是仓库级的，`git ls-remote succbi_project_container` 返回
> Repository Not Found，而同凭据访问 `origin` 正常。语料只有流水线能拿
> （`.cnb.yml:707` 的 `fetch real project corpus`，跨仓 token 来自 `imports` 的
> `metadata-checker-keys/real-fixture.yml`）。见 §5。

## 0. 结论

三个问题都有答案，而且都跟事前假设不同：

1. **Grafeo 不能免掉全量进内存**（§3.2）——复核 §2.5 的怀疑成立。
2. **但这不重要，因为它 0.87 秒就做完了，而我们要 1,008 秒**（§3.2）。
   → **真正的缺陷不在「要不要进内存」，在我们自己的加载路径。**
   514 MiB / 1,008 s ≈ **0.5 MB/s**，这个速率对反序列化而言不合理，
   指向本项目 load path 里有病理问题（疑似 O(n²) 或逐节点事务），
   **换不换后端都值得单独查**。
3. **真正的拦路虎变成了二进制体积**（§1）：最小特性集下 grafeo 一个空壳 main 就
   8.86 MiB，项目上限 10 MB。这是目前最硬的阻塞项。

## 1. 编译与体积（已测定，硬结论）

最小可用特性集：

```toml
grafeo = { version = "0.5", default-features = false,
           features = ["cypher", "storage", "lpg"] }
```

- `lpg` **不可省**：只开 `cypher + storage` 时运行期 panic——
  `no graph store available: enable the 'lpg' feature or use with_store()`。
- 默认特性有 12 个（`embedded` `ai` `algos` `arrow-export` `gql` `grafeo-file`
  `parallel` `regex` `cdc` `hybrid-search` `text-index` `vector-index`），
  向量/文本索引与 CDC 全在默认里，**必须显式关掉**。
- 干净编译 **31.9 s**（8 核），依赖树 `grafeo-common` / `grafeo-core` /
  `grafeo-storage` / `grafeo-adapters` / `grafeo-engine`。

| 测项 | 值 |
|---|---|
| 空壳（不调用 API，链接器可全量 dead-strip） | 445,920 B（strip 后 343,400 B） |
| **实际调用 API 后的 release binary** | **9,286,744 B ≈ 8.86 MiB** |

**⚠️ 这条直接撞项目的 10 MB release 上限。** 单是 grafeo 一个空壳 main 就吃掉
8.86 MiB，而 `metadata-checker` 自身还要占位。复核 §2.3 提出的「体积应当是淘汰规则
而非打分项」在这里从推测变成实测：**按当前特性组合，Grafeo 静态链接进主二进制这条路
是走不通的**，除非

- 找到更小的特性子集（`storage` 拆成 `mmap`/`spill`/`wal` 单选？），或
- 换 opt-level=z / LTO / panic=abort 等体积优化后重测，或
- 接受把图查询拆成独立二进制（那要重新设计 CLI 契约）。

**未做**：上述三条都还没测。这是下一步。

## 2. 功能连通性（已验证）

持久化往返可用：

```rust
let db = grafeo::GrafeoDB::with_config(grafeo::Config::persistent(path))?;
db.session().execute("INSERT (:Page {id:'p1'})")?;   // 写
drop(db);
let db2 = grafeo::GrafeoDB::open_read_only(path)?;    // 重开
db2.session().execute("MATCH (a:Page)-[:OpensPage]->(b:Page) RETURN a.id, b.id")?;
```

小库（2 节点 1 边）重开 + 查询往返 **2.15 ms**。Cypher 语法、`.grafeo` 单文件格式、
只读并发锁均按文档工作。

## 3. 规模测试（合成图）

合成规模对齐 xiaoshouyi 基线（89,094 节点 / 199,575 边）：
1,400 Page + 77,000 Comp + 6,000 Model + 6,000 Field = **90,400 节点**；
边 84,400（Comp-Reads->Model、Model-Contains->Field、Page-OpensPage->Page）。

### 3.1 写入吞吐：Cypher 语句 vs 直写 API，差 60 倍

| 路径 | 规模 | 耗时 |
|---|---|---|
| 逐条 Cypher `INSERT` 语句 | 90,400 节点 | **287.75 s**（≈314 节点/秒） |
| 逐条 Cypher `MATCH…INSERT` 建边 | 84,400 边 | **未能完成**（见下） |
| **直写 API**（`create_node_with_props` / `create_edge`） | 90,400 节点 + 161,400 边 | **4.71 s** |

三条必须写进任何迁移实现的注意事项：

1. **多语句拼一次 `execute` 静默失效。** 首轮把 500 条 `INSERT ...;\n` 拼成一次调用，
   「90,400 节点 94 ms 完成」、落盘 40 KB——只执行了第一条，**不报错**。
2. **`MATCH` 按属性匹配没有索引就是全标签扫描。** 用 Cypher 建边
   （`MATCH (a:Comp {id:'compN'}), (b:Model {id:'modelM'}) INSERT (a)-[:Reads]->(b)`）
   在 77,000 个 Comp 上跑 84,400 次，实测跑满 10 分钟无进展，只能 SIGTERM。
   `db.create_property_index("id")` 是独立 API，不建就没有。
3. **迁移必须走直写 API**（`grafeo-engine` 的 `Session::create_node_with_props` /
   `create_edge` / `create_edge_with_props`，顶层 re-export `NodeId`/`EdgeId`/`Value`），
   自己持有 `NodeId` 建边，不要回头 `MATCH`。这条路 4.7 秒建完整张图。

### 3.2 冷启动 + 多跳查询（决定性测量）

全新进程，`open_read_only` → 一条 4 跳 Cypher
（`(:Page)-[:Contains]->(:Comp)-[:Reads]->(:Model)-[:Contains]->(:Field)`）：

| 运行 | OPEN | open 后 RSS | 4 跳查询 | **启动→首条结果** | peak RSS |
|---|---:|---:|---:|---:|---:|
| 无内存上限 | 865.8 ms | 262 MB | 50.5 ms | **920.7 ms** | 307 MB |
| 复跑确认 | 874.9 ms | 262 MB | 49.5 ms | **929.2 ms** | 307 MB |
| `memory_limit = 128 MB` | 947.5 ms | **262 MB** | 45.3 ms | 996.8 ms | 307 MB |
| `memory_limit = 32 MB` | 894.0 ms | **262 MB** | 51.5 ms | 950.1 ms | 307 MB |
| 对照：2 节点小库 | 41.6 ms | **5 MB** | 0.66 ms | 42.3 ms | 6 MB |

三条读数：

1. **Grafeo 确实是 in-memory 优先。** 2 节点库 RSS 5 MB、90k 节点库 RSS 262 MB——
   262 MB 不是 arena 预分配，是真数据。且 **`with_memory_limit` 完全不约束打开路径的
   驻留**：上限设 128 MB 和 32 MB，RSS 一样是 262 MB，既不报错也不 spill。
   复核 §2.5 的怀疑 1 **成立**：换 Grafeo **不会**免掉「整张图进内存」。
2. **但它 0.87 秒就做完了这件事。** 对照本项目基线的 **1,008 秒**。
3. **内存膨胀约 16 倍**：16.5 MB 的 `.grafeo` 文件 → 262 MB RSS。

### 3.3 合成语料与真实语料的差距（读数时必须带上）

| | 合成 | 真实 xiaoshouyi |
|---|---|---|
| 节点 | 90,400 | 89,094 |
| 边 | 161,400 | 199,575 |
| 落盘体积 | **16.5 MB** | **514 MiB** |

节点/边数量对齐，**但落盘体积差 31 倍**——真实图的每个节点/边都带 JSON `meta`，
合成图的属性只有 `id`/`page`。所以：

- 0.92 s 不能直接搬到真实语料上。线性外推约 **27 s**，仍比 1,008 s 好 **37 倍**；
- 262 MB 的 16 倍内存膨胀若也线性，真实语料约 **8 GB** 驻留——16 GB 机器勉强，
  笔记本上是风险。**这一条必须在真实语料上实测**，不能外推了事。

## 4. API 侧发现：`with_read_store` 改变了选项空间

`GrafeoDB` 的构造方法里有：

```rust
GrafeoDB::with_read_store(store: Arc<dyn GraphStoreSearch>, config: Config) -> Result<GrafeoDB>
```

即 **可以用自己的只读存储实现给 Grafeo 的 Cypher 引擎供数**。这意味着存在第三条路：
**redb 留着不动，只借 Grafeo 的查询引擎**——零数据迁移、零 schema 变更，
拿到 openCypher。计划第 4 节写的「Grafeo 原生 Cypher / `with_read_store`」原来指的
就是这个，复核前两版没意识到它是一条独立路线。

代价与未知：

- 仍要付 8.86 MiB 体积（§1）；
- `GraphStoreSearch` 的方法集尚未查到（docs.rs 索引里只见 `GraphStore` /
  `GraphStoreMut`）。**关键问题**：引擎是按需回调这个 trait（懒加载，正合
  复核 §0.3.1 的下推诉求），还是先把它整体物化？没验证之前不能算数。

`Config` 侧确实有 out-of-core 的旋钮，与官网文案给的印象相反：

- `memory_limit: Option<usize>`（`with_memory_limit` / `with_memory_fraction`）
- `spill_path: Option<PathBuf>` —— "directory for out-of-core processing under memory pressure"
- `section_configs` —— per-section 内存预算与 **RAM vs. disk 分层 pinning**

另：`open_read_only` 的文档写「loads the last checkpoint snapshot but does **not**
replay the WAL」——"loads" 这个词指向快照整体读入。仍需实测区分。

## 5. 语料可达性（阻塞项，需决策）

真实 xiaoshouyi 语料在 `cnb.cool/wu2305/succbi_project_container` 的
`xiaoshouyi-corpus` 分支（sparse `projects/xiaoshouyi`，约 156 MB，1,387 文件）。

- **流水线可达**：`.cnb.yml:707` 的 `fetch real project corpus`，凭据来自
  `imports` 的 `metadata-checker-keys/real-fixture.yml`，走 `http.extraHeader`。
- **dev workspace 不可达**：workspace 的 `CNB_TOKEN` 是仓库级的。实测
  `git ls-remote https://cnb.cool/wu2305/succbi_project_container.git` 返回
  `Repository Not Found`，同一凭据 `git ls-remote origin` 正常。

因此真实语料上的判决实验只能作为 **`api_trigger_*` 流水线事件**跑，需要新增一个
event（照 `.kimi_harness_smoke_ci` 的 imports + corpus fetch 段），由人触发。
合成图能回答「Grafeo 是不是 in-memory 优先」这个架构属性，但回答不了真实语料的
体积/分布特性。
