# M16：SKILL.md 真实使用协议收敛

| milestone | M16 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M16：SKILL.md 真实使用协议收敛

### 目标

把工具能力边界、fallback 路径和保守回答规范写成模型不会误解的任务协议。

### 问题清单

- `SKILL.md` 对 graphdb 缺失、权限失败、锁失败没有明确决策树。
- `SKILL.md` 没有把 `<graph-edge-derived>`、`node_id="?"` 定义为弱证据。
- `SKILL.md` 没有强调 `details` 长数组默认不可全读。
- `SKILL.md` 没有针对单文件 `.tbl`、项目级 `.tbl`、页面内局部 DataFlow 的不同路径给出清晰命令选择。
- `SKILL.md` 没有明确“summary 与 details 冲突时如何回答”，例如模型读写 0 但 DataFlow 消费 52。
- `SKILL.md` 没有要求 AI 在目标 ID 不确定时先使用 find/resolve，而不是猜测。
- `SKILL.md` 对 `--human` 与机器模式边界已经有说明，但没有真实项目故障处理示例。

### 验收目标（M16 已收敛）

- `SKILL.md` 增加“真实项目项目级查询决策树”：check graph、build graph、query、fallback。
- `SKILL.md` 增加“证据强弱分级”：真实 JSON 路径强，graph-derived 中，缺路径弱。
- `SKILL.md` 增加“长输出读取预算”：先 brief，再按需 details，不默认 full。
- `SKILL.md` 增加 `.tbl` 和 DataFlow 问题的命令路径。
- `SKILL.md` 增加冲突处理规则：summary 不完整时结合 details 中角色字段，但必须说明口径。
- `SKILL.md` 增加命令引用规则：所有 target 使用单引号。
- 空上下文 `5.4-mini` 能按协议完成至少 5 个真实项目问题，不出现已知误判。
