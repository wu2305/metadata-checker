# metadata-checker

低代码平台 SuperPage 元数据解析与分析 CLI 工具。

## 功能概述

- **解析 `.spg` 文件**：提取组件树、表达式、动作、页面参数和数据源
- **表达式分析**：识别 30+ 表达式字段，解析引用类型（组件值、模型字段、页面参数、用户属性、系统变量）
- **依赖图构建**：组件间依赖关系、拓扑排序、循环检测
- **计算优先级分析**：`defaultValue` vs `exp` vs `calcCondition`
- **值来源追溯**：递归展开表达式，追踪值的完整来源链
- **跨文件分析**：扫描整个项目目录，构建图数据库（Page/Component/Model/Field/Action 节点）
- **DataFlow 子图展开**：字段级来源追溯，支持节点类型识别（Select/Join/Union/Distinct 等）

## 快速开始

### 编译

```bash
cargo build --release
```

Release 二进制约 1.6MB，支持跨平台编译（`windows_amd64`、`macos_arm64`、`linux_amd64`、`linux_arm64`）。

### 解析单个页面

```bash
# 默认：机器友好的 JSON 输出（单个合法 JSON）
./target/release/metadata-checker page.spg

# 带优先级分析（合并进主 JSON）
./target/release/metadata-checker page.spg --priority

# 查询特定组件（JSON 输出）
./target/release/metadata-checker page.spg --query input3

# 查询特定组件并带优先级分析（单 JSON）
./target/release/metadata-checker page.spg --query input3 --priority

# 进入交互式 REPL（专家探索模式）
./target/release/metadata-checker page.spg --human

# 交互模式带优先级分析
./target/release/metadata-checker page.spg --human --priority

# 交互式查询（REPL）
./target/release/metadata-checker page.spg --interactive
```

### 解析单个 .tbl 文件

```bash
# 物理表（AppTable）：查看字段定义
./target/release/metadata-checker app_table.tbl

# DataFlow：查看输入、输出和字段加工链
./target/release/metadata-checker dataflow_output.tbl

# DataFlow 人类可读摘要
./target/release/metadata-checker dataflow_output.tbl --human
```

输出结构：
- `kind: Table`（物理表）或 `kind: DataFlow`（加工表）
- `summary.table_type`、`field_count`、`input_count`、`output_count`
- `details.fields`：字段列表
- `details.dataflow_inputs`：DataFlow 输入节点
- `details.dataflow_outputs`：DataFlow 输出目标
- `details.field_lineage`：字段来源链（inputField / exp / originalField）

### 项目级图数据库

```bash
# 扫描项目目录并构建/更新图数据库（默认路径：<project-dir>/.metadata-checker.graphdb）
./target/release/metadata-checker --project-dir /path/to/project --build-graph

# 构建到自定义路径（适用于只读项目目录或沙箱环境）
./target/release/metadata-checker --project-dir /path/to/project --build-graph --graph-db-path /tmp/my_project.graphdb

# 检查图数据库状态
./target/release/metadata-checker --check-graph --graph-db-path /tmp/my_project.graphdb

# 查询模型的读写关系与 DataFlow lineage
./target/release/metadata-checker --project-dir /path/to/project --query-model model1

# query-model 输出字段说明：
# - readers: 读取该模型的页面/组件/动作
# - writers: 写入该模型的页面/组件/动作
# - dataflow_inputs: DataFlow 的输入依赖（ outgoing DataflowInput 边）
# - dataflow_outputs: DataFlow 的输出目标（ outgoing OutputsTo 边）
# - produced_by: 物理表的生产者（ incoming OutputsTo 边）
# - consumed_by_dataflows: 消费该输入表的 DataFlow（ incoming DataflowInput 边）
# - upstream_dependencies / downstream_outputs: 兼容旧字段，不建议新模型依赖

# 查询页面的跨文件关系
./target/release/metadata-checker --project-dir /path/to/project --query-page "page/合同管理/销售合同"

# 查询两个页面之间的关系
./target/release/metadata-checker --project-dir /path/to/project --query-cross "page/A" "page/B"

# 展开 DataFlow 子图
./target/release/metadata-checker --project-dir /path/to/project --query-dataflow flow.tbl

# 解释一个组件（单文件模式）
./target/release/metadata-checker page.spg --explain button1

# 解释一个模型（项目图模式）
./target/release/metadata-checker --project-dir /path/to/project --explain model:model1

# 解释一个页面
./target/release/metadata-checker --project-dir /path/to/project --explain page:app/page.spg

# 查看组件周围上下文（深度 2，紧凑输出）
./target/release/metadata-checker --project-dir /path/to/project --context comp:app/page.spg|button1 --depth 2 --budget compact

# 查看字段周围上下文
./target/release/metadata-checker --project-dir /path/to/project --context field:model1.name --depth 2 --budget normal

# 页面级逻辑摘要（推荐 AI 优先使用）
./target/release/metadata-checker --project-dir /path/to/project --query-page-logic page:app/page.spg

# PageLogic 典型输出片段：
# {
#   "kind": "PageLogic",
#   "summary": {
#     "what_is_it": "页面 actions_test，5 个用户入口，读取 0 个数据源，写入 9 个目标，0 个跳转",
#     "page_role": "data_maintenance_page",
#     "entrypoint_count": 5,
#     "write_target_count": 9
#   },
#   "details": {
#     "entrypoints": [...],
#     "action_flows": [...],
#     "write_targets": [...],
#     "navigation": [...],
#     "visibility_rules": [...],
#     "risk_diagnostics": [...]
#   }
# }
```

### human 模式

`--human` 进入交互式 REPL（专家探索模式），包含参数、数据源、组件表达式、依赖顺序、循环警告等。

### 交互模式

`--interactive` 进入 REPL，提示输入组件 ID 进行详细查询：

```
=== SuperPage Interactive Mode ===
Version: 4.19.7 | Theme: default | Components: 5 | Expressions: 6
请输入要查询的组件 ID（直接回车退出）: input3
```

## CLI 参数

```
Usage: metadata-checker [OPTIONS] [FILE]

Arguments:
  [FILE]                    页面元数据 JSON 文件路径

Options:
      --human               进入交互式 REPL（专家探索模式）
      --interactive         --human 的别名，同样进入 REPL
      --non-human           机器友好的 JSON 输出（默认，单 JSON）
      --query <ID>          查询特定组件 ID 的详细信息
      --explain <ID>        解释指定 ID 的语义（component/action/model/field/page/dataflow）
      --context <ID>        输出目标节点周围的最小闭包上下文
      --depth <N>           上下文深度（默认 1，配合 --context 使用）
      --budget <B>           输出体积控制：compact|normal|full（默认 normal）
      --priority            附加计算优先级分析（non-human 合并进 JSON）
      --detail              输出完整原始结构（non-human 模式）
      --project-dir <DIR>   项目目录（用于跨文件分析）
      --build-graph         从项目目录构建/更新图数据库
      --query-model <MODEL> 查询模型的读写关系（需 --project-dir）
      --query-page <PAGE>   查询页面的依赖关系（需 --project-dir）
      --query-cross <A> <B> 查询两页面间的跨文件关系（需 --project-dir）
      --query-dataflow <M>  展开 DataFlow 模型的内部子图（需 --project-dir）
      --query-page-logic <P> 查询页面级逻辑摘要（需 --project-dir）
  -h, --help                打印帮助信息
  -V, --version             打印版本
```

## 项目结构

```
src/
├── main.rs              # CLI 入口与子命令分发
├── cli.rs               # 命令行参数定义（clap）
├── lib.rs               # 库入口
├── superpage/           # SuperPage 元数据解析
│   ├── mod.rs           # 解析入口、组件提取、上下文解析
│   ├── types.rs         # 公共类型定义（SpgComponent、RefType 等）
│   ├── raw_types.rs     # JSON 反序列化结构体
│   └── expr.rs          # 表达式引用解析（正则、classify_ref）
├── dependency.rs        # 组件依赖图、拓扑排序、循环检测、值追溯
├── priority.rs          # defaultValue/exp/calcCondition 优先级分析
├── output/              # 输出模块
│   ├── mod.rs           # human/JSON 双模式输出、summary/detail
│   └── component.rs     # 组件查询输出（human + JSON）
├── parser.rs            # 文件类型识别、统一元数据结构
├── graph.rs             # petgraph 有向图 + redb 持久化
├── scanner/             # 项目目录扫描
│   ├── mod.rs           # 扫描入口、增量检测
│   ├── utils.rs         # 文件收集、路径解析
│   ├── spg.rs           # SPG 元数据解析和图构建
│   └── tbl.rs           # TBL/DataFlow 元数据解析和图构建
└── query/               # 图查询接口
    ├── mod.rs           # model/page/cross 查询
    └── dataflow.rs      # DataFlow 子图展开、字段级来源追溯

tests/                   # 测试用例（覆盖全部模块）
```

## 图数据库

图数据库存储在 `.metadata-checker.graphdb`（位于 `--project-dir` 根目录，可通过 `--project-dir` 配置）。

**节点类型**：`Page`、`Component`、`Model`、`Field`、`Action`

**边类型**：
- `Reads` — 读取模型字段
- `Writes` / `ActionWrites` — 写入模型字段
- `Triggers` — 触发动作
- `Contains` — 包含关系
- `EmbedsPage` — 嵌入页面
- `OpensPage` / `PassesParam` — 打开页面/传递参数
- `SetsParam` — 设置页面参数
- `OutputsTo` — DataFlow 输出到物理表
- `DataflowInput` — DataFlow 输入源
- `DataflowInternal` — DataFlow 内部节点依赖

## --explain 示例

`--explain <ID>` 按目标类型分发语义，输出 `kind: Explain`：

### 解释组件

```bash
metadata-checker --project-dir ./my-project --explain comp:app/page.spg|button1
```

输出摘要：
```json
{
  "what_is_it": "组件 button1，位于页面 page_relations，具有 1 个动作",
  "type": "component",
  "importance": "entrypoint"
}
```

details 包含：
- `reads`：组件表达式引用的模型字段、参数
- `writes` / `writes_models`：action 写入的模型、参数
- `located_in`：父页面 Contains 关系
- `triggers`：触发的 action 列表
- `navigates_to`：跳转/嵌入的目标页面
- `affects_components`：受影响的组件（ActionControlsComponent / SetsParam）
- `triggered_by`：被触发的来源（incoming Reads），不再包含 Contains
- `affects`：legacy 字段，包含 `components` / `models` / `pages` 子数组

### 解释模型

```bash
metadata-checker --project-dir ./my-project --explain model:model1
```

输出摘要：
```json
{
  "what_is_it": "数据模型 model1，被 2 个组件/动作读取，被 6 个组件/动作写入，参与 0 个 DataFlow",
  "type": "model",
  "importance": "action_target",
  "read_by_count": 2,
  "written_by_count": 6
}
```

### 解释页面

```bash
metadata-checker --project-dir ./my-project --explain page:app/page.spg
```

输出摘要：
```json
{
  "what_is_it": "页面 page_relations，包含 4 个组件，2 个入口点，2 个数据源，0 个写入目标",
  "type": "page",
  "importance": "entrypoint",
  "entrypoint_count": 2,
  "data_source_count": 2
}
```

### 解释动作

```bash
metadata-checker --project-dir ./my-project --explain action:app/page.spg|button1|action1
```

输出摘要：
```json
{
  "what_is_it": "link 动作 link:action1，由 button1 触发，读取 model1",
  "type": "action",
  "importance": "entrypoint",
  "parent_component": "button1"
}
```

### explain 不足时的后续命令

| 场景 | 后续命令 |
|------|----------|
| 需要了解周围关联 | `--context <ID> --depth 2` |
| 需要页面整体逻辑 | `--query-page-logic <PAGE>` |
| 需要模型全量读写 | `--query-model <MODEL>` |
| 需要 DataFlow 子图 | `--query-dataflow <MODEL>` |

## 统一 JSON 输出 Schema（Machine Contract）
## 统一 JSON 输出 Schema（Machine Contract）

所有机器输出（默认 `--non-human`）通过 `src/output/schema.rs` 中的 `AiOutput` struct 统一序列化，**禁止手写 `json!`**。

```json
{
  "schema_version": "1.0",
  "kind": "SuperPage | PageQuery | ModelQuery | CrossPageQuery | DataFlowQuery | ComponentQuery | PriorityQuery | Explain | Context | PageLogic",
  "query_target": "...",
  "summary": { /* 低噪声摘要，AI 优先读取 */ },
  "details": { /* 详细信息，按需展开 */ },
  "evidence": [ /* 证据链：只要 summary 有实质内容，evidence 不能为空 */ ],
  "diagnostics": [ /* 诊断信息：统一包含 severity/code/message/location/suggestion */ ],
  "next_queries": [ /* 建议的下一步查询：必须为真实可运行命令 */ ]
}
```

### AI 使用顺序

1. **先读 `summary`**：建立页面/项目地图，获取高层信息
2. **遇到 `diagnostics` 时保守回答**：不确定的关系必须保守处理
3. **需要核查时读 `evidence`**：验证具体结论的出处
4. **需要细节时读 `details`**：按需展开完整结构化结果

### 兼容字段说明

- `upstream_dependencies` / `downstream_outputs` 为 legacy 兼容字段，不应作为主语义字段
- `--context` 中新增 `related_nodes`（全类型节点闭包）；`related_components` 仅保留 `type=Component` 条目用于兼容
- `--context` 的 `evidence` 含 upstream/downstream 边级采样证据（`compact=3`、`normal=5`、`full=20`）；缺原始路径时 `json_path` 为 `<graph-edge-derived>`
- `action_flows[].action_category` 当前稳定枚举：`data_write` / `data_read` / `navigation` / `param_mutation` / `ui_control` / `validation` / `data_initialization` / `data_refresh` / `unknown`
- `details.expressions[]` 增加 `component_id` / `field` / `source_file` / `json_path` / `refs_count` / `resolved_occurrence_count`，其中 `refs` 为去重集合，`resolved_refs` 为出现级列表；`source_file` 优先真实输入路径
- 当 PageLogic 细项数量超过 evidence 展开上限时，会输出 `EVIDENCE_SAMPLED` 诊断，提示 evidence 为低噪声采样而非全集
- 禁止默认读取 raw JSON，必须从 `summary` 开始

详细字段定义见 `docs/schema.md`。

## 测试

```bash
cargo test
```

150+ 个测试覆盖：
- SuperPage 解析（组件、参数、数据源、表达式）
- 表达式引用解析（简单、复杂、宏、模型、IF 条件）
- 依赖图构建与拓扑排序
- 循环检测
- 优先级分析
- 输出格式化
- 图数据库操作
- 扫描器（动作解析、.tbl 解析、DataFlow 内嵌模型）
- 边界错误处理
- 真实场景用例

### 新增测试（P0-P3）

- **换行表达式**：`tests/fixtures/newline_expressions.spg` — 验证 `\n` 不 panic，引用仍能提取，字符串内换行不误判
- **复杂循环检测**：`tests/fixtures/complex_cycle.spg`（A→B→C→D→B）— 验证环内节点 B/C/D，A 不在环内
- **稳定拓扑排序**：`tests/fixtures/stable_order.spg` — 多次运行结果一致，按元数据出现顺序处理
- **组件值追溯**：单级/多级/多分支 fixture — 验证 `trace_value_source` 能跨组件追溯到 Param/ModelAuto
- **来源分类（SourceType）**：`Param`、`UserInput`、`ModelAuto`、`System`、`Computed`、`Constant`、`Unknown` — 测试覆盖每个枚举值
- **性能与规模**：`tests/fixtures/large_page.spg`（150 组件）— 解析+拓扑排序 debug 模式 < 2s


## 真实语料与 AI 验收（M9）

| 阶段 | 产物 | 职责 |
|------|------|------|
| M9-A | `tests/fixtures/corpus/manifest.json` | 原始语料元信息索引（1290+ 样本） |
| M9-B | `tests/fixtures/corpus/selection.json` | 精选 12 条代表样本进入仓库 |
| M9-C | `tests/fixtures/corpus/snapshots/` | 结构化 snapshot，捕获输出契约变化 |
| M9-D | `tests/fixtures/corpus/ai_eval/ai_eval_cases.json` | AI 问答评测集，验证模型能否基于 CLI 输出回答业务问题 | |

### M9-A 语料清单

- 机器可读清单：`tests/fixtures/corpus/manifest.json`
- 人类摘要与维护说明：`docs/corpus-manifest.md`
- 仅记录样本路径、大小、文件类型与启发式覆盖标签，不复制真实项目大文件
- 允许来源仅包含：
  - `/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`
  - `tests/fixtures/real_world_*.spg`
  - `tests/fixtures/test_project/**/*.spg|*.tbl`

### M9-B 精选语料

- 入选清单：`tests/fixtures/corpus/selection.json`
- 精选 12 条代表样本（5 copied + 7 referenced），覆盖复杂页面、只读页面、表单写入、弹窗、嵌套 visibility、DataFlow、链式物理表、刷新类 action、conditionExp、waitPrev、link_param、duplicate_action_id
- 说明文档：`tests/fixtures/corpus/samples/README.md`

### M9-C Snapshot 契约

- snapshot 目录：`tests/fixtures/corpus/snapshots/`
- 首批 5 个 snapshot case，覆盖 `--query-page-logic`、`--explain`、`--context`、`--query-dataflow`
- 只保存结构化摘要（schema_version、kind、summary 关键字段、diagnostics code、evidence claim、details count），不保存完整 raw JSON
- 用于捕获输出契约变化：任何合同字段变更都会触发 snapshot diff
- 更新机制：`UPDATE_CORPUS_SNAPSHOTS=1 cargo test --test corpus_snapshot_tests`

### M9-D AI 问答验收

### M9-F AI Eval 独立运行器与回答判分

- 结构化命令计划：`minimal_command_plan` 从字符串升级为 `command_kind` + `target` + `args` + `requires_project_dir`
- 回答判分元数据：`answer_assertions`（must_include / must_not_include / diagnostic_disclaimer_required / evidence_reference_required）
- 风险覆盖标签：`risk_tags` 覆盖 page_logic / explain / context / dataflow / lineage / condition / diagnostic
- 难度分级：`difficulty` = basic / intermediate / hard
- 运行约束：`max_command_count` ≤ 3，`allowed_output_sections` 控制模型可读字段
- 评测记录模板：`docs/ai-eval-run-template.md`
- 验证：`cargo test --test ai_eval_tests` 覆盖 23 项结构/语义/执行测试


## 技术栈
- **Rust 2024 Edition**
- **clap** — CLI 参数解析
- **serde_json** — JSON 序列化/反序列化
- **regex** — 表达式引用正则解析
- **petgraph** — 有向图数据结构
- **redb** — 嵌入式 KV 持久化
- **anyhow** — 错误处理

## 开发规范

- 所有代码注释使用中文
- 禁止 `#[allow(dead_code)]`，必须正确消费字段
- 使用 `assert_eq!` 进行测试断言
- 每次工作进度需 `git commit`
- 编译优化已配置：`opt-level = z`、`lto = true`、`strip = true`
