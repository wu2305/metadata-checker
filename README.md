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
# human 可读模式（进入交互式查询）
./target/release/metadata-checker page.spg --human

# JSON 结构化输出（默认）
./target/release/metadata-checker page.spg --non-human

# 查询特定组件并带优先级分析
./target/release/metadata-checker page.spg --query input3 --priority
```

### 项目级图数据库

```bash
# 扫描项目目录并构建/更新图数据库
./target/release/metadata-checker --project-dir /path/to/project --build-graph

# 查询模型的读写关系
./target/release/metadata-checker --query-model model1

# 查询页面的跨文件关系
./target/release/metadata-checker --query-page "page/合同管理/销售合同"

# 查询两个页面之间的关系
./target/release/metadata-checker --query-cross "page/A" "page/B"

# 展开 DataFlow 子图
./target/release/metadata-checker --query-dataflow flow.tbl
```

### human 模式交互查询

当仅使用 `--human` 参数（不带 `--query`）时，工具会列出所有组件 ID，提示用户输入要查询的组件 ID：

```
=== 交互式查询 ===
请输入要查询的组件 ID（直接回车退出）: input3
```

## CLI 参数

```
Usage: metadata-checker [OPTIONS] [FILE]

Arguments:
  [FILE]                    页面元数据 JSON 文件路径

Options:
      --human               人类可读输出（交互模式）
      --non-human           机器友好的 JSON 输出（默认）
      --query <ID>          查询特定组件 ID 的详细信息
      --priority            附加计算优先级分析
      --project-dir <DIR>   项目目录（用于跨文件分析）
      --build-graph         从项目目录构建/更新图数据库
      --query-model <MODEL> 查询模型的读写关系
      --query-page <PAGE>   查询页面的依赖关系
      --query-cross <A> <B> 查询两页面间的跨文件关系
      --query-dataflow <M>  展开 DataFlow 模型的内部子图
  -h, --help                打印帮助信息
  -V, --version             打印版本
```

## 项目结构

```
src/
├── main.rs        # CLI 入口与子命令分发
├── cli.rs         # 命令行参数定义（clap）
├── lib.rs         # 库入口
├── superpage.rs   # .spg JSON 解析、组件树提取、表达式引用解析
├── dependency.rs  # 组件依赖图、拓扑排序、循环检测、值追溯
├── priority.rs    # defaultValue/exp/calcCondition 优先级分析
├── output.rs      # human/JSON 双模式输出、交互式查询
├── parser.rs      # 文件类型识别、统一元数据结构
├── graph.rs       # petgraph 有向图 + redb 持久化
├── scanner.rs     # 项目目录扫描、增量更新、SPG/TBL 处理
└── query.rs       # 图查询接口（model/page/cross/dataflow）

tests/             # 测试用例（115 个测试覆盖全部模块）
```

## 图数据库

图数据库存储在 `/tmp/metadata-checker.graphdb`（可通过 `--project-dir` 配置）。

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

## 测试

```bash
cargo test
```

115 个测试覆盖：
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
