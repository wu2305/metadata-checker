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

> 状态：测量进行中（`tools/graph-load-measure.py --rss-sample`，2 轮，
> 每轮按 PR2 基线约 1,008 s）。读数落地后补写本节。

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

中途观察（加载未完成时）：被测进程 RSS 已达 **3,896,956 KiB ≈ 3.72 GiB**，
且仍在上升。合成图外推的 8 GB 量级方向上没有被证伪。
