# CNB 冒烟 stage 的本地 dash 干跑（M58.2）

> 适用：`.cnb.yml` 的 `api_trigger_kimi_harness_smoke` → `kimi-code harness smoke run` stage
> 与 `report smoke summary` endStage（两者都逐字提取、dash 执行）。
> 工具：[`scripts/cnb-smoke-dry-run.sh`](../../scripts/cnb-smoke-dry-run.sh)

## 为什么有这份 runbook

CNB stage 脚本由 `sh` 执行，`browser-wasm-ci` 基于 `node:22-bookworm`，`/bin/sh` 是 **dash**。

这条约束以最坏的方式暴露过一次：冒烟循环最初写成 `while IFS=$'\t' read -r ...`。dash 不认 ANSI-C quoting，把 `$'\t'` 当作字面的 `$ ' \ t` 四个字符，于是按字母 `t` 切分 —— case_id 被切碎、transcript 互相覆盖、问题文本变成垃圾，而**退出码全 0、记录数断言通过、token 检查通过，整条流水线全绿**。

关键教训：**`bash -n` 与 bash 下的 stub 干跑都查不出这一类缺陷。** 它们查语法，而这是语义差异——在 bash 下这段代码完全正确。唯一有效的验证是用 dash 实跑。

## 怎么跑

```sh
# 正常路径：期望退出 0，产出 6 条 record
scripts/cnb-smoke-dry-run.sh

# 故障注入：每一项都必须判红，绿了就是缺陷
scripts/cnb-smoke-dry-run.sh --fault empty   # 没有 smoke.enabled 的 case
scripts/cnb-smoke-dry-run.sh --fault dup     # smoke.order 撞号
scripts/cnb-smoke-dry-run.sh --fault noeat   # 循环中途丢一条 record
scripts/cnb-smoke-dry-run.sh --fault judgeleak # judge 产物与 stdout 含 token
```

脚本自身对结果做断言：正常路径非 0 判失败，注入故障后退出 0 同样判失败。`judgeleak` 还会断言 token 既未进入 stage/endStage 日志，也未残留在产物中，且 **main stage 判红后 endStage 仍发布脱敏后的 `judge.md`**——判分依据不能随非零退出码一起埋掉。**「故障路径必须红」和「正常路径必须绿」同等重要**——静默假绿或失败路径泄密正是这套东西要防的失效模式。

前置：`dash`（`brew install dash`）、`python3` + `pyyaml`、`node`。

## 两条设计约束

**1. stage 与 endStage 脚本逐字从 `.cnb.yml` 提取，不手抄。**

脚本用 `python3` + `pyyaml` 解析 `.cnb.yml`，按 stage 名取出 `script` 原文（endStage 同理，取 `report smoke summary`）。手抄的副本会和真身漂移，而漂移的方向恰好是「我以为我验过了」。stage 改名时提取会直接报错，不会静默跑一份空脚本。endStage 与 CNB 行为一致，无论 main stage 退出码如何都会执行——失败路径的报告发布（`judge.md`、records 投影、run manifest）正是它的职责。

**2. 只 stub 外部依赖，不改 shell 语义。**

| stub | 顶替原因 |
|------|----------|
| `date` | macOS 的 BSD date 没有 `%3N`（毫秒）；stub 补 GNU 行为 |
| `timeout` | BSD 无此命令；stub 丢掉时长后直接 exec |
| `kimi` | 不调真模型；产出与真实 stream-json 同形的 transcript（assistant 行携带 tool_calls 数组），让 mc/raw fallback 结构化解析真正跑起来 |
| `cargo` | judge 需要 `CNB_TOKEN` 与真模型；stub 伪造 `judge.json` / `judge.md` 契约 |
| `ln` | stage 会往 `/usr/local/bin` 建链接，开发机上不该写那里 |
| `metadata-checker` | stage 用 `command -v` 定位二进制算 sha256 写 run.json，开发机上没有真身 |
| `sha256sum` | macOS 没有 GNU coreutils；stub 用 `shasum -a 256` 顶，镜像里有真身 |

stub 补的是**镜像里本来就有、开发机上没有**的 GNU 行为。任何为了「让脚本跑起来」而改写 stage 语法的做法都会让这次验证失去意义。

故障注入改的是**输入数据**（case fixture）或外部命令行为，同样不动 stage 脚本本身。`judgeleak` 让 cargo stub 把 token 同时写进 `judge.md` 与 stdout，用于验证 stage 必须先捕获 judge 日志、扫描并脱敏完整目录，之后才能输出日志并判红。

## 覆盖边界

干跑验证的是 shell 语义、控制流、故障路径判红、judge 日志的脱敏顺序、records 字段与取值、endStage 的报告发布（正常与失败路径都发布 `judge.md` 与 run manifest），以及三组行为：mc/raw fallback 的结构化计数（kimi stub 对「入口」一问额外模拟直接读 `.spg`）、开跑时清理上一轮残留产物、`run.json` 身份清单产出。它**不**验证：真实模型行为、真实项目图构建、判分质量、墙钟量级（stub 是毫秒级，真跑是分钟级），也不覆盖 `fetch real project corpus` stage 的 corpus pin/检出逻辑——那是独立 stage，干跑没有 git stub。

因此干跑通过**不等于**验收通过。stage 的真实验收是在 CNB 上跑一次 `api_trigger_kimi_harness_smoke`。

## 配套的 schema 防线

records.jsonl 的产出方是 `.cnb.yml` 里的 node 脚本，消费方是 Rust，中间没有编译期联系。`tests/kimi_harness_judge_tests.rs` 的 `test_cnb_record_emitter_matches_trial_record_schema` 直接解析 `.cnb.yml` 比对两侧字段名，把字段错字拦在 `cargo test`：

```sh
cargo test --features cli-local --test kimi_harness_judge_tests
```

判分 stage 另外会就地校验真实的 `records.jsonl`（schema、空文件、负墙钟、`(case_id, variant, trial)` 撞号）。
