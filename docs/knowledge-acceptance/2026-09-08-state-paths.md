# PR34：源码反例修正与状态路径回归

本次修正知识条目与评分，不修改 Rust、评测器、提示词、top-k 或日常索引配置。
上一轮48题原始回答保持不变；首次评分另存 `initial-judgments.json`，
[报告](2026-09-07-boundary-regression-2.md) 更正为44通过、1部分通过、3失败。
E3 的错误末段和 H1 的无条件首句不再因主体结论正确而放过。

## 修正依据

- `src/graph_redb.rs:1191-1199`：有效 `delta=None` 提交会写 v2 并置 Current，
  不限于新库首次构建，也不依赖1024阈值。`:1007-1023` 的空提交先返回，不能说调用 persist 必重建。
  checkpoint-only 的 `Some` 保持旧状态，`persist_with_checkpoint` 传 `None` 走全量写入。
- `src/stdio_server.rs:451-459`：普通查询在检查 reload 后保留原 command，
  将 check_reload 设 false 再传 runtime，不会改成 CheckReload 命令。
- 主题二 §1.6 的说明移到完整表格之后，锁/hydrate 等行不再成为孤立的竖线文本。

源码与主题文档分支基线 `715ef3c` 一致。本轮修正源版本为 `58d725b`。

## 运行与判定

原48题加 [四道状态路径题](questions-state-paths.json) 共52题，均为开发回归。
新增题在修正条目之后、首次执行之前提交，用于核对已知反例；不是外部盲测。
G1 要区分已有 Stale 库的有效全量提交与首次初始化；G2 必须保留空提交例外；
G3 要区分 checkpoint-only 的 Some/None；G4 必须保留原 QueryModel 命令。
每题完整回答的附加断言也须有支持；错误机制为 fail，前提不精确为 partial。

[一次性配置](runs/2026-09-08-state-paths.cnb.yml) 包含强制重建、16项脚本测试与52题独立请求。
执行构建 [cnb-vud-1k1v5efku](https://cnb.cool/wu2305/metadata-checker/-/build/logs/cnb-vud-1k1v5efku)，
索引/runner 均固定 `58d725b`，模型请求与完整 SSE 由现有 runner 记录。
组合题集按四个文件的顺序合并，以 `ensure_ascii=False, indent=2` 和末尾换行编码为 UTF-8；
配置中保留确切命令，可用归档的 SHA 与题集 hash 重放。

## 结果

本轮 **44 pass / 5 partial / 3 fail**，整体仍未通过。新增 G1/G2/G4 通过，G3 部分通过。
显式全量提交与原命令保留的条目已纠正，但不能据此宣称回答模型可靠。
完整 [产物](runs/2026-09-08-state-paths.json.gz)、
[逐题判定](runs/2026-09-08-state-paths.judgments.json)、
[完整性清单](runs/2026-09-08-state-paths.integrity.json) 均不入索引。

| 题目 | 判定 | 保留的问题 |
|---|---|---|
| A6 | partial | 将打开库的CLI/stdio差异归到adapter层 |
| B6 | fail | 将scanner诊断的读取入口误说为hydrate_diagnostics；实际为load_scanner_diagnostic_entries |
| H1 | fail | top5未召回CI条目，只能答未知；安全拒答不等于检索问答通过 |
| E3 | fail | 又将CheckReload列入stdio提前返回命令，完整回答仍与源码冲突 |
| F2 | partial | 增量checkpoint-only问题首答会变，再用全量路径补充，改变题目前提 |
| F3 | partial | 从SPG终止本轮外推不会继续下一轮，未保留未来可重试的边界 |
| F6 | partial | 从知识片段未列出缺陷外推M28计划没有具体缺陷 |
| G3 | partial | 将Some的checkpoint-only保持状态限定为未达阈值，实际无图变更时不判断阈值 |

H1的top1分数约0.0033，五个片段都来自主题文档；E3的top1约0.9987，且召回包含正确的
stdio分支说明。这是一次运行中观察到的不同失败类型，不能仅用分数证明回答正确。
后续应固定索引与查询诊断H1召回，固定E3/B6片段对比模型或逐断言证据检查；
本轮不再追加答案调同一题集，不扩大主题，也不声称已排除模型波动。

## 验证

- 一次性配置通过YAML、语义、Schema校验；远端16项脚本回归通过，52题完整采集。
- 核验52题组合hash、runner hash、260个片段来源URL与SHA、52个独立双消息请求、
  实际模型deepseek-v4-flash、索引前后相同，以及五个入库文件与源版本blob一致。
- 产物SHA256：`a0a4abebcd6d7f3d5d810fbe17a94e76260d2023f08fb5098d25081555b9dffc`。
- 表格结构检查确认七个场景在同一张表内；旧回答与首次评分逐字保留，修订计数一致。
- 源修正提交的PR Rust/coverage/browser-wasm检查通过（`cnb-oag-1k1v5efde`）；
  browser offscreen bench跳过，不计为通过。本地未运行Cargo，无新增真实语料性能验收。
