# 知识库语料（白名单）

> 本目录是**唯一**被 CNB 知识库流水线索引的目录（除 `AGENTS.md` / `SKILL.md`）。
> 写入本目录的文档必须遵守下方规范，否则会污染检索。

## 为什么单开一个目录

`docs/` 下 253 个跟踪文件里，`archive/` 46 个、`ai-eval-runs/` 115 个。
默认全量 Markdown 索引会把已归档的历史结论、评测答案键与当前事实混在一起召回，
向量检索无法区分新旧。首版采用**显式白名单**：只有本目录 + `AGENTS.md` + `SKILL.md` 入库。

## 写文档规则

| 规则 | 说明 |
|------|------|
| 状态分层 | 每个主题必须分「当前实现 / 已批准计划 / 已知缺陷 / 历史状态」四节，缺节写「无」 |
| 源码依据 | 每条调用链结论必须带 `src/xxx.rs:NNN` 与分析 SHA；无源码依据标「未知」 |
| 自包含 | 每个小节自带状态与限制，不依赖前文（分块 `chunkOverlap: 0` 会切断上下文） |
| 否定结论 | 「尚未实现」「未接入」「不代表验收」必须写在描述同一小节内 |
| 脱敏 | 不含真实凭据、内网域名、本机绝对路径 |

## 验收

- [knowledge-acceptance-questions.md](knowledge-acceptance-questions.md)：约 20 条真实开发问题，含反误导题
- 合并后由空上下文 agent 逐题查询，记录召回片段与回答是否正确

## 索引配置

见 `.cnb.yml` 的 `.knowledge_base_ci`：`issueSyncEnabled: false`、
`ignoreProcessFailures: false`、显式 `include`/`exclude`。
