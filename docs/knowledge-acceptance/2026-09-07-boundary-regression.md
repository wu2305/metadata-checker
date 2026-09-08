# PR33 合并后的知识边界回归

PR33 已 squash 合入 `main`（`be23369`），后续修复在 PR34。此前
[30 pass / 1 partial / 1 fail](2026-09-07-pr33-review.md) 的原始记录保持不变。

## 范围与预先确定的判定

本轮只澄清两篇主题文档中的执行层和证据用途，删除误提交的两个 Python 缓存并忽略后续缓存。
Rust 实现、评测器、system prompt、top-k=5 和日常 `forceRebuild: false` 均未修改。
修复前提交 `0ceec9c` 冻结 [八道边界题](questions-boundaries.json)。这些题根据已知 A4/C3
错误设计，修复也针对这些边界，因此八题从一开始就标为 `boundary_regression`。
加上原32题均为开发回归，不是独立验收或泛化能力证明。
本轮基于合并后的 `be23369`，还包含上一轮 `31d0204` 之后他人修正的行号和语料说明；
与上一轮分数的差异不能全部归因于本轮两个小节，也未排除模型回答波动。

人工检查完整回答中的每个附加断言；仅主结论正确不足以通过。主要结论正确但混淆执行层
计 partial，编造机制或证据资格计 fail。E1–E8 的检查重点如下：

| 题目 | 必须保留的边界 |
|---|---|
| E1 | stdio 的三个 `if` 分支分别调用 diff handler、status、reload 后返回 |
| E2 | 不把 runtime 可变借用注释用于解释 stdio 三个分支 |
| E3 | runtime 内部 ReloadGraph/CheckReload 处理与 stdio 提前返回是不同层 |
| E4 | 普通请求的 check_reload 布尔选项不使查询短路；失败只记诊断 |
| E5 | 旧后端测量不证明 Grafeo；原因是测量对象，而非报告目录 |
| E6 | 排除索引不取消其作为旧后端测量记录的用途 |
| E7 | 搬动文件不改变实际测量对象，不能因此验收 Grafeo |
| E8 | 只有目录名不足以判断证据有效性，须核对后端、版本和运行条件 |

以上重点是必要条件，不能替代对检索证据是否支持完整回答的检查。

## 复现

索引与 runner 固定为 `3c8257d`。一次性 CNB 配置先强制重建白名单语料，
再执行16项脚本测试和40题独立问答；配置通过 YAML、语义、Schema 校验。
构建：[cnb-ieo-1k1u90c26](https://cnb.cool/wu2305/metadata-checker/-/build/logs/cnb-ieo-1k1u90c26)。
完整一次性配置见 [归档](runs/2026-09-07-boundary-regression.cnb.yml)，仅用于手动重放，
不是仓库日常流水线入口。

将 `questions.json` 数组与 `questions-boundaries.json` 数组依次拼接，使用
`json.dumps(questions, ensure_ascii=False, indent=2) + "\n"` 编码为 UTF-8 临时文件，
传给现有 `scripts/kb-evaluate.py --questions`。runner 保留该组合题集 hash。
问答模型只收到本题与检索片段，不收到本报告、其他题目或答案键。

## 结果与剩余工作

本轮 **35 pass / 2 partial / 3 fail**：原32题为27/2/3，新增八题为8/0/0。
A4、C3 在本轮通过，但整体知识库问答仍未通过，不能据此扩展主题。
完整 [原始产物](runs/2026-09-07-boundary-regression.json.gz)、
[逐题判定](runs/2026-09-07-boundary-regression.judgments.json)、
[完整性清单](runs/2026-09-07-boundary-regression.integrity.json) 均保存在不入库目录。

| 未通过题 | 判定 | 原因 |
|---|---|---|
| B3 | fail | TBL/SPG 主结论正确，但编造 CLI 文档出现坏 SPG 且可能与扫描文档冲突 |
| B5 | partial | 阈值正确，但把首次初始化归为从 Stale 回到 Current，混淆初始状态与状态迁移 |
| C6 | fail | top5 未召回 human 旧路径条目，答未知；拒绝编造是正确行为，但整体检索问答失败 |
| C7 | fail | M28 未做的结论正确，却无据断言 human/non-human 缺陷与 M28 无关 |
| H1 | partial | 引文有“未预置时”，首句却无条件宣称缺 token 即失败，遗漏预置目录例外 |

下一步应分开验证两个问题：

1. **召回缺失**：针对 C6 固定查询和索引，保留重复查询的完整 top5，确认正确条目
   的排名与分块位置后再改组织方式；不能把未知回答算成知识库已经覆盖该问题。
2. **附加推断**：固定 B3/C7 等题的检索片段，对比回答模型或逐条证据检查方法，
   记录所有请求及结果；不要同时改检索与回答规则后归因，也不要继续仅靠往条目追加答案调绿。

本轮没有修改这些新失败对应的条目或提示词，保留它们作为后续实验输入。
本次八题也不能重新包装成外部盲测。

## 验证与限制

- 远端16项脚本测试全部通过，40题采集正常结束。
- 人工逐题核对完整回答；另核验组合题集 hash、runner hash、200个片段的白名单来源与 SHA、
  40个独立双消息请求、实际模型 `deepseek-v4-flash`、索引前后相同，以及五个索引文件的 Git blob。
- 本轮归档 SHA256：`830b307ee519b46e608f36e505b18ac6fd0c9f49dae53080b1610092db5f8de5`。
- 只修文档语义，不新增 Rust 行为测试，也未在本地运行 Cargo。PR 检查以 CNB Checks 为准；
  检查成功不代表问答质量通过，也不提供新的 full-criterion 真实语料测量。
