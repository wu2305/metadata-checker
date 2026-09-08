# 2026-09-08 归档分支逐项改动复核

以 PR36 合并后的 `main=3d595d3d61f1c926af8206ed160e04269bb1b9d5` 为固定源码基线，
继续核查 [历史版本报告](branch-retention-history-2026-09-08.md) 中没有精确版本证据的
43 项。计数单位始终是「归档分支、路径」组合，不是独立文件数。

本轮 **24 项完成改动去向核查**；加上 PR36 已确认的 1 项延期原型，仍有 **18 项未决**。
其中一份评测设计稿此前未进入当前文档树，本轮恢复历史原文。没有恢复旧代码或删除归档 tag。
128 项已有的精确历史版本证据不变，也没有因此升级为当前行为等价证明。

PR37 合并后的继续核查见 [性能归档改动去向复核](branch-retention-performance-2026-09-08.md)。
下文与本报告 JSON 保留本轮快照，后续结论单独记录。

## 已核查结果

| 归档分组 | 本轮项数 | 结果与主要证据 |
|---|---:|---|
| backup-before-resplit | 5 | 固定六问迁移为 fixture 驱动子集、records/trials与阶段B；navigation 的action_type/raw_expr保留，并收紧为真实跳转边；spec随批准方案演进 |
| local-main-20260906 | 8 | batch warm、反向依赖、计时分层、缓存预算规则与旧测试断言保留；后置clear已由物化前过滤替代 |
| m51-performance-diagnosis | 1 | src/lib.rs的两个新增profile模块导出原样保留 |
| m52-redb-v2-and-ci | 3 | 格式草稿保留并纠正旧编号/适配范围；CHECKLIST由INDEX接管；另恢复尚未进入当前树的规则覆盖与加权评分草稿 |
| pr33-kb-fixes | 7 | 入库边界、问题集迁移、完整原始回答要求和主题内容保留；错误事实、行号与验收口径经后续修正 |

逐项归档提交、base、mode/blob、完整 patch 的 SHA-256、对应 main 位置和限制，均见
[JSON](branch-retention-patches-2026-09-08.json)。证据文字是审阅摘要；不是运行日志或
逐字引文。测试比较检查了具体断言，未用函数名存在替代正文核对。
JSON 内 `evidence` 有四种形态：自由文本审阅摘要、`{ref,summary}` 引用既有结论、
`{ref,summary,finding}` 本轮核查（ref 指归档定位，finding 指相对 pinned main 的发现）、
`{detail,line,path}` 精确位置；形态语义不同，字段说明见 JSON 顶层 `evidence_schema`。

本轮修正了三处仍会误导后续 agent 的文档：

- M53 的“待合入”与 CHECKLIST/planning 中的“M53 active”改为当前 INDEX 对应状态。
  这里只确认旧实现/断言在 main，不重新宣称历史性能测量已执行。
- harness spec 的“阶段B独立pipeline”改为同一 pipeline 的下一 stage，匹配
  `.cnb.yml:1323` 与 `test_cnb_stage_b_judge_follows_smoke_in_same_pipeline`。
- [规则覆盖与加权评分草稿](../archive/proposals/2026-07-05-skill-rule-coverage-and-score.md)
  按原字节恢复，单列未批准/未实现/旧编号失效说明；不启用旧权重或门禁。

## 未决项与验收限制

18 项全部来自性能累积改动：M51 的 7 项、M52-performance-optimization 的 11 项。
机器清单逐项标为 `unresolved`；它们涉及 explain/profile、page-logic、prerequisites、
path/runtime及其测试。未完成对所有旧 hunk 的行为对照，不据此认定丢失或已覆盖。
这些项仍需逐段检查并给出保留、替换、延期或待恢复的证据。

核查也发现保留的测试不一定充分：`m53_compact_warm_cache_trims_unused_path_categories`
中 `full.candidate_paths >= compact.candidate_paths` 在两者都为0时仍通过。
本报告依靠源码桶规则与完整旧新测试 diff 判断改动去向，没有将这条断言当独立正向验收。
若后续加固，应使用会生成被裁剪桶的固定图，明确断言 full 非空和 compact 为空。

PR33 历史题表仍留有旧 A4 行号和过时 B4 表述；它们是保留的历史题键，当前报告和语料
已记录纠正，本轮不改写历史回答。保留一份文档不等于认定其所有历史断言仍正确。

## 验证口径

- 重算24项 base→archive patch hash，并核对归档tag解引用后的commit、mode/blob；
  43项清单无重复遗漏，24本轮＋1前轮＋18未决，原两份JSON未改写。
- local-main的warm scalability旧测试与main仅格式差异；runtime旧测试仅格式变化和
  两个后续新增用例；runtime新增函数逐段核对，unique Arc路径只改了格式/尾逗号。
- 恢复草稿的“历史原文”与归档blob逐字相等；新报告链接和JSON可解析，`git diff --check`通过。
- 本轮只改文档及证据，没有运行本地Cargo、修改Rust、生产提示词或知识库语料。
  索引日志和main性能流水线失败另见 [索引生成记录](../knowledge-acceptance/2026-09-08-index-provenance.md)。
