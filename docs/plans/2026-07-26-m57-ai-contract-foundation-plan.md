# M57 AI contract foundation 实施计划

> 状态：**approved**（2026-07-26）  
> Spec：[2026-07-23-m57-diff-refresh-closeout-design.md](../specs/2026-07-23-m57-diff-refresh-closeout-design.md)  
> Journal：[m57-diff-refresh-closeout.md](../milestones/performance/m57-diff-refresh-closeout.md)

## 目标

先为后续 Small Language Model（SLM）测评固定一条可消费、可回归的 M57 机器契约：差量刷新结果带稳定版本/类型与 `PersistReport`，one-shot 输出严格单行 JSON，stdio 结果与 one-shot 共享字段，登录失败保留脱敏后的服务端诊断。

这只是 M57 的 AI-facing 基础层。M58 仍负责空上下文 runner、AnswerJudge、ModelAdapter、RunReport 和模型基线，不在本计划中实现。

## 范围与非目标

### 本计划范围

- `DiffRefreshReport` 增加稳定的 `schema_version`、`kind` 和 `persist_report` 字段。
- 编排器完整透传 checkpoint-only、delta persist 的 `PersistReport`。
- stdio `diff_refresh.result` 和 one-shot `--session-diff-refresh` 复用同一报告字段。
- one-shot 成功/失败均输出单个 JSON 对象单行，保留现有稳定错误码。
- login 401/403 错误保留 JSON body 中的 `message` / `error.message`，并经过现有脱敏逻辑。
- TDD 覆盖正常、空刷新、错误和敏感信息边界。

### 明确不做

- M58 的 AnswerJudge、模型适配器、模型排行榜和 LLM 层 CI 阈值。
- M57 的页范围 auto/explore/声明、tick/backoff 调度和 LongLived 100 轮持久化策略；这些仍按 M57 Phase 1–3 清单推进。
- 修改 Rust/WASM 核心之外的第二套解析、图查询或输出实现。

## 文件边界

| 文件 | 责任 |
|---|---|
| `src/diff_refresh/orchestrator.rs` | 报告版本、类型与 `PersistReport` 透传 |
| `src/main.rs` | one-shot 结果序列化为单行并复用报告字段 |
| `src/stdio_server.rs` | stdio diff_refresh 结果契约保持一致 |
| `src/session/reqwest_provider.rs` | 401/403 body 诊断提取与脱敏 |
| `tests/m57_ai_contract_tests.rs` | 报告结构、单行输出、空刷新契约 |
| `tests/stdio_server_tests.rs` | bound stdio 成功结果字段 |
| `tests/session_reqwest_provider_tests.rs` | 登录错误 body 与敏感信息边界 |
| `tests/regression_tests.rs` | one-shot 成功/失败 envelope 与单行约束 |

## TDD 顺序

1. 先写失败测试：报告字段、stdio 字段、one-shot 单行输出、401 body message、密码/token 脱敏。
2. 运行影响面测试并记录红灯原因；不因红灯临时放宽断言。
3. 实现最小代码路径：复用现有 `PersistReport`、`sanitize_session_error_message` 和 stdio 序列化，不引入新 crate。
4. 运行同一批测试至绿，再补 `cargo fmt --check` 与 `cargo check --features cli-local`。
5. 复核 one-shot 与 stdio 字段集合，避免测试只验证某一个 adapter。

## 验收门

- [ ] `DiffRefreshReport` 序列化包含 `schema_version=1.0`、`kind=DiffRefresh`、`persist_report`。
- [ ] 非空 delta 刷新能看到真实 `PersistReport`；空 bootstrap 也能看到 checkpoint-only 的报告。
- [ ] stdio `diff_refresh` 成功结果与 one-shot 共享上述报告字段。
- [ ] one-shot stdout 恰好一个 JSON 对象行；稳定错误码不改变。
- [ ] 401/403 的服务端 message 可诊断但不包含 password、token、cookie、cipherPassport、Set-Cookie。
- [ ] 失败路径不推进 checkpoint、不吞掉原有错误分类。

## 后续衔接

完成本计划后，M58 才能以固定的 `SKILL.md + CLI/stdio + project path + question` 输入做空上下文 SLM 测试；M58 的 judge 只消费稳定 JSON，不反向依赖 redb 或内部 Rust 类型。
