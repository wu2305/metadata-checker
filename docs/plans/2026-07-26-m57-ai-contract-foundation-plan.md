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
- M57 的页范围 auto/explore/声明和 LongLived 100 轮持久化策略；这些仍按 M57 Phase 1–3 清单推进。tick/backoff 本轮作为已批准计划的下一执行切片落地。
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

- [x] `DiffRefreshReport` 序列化包含 `schema_version=1.0`、`kind=DiffRefresh`、`persist_report`。
- [x] 非空 delta 刷新能看到真实 `PersistReport`；空 bootstrap 也能看到 checkpoint-only 的报告。
- [x] stdio `diff_refresh` 成功结果与 one-shot 共享上述报告字段。
- [x] one-shot stdout 恰好一个 JSON 对象行；稳定错误码不改变。
- [x] 401/403 的服务端 message 可诊断但不包含 password、token、cookie、cipherPassport、Set-Cookie。
- [x] 失败路径不推进 checkpoint、不吞掉原有错误分类。

实现提交：`c9302fd`、`145e838`。TDD 提交：`d452e85`。

## 批准的后续切片：M57-1 tick loop

状态：**done**（实现 `8249d6f`；error-chain 修复 `2ec96a8`；TDD `7763b20`）

- 复用现有 `BackoffSchedule`，新增同步 tick runner，驱动 `DiffRefreshOrchestrator::refresh_once`。
- 网络/临时传输错误才退避；数据格式错误、鉴权错误和未分类错误立即返回，不静默重试。
- sleep 通过 hook 注入测试，生产入口使用标准线程 sleep；不引入 daemon、多项目并行或第二套刷新栈。
- runner 输出稳定统计：尝试次数、成功次数、可重试失败次数、实际安排的退避秒数、成功报告和是否达到尝试上限。
- TDD 覆盖网络失败后恢复、数据错误不退避、尝试上限和零尝试边界。

## 批准的后续切片：M57-2 RefreshScope declaration

状态：**done**（实现 `e980b37`；JSON 稳定性修复 `dd231af`；TDD `387871b`）

- 显式 module/source/file 过滤必须在报告中声明为 `applied=true`。
- 多个显式条件必须声明 `compound`，不能只保留一个条件。
- `current_source_path` 当前仅是排序提示，报告必须标记 `auto` 但未应用，并给出回落原因。
- 无显式条件时明确回落 `project`，不得静默声称已做局部刷新。

## 批准的后续切片：M57-3 runtime auth error mapping

状态：**done**（TDD `55e5339`；实现 `6bb8364`）

- 401/403 在 refresh error chain 中统一映射为 `SESSION_AUTH_REQUIRED`。
- 保留脱敏后的服务端诊断，不输出 password、token、cookie、cipherPassport 或 Set-Cookie。
- 普通数据错误和未分类错误继续返回 `DIFF_REFRESH_FAILED`，不扩大鉴权分类。
- 默认不静默重登；调用方重新启动/绑定 session 即为 rebind 路径。
- `m57_auth_flow_tests` 2 passed；stdio auth failure 专项与 stdio 全套回归通过。

## 批准的后续切片：M57-4 DiffRefresh scope contract

状态：**done**（TDD `f0b03f1`；实现 `c3f9fda`）

- `DiffRefreshReport` 是 one-shot、stdio、tick 的共同结果，必须直接带 `RefreshScope`。
- 无显式 scope 时统一声明 `project/fallback/applied=false`，不得让 SLM runner 将全项目刷新误读成局部刷新。
- scope 枚举继续复用 M57-2 的 snake_case JSON 名称，不引入第二套类型或解析栈。
- scope contract 2 passed；AI contract、tick、stdio 和编排器回归通过。

## 批准的后续切片：M57-5 Phase 2 incremental read model

状态：**done**（TDD `4eacb64`；实现 `c80e95c`）

- `GraphRuntime::prepare_replacement` 使用 `dirty ∪ deleted` 选择派生 read-model 更新路径。
- 稳定 node set 优先增量更新 Dense/Facts/PageDep；新增/删除导致无法安全局部修补时必须显式回落 full rebuild，不得伪报增量。
- TDD 固定 full rebuild 等价性，以及 1/10/100 dirty scale curve 的可观测输出。
- `m57_incremental_read_model_tests` 覆盖稳定 node set 等价、删除回退和 1/10/100 曲线，共 3 passed；fixture 样本 full `7 ms`，增量 read-model `0/2/5 ms`，复跑为 full `17 ms`、增量 `2/6/17 ms`，仅作可观测性样本。
- 真实项目 release 基线已通过：`78127 nodes / 150164 edges`，cold full `249906 ms`；实际 node-name mutation 下 dirty `1/10/100` 增量分别 `1653/2347/3052 ms`，约为 `151.2x/106.5x/81.9x` 加速，三档均保持 `Incremental`。

## 后续衔接

完成本计划后，M58 才能以固定的 `SKILL.md + CLI/stdio + project path + question` 输入做空上下文 SLM 测试；M58 的 judge 只消费稳定 JSON，不反向依赖 redb 或内部 Rust 类型。
