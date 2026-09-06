# M59 真实语料实测记录（2026-09-06）

> 性质：[Grafeo 后端迁移设计](../specs/2026-09-05-grafeo-backend-migration-design.md)
> §4 第 2 条阻塞项的实测。该条虽列在 D1，但**必须先于 A4**——A4 冻结 schema，
> 而内存读数直接决定「要不要瘦身节点 meta」。
> 前置：§4 第 1 条（语料在 dev workspace 不可达）已于 2026-09-06 解除。
> 这是**本项目第一次**在真实语料上测到这批数，此前的 8 GB 是线性外推。

## 0. 环境与输入

| 项 | 值 |
|---|---|
| workspace | `cnb-j1o-1k1qvdkp3`（CNB dev env） |
| 代码 | `codex/m58-slm-eval-foundation` @ `9200ec5` |
| 二进制 | `target/cnb/workspace/release/metadata-checker` 0.1.0（`--release`） |
| 语料 pin | `c3c0528fdd28e2600e0b0235040fb349b3c2d446` |
| 语料规模 | `.spg` 501 / `.tbl` 828，源目录 158M |

语料由 `.cnb.yml` 的 `fetch real project corpus` stage 拉取，仓库/pin/路径均来自
`metadata-checker-keys/real-fixture.yml`，本文不复制其中任何取值。

## 1. 峰值 RSS 怎么测的（无 GNU time 时的替代）

镜像里没有 `/usr/bin/time`（`browser-wasm-ci`），第一版测量脚本因此在第 26 行
直接死掉，而日志停在 stage 标题上，**看起来像是在跑长任务**——白等了 6 分钟。
教训：`set -euo pipefail` 下的早退和「正在跑」在日志上无法区分，要看落盘产物。

替代方案 `tools/peak-rss-run.py`：跑一条命令，取
`resource.getrusage(RUSAGE_CHILDREN).ru_maxrss`。

**污染前提**：`ru_maxrss` 是**已回收的全部子进程的最大值**，不是最后一个子进程的
值。所以每条被测命令必须**各自一次** `peak-rss-run.py` 调用，不能在同一个进程里
连测两条再读一次。

## 2. 建图路径（写入侧）

```
python3 tools/peak-rss-run.py --label build-graph --json out.json -- \
  "$BIN" --project-dir "$CORPUS" --build-graph --graph-db-path /tmp/xsy-m59.graphdb
```

| 指标 | 读数 |
|---|---|
| wall | **64.67 s** |
| 峰值 RSS | **1,733,648 KiB = 1,693.0 MiB** |
| exit | 0 |
| indexed / unchanged / dirty / deleted | 1329 / 0 / 1329 / 0 |
| node_count | **89,178** |
| edge_count | **200,028** |
| `SCANNER_UNRECOGNIZED_CONTAINER_KEY` | **312** |
| `SCANNER_DUPLICATE_COMPONENT_ID` | **1,105** |
| graphdb `du -sb` | **538,972,160 B（514 MiB 整）** |

### 2.1 与 PR2 基线的差分

对照 `docs/milestones/performance/performance-baseline.md:775-790`：

| | PR2 基线 | 本次 | 差 |
|---|---|---|---|
| 节点 | 89,094 | 89,178 | +84 |
| 边 | 199,575 | 200,028 | +453 |
| graphdb 字节 | 538,972,160 | 538,972,160 | **0** |
| 建图 wall | 45.9 s | 64.67 s | +18.8 s |

**两条陈旧诊断基线经此确认仍然准确**：312 与 1,105 一字未变，这个待办可以关掉。

节点/边的小幅增长来自基线之后落地的扫描改动；字节数分毫不差是 redb 页分配粒度
的产物，不构成「图没变」的证据——**不要**拿它当回归判据。建图 wall 的 +41%
未单独归因，本次不追（迁移后 redb 写入路径整体退役）。

## 3. `--check-graph` 不是加载路径（一条重要的否定结论）

```
check-graph : wall_s 0.005, peak_rss 6.0 MiB
```

5 毫秒、6 MiB。**它只读文件头，根本没有把图读进来。** 任何拿
`--check-graph` 的读数去回答「加载要多少内存」的做法都是错的——这正是本次差点
掉进去的坑。§4 的驻留必须走 stdio 加载路径。

同理，§2 的 1,693 MiB 是**写入侧**峰值，与加载侧驻留是两个数，不可互相替代。

## 4. 加载后驻留（stdio 路径）

> 状态：**两轮读数均已到手**（2026-09-06）。结论见 §4.2，复现性见 §4.3。

方法：

```
python3 tools/peak-rss-run.py --label graph-load --json rss.json -- \
  python3 tools/graph-load-measure.py --bin "$BIN" --graph-db /tmp/xsy-m59.graphdb \
    --project-dir "$CORPUS" --runs 2 --timeout-secs 2400 --rss-sample
```

`--rss-sample`（本次新增，`90d9a2f`）在 `[stdio-server] Graph loaded` 标记出现、
`status` 返回之后、kill 之前读 `/proc/<pid>/status`，同时取：

- `VmRSS` —— 此刻的**驻留**，即 §4.2 要的那个数；
- `VmHWM` —— 进程生命周期峰值，加载期临时缓冲会把它抬到驻留之上。

两者分开记；混用会高估常驻成本。

### 4.1 第 1 轮读数

`m59-measure/graph-load.json`（run 1）：

| 项 | 原始值 | 换算 |
|---|---|---|
| `resident_rss_kib_after_load`（`VmRSS`） | 3,897,084 KiB | **3,805.7 MiB = 3.72 GiB** |
| `peak_rss_kib_vmhwm`（`VmHWM`） | 3,959,288 KiB | 3,866.5 MiB = 3.78 GiB |
| `spawn_to_graph_loaded_ms` | 1,148,706 ms | **1,148.7 s ≈ 19 min 9 s** |
| `total_wall_ms_incl_query` | 1,148,929 ms | 加载后首条查询只占 223 ms |
| `node_count` / `edge_count` | 89,178 / 200,028 | 与建图侧一致 |

stderr 确认加载确实完成（不是超时被 kill）：
`[stdio-server] Graph loaded (LongLived), 89178 nodes, read_model=true, ready`。

### 4.2 三条结论

**一｜8 GB 外推偏高约 2 倍，但量级方向没错。**
`.graphdb` 落盘 538,972,160 B = 514.0 MiB，驻留 3,805.7 MiB，膨胀 **7.4×**。
合成图那次是 16×（16.5 MB → 262 MB），据此外推的 8 GB 因而高了约 2.1 倍。
真实读数 3.72 GiB 仍然是**单进程常驻近 4 GB**，不是可以忽略的量。

**二｜这 3.7 GiB 是结构本身，不是加载期的临时缓冲。**
`VmHWM − VmRSS` 只有 62,204 KiB ≈ 60.7 MiB。也就是说峰值几乎全部留了下来，
加载路径没有大块临时分配。**对 A4 的直接含义**：瘦身节点 meta 的收益会
一比一体现在常驻内存上，不会被「反正峰值也就那样」抵消——这是把「要不要瘦身」
从猜测变成可算账的那个事实。

摊到图元素上：3,897,084 KiB ÷ (89,178 + 200,028) = **约 13.5 KiB / 每节点或边**。
一个组件节点的真实语义内容远不到这个数，差额在 meta 与索引结构里。

**三｜加载速率 0.447 MiB/s，比 §0 记的病理还慢。**
514.0 MiB ÷ 1,148.7 s。设计文档 §0 记的「我们要 1,008 s、Grafeo 0.87 s」在真实
语料上复现且更差（1,148.7 s）。这条**不阻塞迁移**——迁移完成后这段代码退役，
差值自动消失——但它与 §2 建图侧只要 64.67 s 的对比值得记一笔：**写入侧不慢，
慢的是读回来**，说明病理在反序列化/图重建侧，不在扫描侧。

### 4.3 第 2 轮：复现性

第 2 轮与 B3 的 `cargo test` 在同一 workspace 上并发，因此**它的 wall 时间不可与
第 1 轮直接比较**；RSS 不受 CPU 争用影响，仍然可比。

| 指标 | 第 1 轮 | 第 2 轮 | 差值 |
|---|---|---|---|
| `resident_rss_kib_after_load`（`VmRSS`） | 3,897,084 KiB | 3,897,108 KiB | **+24 KiB（+0.0006%）** |
| `peak_rss_kib_vmhwm`（`VmHWM`） | 3,959,288 KiB | 3,958,988 KiB | −300 KiB（−0.008%） |
| `VmHWM − VmRSS` | 62,204 KiB = 60.7 MiB | 61,880 KiB = 60.4 MiB | −324 KiB |
| `spawn_to_graph_loaded_ms` | 1,148,706 ms | 1,199,006 ms | +50,300 ms（+4.4%，见下） |
| `node_count` / `edge_count` | 89,178 / 200,028 | 89,178 / 200,028 | 一致 |

两轮 harness 合计 `wall_s` 2,348.2 s，`peak_rss_kib` 3,959,288（即第 1 轮的 `VmHWM`）。

**结论：内存驻留基本是确定量。** 两轮 `VmRSS` 相差 24 KiB——六个数量级里的一位，
远小于任何有决策意义的幅度。§4.2 对 A4 的那条输入（`VmHWM − VmRSS` ≈ 60 MiB，
峰值几乎全是常驻结构，瘦身 meta 一比一回收常驻内存）在两轮上独立成立，
不是单次采样的偶然。

**+4.4% 的加载耗时是 CPU 争用，不是方差。** 第 2 轮全程与 B3 的
`cargo test`（8 核 workspace）抢核。这个数**不可用作**加载性能的第二个样本；
§4.2 结论（三）关于「慢在读回侧」的论证只依赖第 1 轮的独占读数与建图侧的
64.67 s 对比，不受影响。真要给加载耗时做复现性验证，需要一次独占跑——
但该路径随 redb 一起退役，不值得为它再占一次核时。
