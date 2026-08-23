# M58.2 续作：逐 case provenance 与语料扩充计划

> 状态：**approved**（用户 2026-08-23 指示完成 M58.2 剩余缺口）
> 范围：M58.2 阶段 A 状态表遗留的两个「未做」——(b) 逐 case `provenance`、(c) 语料扩充。
> 「配对网格 variant × case × trial」已随 M58.4 Phase B 落地（`.cnb.yml` env 网格 2×6×3=36），
> 本计划不含网格实现，只做 journal 勘误回填。
> 上游：[M58.2 spec](../specs/2026-08-09-slm-eval-harness-design.md)（approved）、
> [M58.2 LLMOps plan](2026-08-23-m58-2-llmops-eval-loop-plan.md)（approved，PR1/PR2 已 done）。
> journal：[m58-cheap-model-comprehension-eval.md](../milestones/ai-eval/m58-cheap-model-comprehension-eval.md)。

## 背景与缺口

fixture `tests/fixtures/corpus/ai_eval/xiaoshouyi_large_real_cases.json` 的顶层 `provenance`
（:25-36）自述：8 个 case 共用一份事后追记的 provenance，`claimed_validation` 三项在仓库里
没有对应产物，`evidence_status: recalled_unverifiable`。其 note 明确要求：**后续新增 case
必须逐 case 写 provenance，并把产物落到 `evidence_artifacts` 指向的路径。**

语料现状：8 个 case（hard×2 / expert×3 / easy×3），6 个入冒烟；hard 层冒烟内只有 1 个
（`xiaoshouyi_text41_display_conditions`），是明显短板。语料扩充到位之前，所有冒烟数字
都只是调试仪表。

## PR 切分

### PR1 — 勘误 + 逐 case provenance schema + 8 个存量 case 复核

- journal 勘误：配对网格行改 done（已做，单独小 commit）。
- `tests/support/kimi_smoke_judge.rs` 过时注释勘误（variant 恒为 baseline → 实际由 env 网格驱动）。
- fixture 每个 case 增加 `provenance` 对象，字段对齐顶层并加验证坐标：

  ```json
  "provenance": {
    "extracted_by": "…",
    "extraction_method": "…",
    "evidence_status": "replay_verified | recalled_unverifiable",
    "claimed_validation": ["…"],
    "evidence_artifacts": ["docs/ai-eval-runs/2026-08-23-m58-2-gold-verification/<case_id>/"],
    "verified_with": { "date": "…", "corpus": "… @ 6920ac51", "binary": "metadata-checker @ <sha> release (cli-local)" },
    "note": "…"
  }
  ```

- 复核方式：CNB workspace 上用**当前源码**编的 release 二进制（`--features cli-local`），
  在 pin 住的语料（`xiaoshouyi-corpus @ 6920ac51`）上逐 case 重跑 `required_commands`，
  逐条核对 `expected_output_assertions`。产物落
  `docs/ai-eval-runs/2026-08-23-m58-2-gold-verification/<case_id>/`（命令清单 + 退出码、
  原始 CLI JSON 输出、逐条断言核对结果）。
- 判定口径：全部断言通过 → `replay_verified`；任何断言不通过 → 如实记录为 gold 漂移，
  修 gold 或标记 `recalled_unverifiable`，**不允许**用新的不可核查断言填缺口。
- schema 守卫测试：每个 case 必须有 `provenance`；`evidence_status` 只能取枚举值；
  `replay_verified` 的 case 必须有非空 `evidence_artifacts` 且路径在仓库内存在。

### PR2 — 语料扩充

- 目标：每个难度层冒烟内 ≥2 个 case。优先补 hard 层（当前冒烟内 1 个）。
- 新增 case 流程（与既有 8 个 case 同构）：
  1. 从 pin 语料的真实结构出发命题（页面/组件/模型必须真实存在）；
  2. 用当前 release 二进制实跑取事实，写 `standard_answer` / `expected_facts` /
     `forbidden_claims` / `expected_output_assertions`；
  3. 逐 case `provenance` + 证据产物（同 PR1 目录约定）；
  4. `eval_count`、`smoke.order`（全局唯一正整数）同步。
- 同步面：干跑脚本 `scripts/cnb-smoke-dry-run.sh` 的计数断言（record 数 = case 数 × 2 variant
  × 3 trial、每 variant×trial 的 transcript 份数）；引用 6-case 集合的测试与文档。

## 非目标

- 不改 judge 语义判定逻辑、不动已冻结的 JSON-command runner（`tests/support/m58_ai_eval.rs`）。
- 不改 SKILL.md / CLI 动词 / 输出 schema（属 M58.3 面）。
- 不把扩充后的冒烟数字写成产品结论；引用规则不变（语料扩充到位前都是调试仪表）。
- 不把 `recalled_unverifiable` 的存量 case 洗成已验证——重放不过就如实记。

## 验收

1. 每个 case 有合法 `provenance`，守卫测试绿；`replay_verified` 的 case 证据产物可复核
   （产物里的命令在 pin 语料 + 记录的二进制坐标下可重放）。
2. 语料扩充后每层冒烟 ≥2 个 case；新 case 的 gold 全部由实跑证据支撑。
3. 远端 `cargo fmt --check`、`kimi_harness_judge_tests`、`m58_3_command_surface_tests`、
   `ai_eval_tests` 绿；本地干跑正常路径绿、四条故障注入全部判红。
4. journal / INDEX 链到本计划并回填状态。
