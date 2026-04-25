# AGENTS.md — metadata-checker

> 本文件指导其他 AI Agent 在此仓库中工作。所有 Agent 必须遵守以下规范。

## 项目简介

`metadata-checker` 是一个 Rust CLI 工具，用于解析低代码平台的 SuperPage（`.spg`）和 Table（`.tbl`）元数据文件。核心能力包括组件树提取、表达式解析、依赖图构建、值来源追溯、计算优先级分析和跨文件图数据库查询。

## 技术约束

- **语言**：Rust 2024 Edition，禁止引入其他语言
- **二进制大小**：Release 版本控制在 10MB 以内（当前约 1.6MB）
- **跨平台**：需支持 `windows_amd64`、`macos_arm64`、`linux_amd64`、`linux_arm64`
- **依赖管理**：新增依赖须使用 Cargo，优先选择轻量级 crate
- **禁止**：
  - `#[allow(dead_code)]` — 必须正确消费字段或删除
  - 深拷贝优化不当时不做 — 能引用则引用，能 move 则 move
  - 使用 `panic!` 处理可恢复错误 — 使用 `anyhow::Result`

## 代码规范

### 注释
- 所有模块级、类型级、函数级注释使用**中文**
- 文档注释（`///`）必须放在 `#[derive(...)]` 之前
- 复杂算法需添加行内注释说明意图

### 命名
- 函数/变量：`snake_case`
- 类型/枚举：`PascalCase`
- 常量：`SCREAMING_SNAKE_CASE`
- 避免单字母变量名（循环索引除外）

### 测试
- 使用 `assert_eq!` 进行断言
- 测试用例必须说明测试目的（通过函数名或注释）
- 新增功能必须配套测试，覆盖正常和边界场景
- 真实场景测试数据放在 `tests/fixtures/`

### 错误处理
- 使用 `anyhow::Result` 作为返回类型
- 使用 `.with_context()` 添加上下文信息
- 不要在库代码中直接 `println!`/`eprintln!`，由调用方决定输出方式

## 模块职责

| 模块 | 职责 | 修改前必读 |
|---|---|---|
| `superpage.rs` | JSON 反序列化、组件树递归提取、表达式引用正则解析 | 理解 `RawComponent` 和 `SpgComponent` 的映射关系 |
| `dependency.rs` | 依赖图构建、拓扑排序、循环检测、值来源递归追溯 | 了解 `RefType` 分类和 `ValueTrace` 结构 |
| `priority.rs` | `defaultValue`/`exp`/`calcCondition` 优先级判定 | 优先级规则：`calcCondition` > `exp` > `defaultValue` |
| `output.rs` | human/JSON 双模式输出、交互式查询循环 | 修改输出格式需同步更新两种模式 |
| `parser.rs` | 文件类型识别、统一元数据入口 | 新增文件类型需在此注册 |
| `graph.rs` | petgraph 内存图 + redb KV 持久化 | 节点/边类型变更需同步序列化逻辑 |
| `scanner.rs` | 目录扫描、增量更新（mtime+size+hash）、SPG/TBL 处理 | 增量逻辑涉及文件状态比较，改动需谨慎 |
| `query.rs` | 图查询接口：`query_model`/`query_page`/`query_cross`/`query_dataflow` | DataFlow 子图展开涉及字段级追溯，较复杂 |
| `cli.rs` | clap 参数定义 | 新增 CLI 参数需同步更新 help 文本 |
| `main.rs` | 子命令分发 | 新增子命令需在此注册并调用对应模块 |

## 常见修改场景

### 新增表达式字段
1. 在 `superpage.rs` 的 `always_expr_fields` 或 `conditional_expr_fields` 中添加字段名
2. 在 `RawComponent` 中添加对应的反序列化字段（如需要）
3. 在 `extract_components` 中处理该字段
4. 补充测试用例到 `tests/superpage_tests.rs` 和 `tests/core_feature_tests.rs`

### 新增跨文件关系类型
1. 在 `graph.rs` 的 `EdgeType` 枚举中添加新边类型
2. 在 `scanner.rs` 中识别并创建对应的边
3. 在 `query.rs` 中处理新边类型的查询输出
4. 更新 `tests/scanner_tests.rs`

### 新增 CLI 子命令/参数
1. 在 `cli.rs` 的 `Commands` 枚举或 `Cli` 结构体中添加
2. 在 `main.rs` 中匹配并调用处理函数
3. 更新 `README.md` 和 `--help` 输出

### 优化性能（减少 clone）
1. 先 `cargo check` 确认编译通过
2. 识别 `.clone()` 热点（`grep -n '\.clone()' src/*.rs`）
3. 优先尝试：返回引用（`&T`）、使用 `Cow<str>`、预分配局部变量复用
4. 修改后 `cargo test` 确保全部通过
5. `git diff` 确认变更范围合理

## 提交规范

- **每次工作进度必须 `git commit`**
- 提交信息格式：`type: description`
- 常用 type：
  - `feat:` 新功能
  - `fix:` 修复
  - `perf:` 性能优化
  - `test:` 测试补充
  - `docs:` 文档/注释
  - `refactor:` 重构

## 测试命令

```bash
# 快速检查
cargo check

# 运行全部测试
cargo test

# Release 编译（验证二进制大小）
cargo build --release
ls -lh target/release/metadata-checker

# 在真实项目上测试
./target/release/metadata-checker --project-dir /path/to/project --build-graph
```

## 外部参考

- 低代码平台代码仓库：`/Users/wuhaocheng/Downloads/bi`
- 真实测试项目：`/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`
- SKILL.md：`/Users/wuhaocheng/.codex/skills/metadata-checker/SKILL.md`

## 常见问题

**Q: 编译出现 dead_code 警告怎么办？**  
A: 不要加 `#[allow(dead_code)]`。要么使用这个字段，要么删除它。

**Q: 需要新增外部依赖怎么办？**  
A: 编辑 `Cargo.toml`，优先选择下载量高、体积小的 crate。添加后需评估对二进制大小的影响。

**Q: 修改了输出格式但测试失败？**  
A: `output.rs` 的 human 和 JSON 模式需同步更新。检查 `tests/output_parser_tests.rs` 和 `tests/core_feature_tests.rs`。

**Q: 如何验证图数据库增量更新？**  
A: 先 `--build-graph`，修改某个 fixture 文件，再次 `--build-graph`，检查日志中是否只处理变更文件。
