# M50 Browser Offscreen WASM Bench 落地方案

> 状态：方案已确认，先固化 bench contract；CNB.cool 流水线与具体实现分阶段落地。

## 目标

这批 benchmark 只覆盖浏览器插件的 Offscreen local graph 链路：

```text
selection
-> offscreen runtime host
-> fake fetch 返回本地 .spg raw text
-> ensureRuntime / wasm init
-> initRuntime
-> loadSuperpageDocument
-> buildOrUpdateSuperpageGraph
-> analyzeLocalGraph
-> artifact/timing 输出
```

约束：

- 不拉起浏览器。
- 不集成 Controller。
- 不连接真实远端 API。
- 必须运行真实 `browser-wasm` 编译产物。
- Node replay runner 只负责执行插件链路和统计，不承载解析、图查询或业务推理能力。

## 样本与 Fixture

样本通过一次性 Rust parser 扫描真实项目生成。扫描只做一次，之后 benchmark 只消费已入仓 fixture 和 manifest，不再依赖真实项目目录。

建议目录：

```text
tests/fixtures/browser-offscreen-real-project/
  manifest.json
  project/
    <保留真实相对路径>/PageA.spg
    <保留真实相对路径>/PageB.spg
```

第一批固定 5 类页面：

- `typical_p75_page`
- `large_raw_page`
- `high_component_page`
- `high_reference_page`
- `worst_combined_page`

每个页面保存 3 类组件候选：

- `high_reference_component`
- `container_component`
- `leaf_component`

manifest 保存相对路径、组件 ID、raw bytes、component count、reference count、候选组件统计。不保存 raw metadata 到 manifest。`.spg` 快照以裁剪项目目录形式入仓，第一版允许保留真实业务元数据，后续再做脱敏。

必须保留真实项目相对路径，因为元数据内部存在相对路径引用。fixture 文件应按真实相对路径落盘，避免 bench 运行时做复杂路径映射。

## WASM 构建

benchmark 必须自动生成真实 WASM，不提交生成产物。

本地策略：

- 从 `Cargo.lock` 读取 `wasm-bindgen` crate 版本。
- 检测本机 `wasm-bindgen` CLI 版本。
- 版本不匹配时 fail，并输出明确安装命令。
- 不在本地自动安装 CLI。

构建步骤：

```bash
cargo build --release --no-default-features --features browser-wasm --target wasm32-unknown-unknown
wasm-bindgen --target nodejs --out-dir target/browser-offscreen-bench-wasm <wasm>
```

CNB.cool CI 中通过固定 Node.js Docker image 和流水线步骤安装匹配版本 `wasm-bindgen-cli`。

## Replay Runner

建议新增：

```text
browser/bench/
  offscreen-local-graph-replay.mjs
  summarize-offscreen-bench.mjs
  validate-offscreen-bench-output.mjs
```

场景：

- `cold`：新 runtime、新 WASM 初始化、新 metadata cache。
- `warm_same_document_new_component`：同一 document 已 fetch/load，切换组件。
- `repeat_same_component`：同一 document 同一组件重复运行。

默认参数：

- `--iterations 10`
- `--warmup 2`
- cold 每次独立 runtime 环境。
- warm/repeat 在同一 runtime 环境内连续运行。
- 参数允许覆盖。

## 指标与报告

第一版只报告，不配置阈值，不做 ratio，不做稳定性建议。

JSONL 每次 iteration 一行：

```json
{
  "capability": "browser_analyze_selection",
  "mode": "browser_offscreen_replay_wasm",
  "scenario": "cold_large_raw_page_high_reference_component",
  "sample": "large_raw_page",
  "component_kind": "high_reference_component",
  "duration_ms": 1280,
  "timing": {
    "ensure_runtime_ms": 310,
    "fetch_metadata_ms": 2,
    "init_runtime_ms": 4,
    "load_document_ms": 410,
    "build_graph_ms": 520,
    "analyze_local_graph_ms": 31,
    "total_ms": 1280
  }
}
```

输出产物：

```text
browser-offscreen-bench.jsonl
browser-offscreen-summary.json
browser-offscreen-summary.md
browser-offscreen-env.json
browser-offscreen-samples.json
browser-offscreen-bencher-bmf.json
```

Markdown 报告包含：

- scenario
- sample
- component kind
- iteration count
- `total_ms` 的 p50 / p95 / max
- 各阶段 p50 / p95 / max
- `dominant_stage`

CI 第一阶段只因可信产出失败而失败：

- WASM 构建失败。
- runner 崩溃。
- manifest/fixture 解析失败。
- 任一 scenario `ok=false` 或 `artifact_ready=false`。
- timing 字段缺失或非数字。

性能数值变慢第一阶段不 fail。阈值策略需等 CNB.cool 环境运行一段时间后再确定。

Bencher.dev 上报：

- `browser-offscreen-summary.json` 通过 `browser/bench/offscreen-summary-to-bencher-bmf.mjs` 转换为 Bencher Metric Format JSON。
- 转换结果按 `browser_offscreen/<scenario>/<sample>/<component_kind>/<metric>/<stat>` 命名 benchmark。
- `metric` 包含 `total_ms` 与各阶段 timing key；转换脚本支持 `p50`、`p95`、`max`。
- 当前 `perf-browser-offscreen-ci` 是 smoke 模式，Bencher 上报只使用 `p50`，避免单次迭代下重复上传等价的 `p95` / `max`。
- Bencher `latency` 以 ns 记录，因此转换脚本会把 ms 乘以 `1_000_000`。
- CI 通过 CNB 密钥仓库文件 `wu2305/metadata-checker-keys/bencher.yml` 注入 Bencher 环境变量；只在 `BENCHER_API_KEY` 与 `BENCHER_PROJECT` 已配置时上报；第一阶段不配置 threshold。

## Makefile 入口

建议新增：

```text
make perf-browser-offscreen-samples
make perf-browser-offscreen
make perf-browser-offscreen-bencher-bmf
make perf-browser-offscreen-ci
```

职责：

- `perf-browser-offscreen-samples`：本地一次性生成 fixture/manifest。
- `perf-browser-offscreen`：本地完整 bench。
- `perf-browser-offscreen-bencher-bmf`：把已有 summary 转为 Bencher BMF JSON。
- `perf-browser-offscreen-ci`：CI 完整 bench，消费已入仓 fixture/manifest。

样本扫描 helper 可以是一次性工具，不要求长期维护。长期维护目标是 replay runner、输出 contract 和 CI 入口。

## CI 分层

CNB 流水线按分支与事件分层，避免所有 push 都跑重 benchmark。

| 触发 | 流水线 | 用途 |
|---|---|---|
| `main` push | `rust-only-ci`、`browser-wasm-env-probe`、`browser-offscreen-bench-ci`、`fixture-bench-ci`、`full-criterion-bench-ci` | merge 后权威性能记录 |
| 非 `main` push | `rust-only-ci-branch-push`（stage `if` 跳过 main） | 分支轻量验证 |
| 任意 PR | `rust-only-ci`、`browser-wasm-env-probe`、`browser-offscreen-bench-ci` | PR 快速验证 WASM/offscreen 链路 |

约束：

- 不在 `pull_request.merged` 重复跑 benchmark；merge 后由 `main` push 承担权威记录。
- 第一版不设性能阈值，bench 失败仅因构建崩溃、fixture/manifest 无效或 timing 字段缺失。
- `browser-offscreen-bench-ci` 的 `endStages` 输出 `browser-offscreen-ci-perf-index.json` 机器可读索引。
- `full-criterion-bench-ci` 仅在 `main.push` 运行，复用 `METADATA_CHECKER_REAL_PROJECT_DIR` 或 clone `REAL_PROJECT_FIXTURE_REPO_*` 指定的 fixture 仓库，并把完整 Criterion bench 上报到 Bencher.dev。

### 后续：手动 / 定时 full benchmark

真实项目 full bench（`make perf-real`、`make perf-real-mutation` 等）不在普通 CI 中运行，后续可通过：

- CNB `web_trigger` / `api_trigger`（`$` 兜底分支下）手动触发
- `crontab` 定时触发（放在具体分支名下）

本阶段不实现上述入口，仅在文档与 `.cnb.yml` 注释中预留。

### Rust-only CI

不依赖 Node 环境。因为当前主要是单人 PR，可以偏完整：

```bash
cargo fmt --check
cargo check --benches
cargo test
cargo check --no-default-features --features browser-wasm --target wasm32-unknown-unknown
```

### Node-dependent CI

使用固定 Node.js Docker image。第一版只跑 browser offscreen bench，不额外跑 browser JS smoke test。

职责：

- 使用 `.cnb/images/browser-wasm-ci.Dockerfile` 构建并缓存 browser WASM CI 环境。
- 预装 Rust wasm target。
- 预装匹配版本 `wasm-bindgen-cli`。
- 预装 `bencher` CLI。
- 自动生成 Node wasm-bindgen glue。
- 跑完整 offscreen replay bench。
- 生成 JSONL / JSON / Markdown / env artifacts，并在配置 Bencher secret 后上报 summary 指标。

启动优化：

- `docker.build.versionBy` 绑定 `.cnb/images/browser-wasm-ci.Dockerfile` 与 `Cargo.lock`，仅当工具链定义或 wasm-bindgen 版本变化时重建环境镜像。
- `CARGO_TARGET_DIR` 对 browser WASM pipeline 隔离到 `target/cnb/browser-wasm-probe` 与 `target/cnb/browser-offscreen`，避免 release / bench / wasm 产物互相污染。
- rust-only CI 保留默认 `target/debug`，因为部分 CLI 集成测试会直接执行 `target/debug/metadata-checker`。
- browser WASM 构建脚本必须尊重 `CARGO_TARGET_DIR`，否则 CI target 缓存不会生效。

如果 CNB.cool 支持 artifact 预览或静态报告预览，可以额外保留可视化报告。CI 判断和未来基线对比不得依赖可视化页面。

## 落地顺序

1. 保存本方案作为 bench contract。
2. 配置 CNB.cool 远程仓库。
3. 确认 CNB.cool Docker image、artifact、缓存、`wasm-bindgen-cli` 安装方式。
4. 生成 fixture/manifest。
5. 实现 WASM 自动构建、Node replay runner、summary、validate 和 Makefile。
6. 编写 CNB.cool 流水线。
