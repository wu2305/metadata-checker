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

### 项目级图数据库

```bash
# 扫描项目目录并构建/更新图数据库
./target/release/metadata-checker --project-dir /path/to/project --build-graph

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
- `writes`：action 写入的模型、参数
- `triggered_by`：父页面（Contains）、被引用组件（incoming Reads）
- `affects`：触发的 action（Triggers）、导航目标（OpensPage）

### 解释模型

```bash
metadata-checker --project-dir ./my-project --explain model:model1
```

输出摘要：
```json
{
  "what_is_it": "数据模型 model1，被 2 个组件/动作读取，被 6 个组件/动作写入，参与 0 个 DataFlow",
  "type": "model",
  "importance": "write_target",
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
  "importance": "navigation",
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
- 禁止默认读取 raw JSON，必须从 `summary` 开始

详细字段定义见 `docs/schema.md`。

## 测试

```bash
cargo test
```

115+ 个测试覆盖：
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
