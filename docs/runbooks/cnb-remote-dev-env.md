# CNB 云原生开发环境：开启、连接与快速迭代

本文记录如何拉起 CNB 云原生开发环境（cloud-native dev env）、连上去跑测试，以及在上面
快速迭代的操作流程。目的是把本地笔记本从「跑 CI」这件事里解放出来：编辑、提交、推送留在
本地，编译和测试放到远端。

环境定义在 `.cnb.yml` 的 `.cloud_native_dev_env` 锚点（`vscode` 事件），与跑 M58 评测的
`.m58_cnb_llm_ci`（`api_trigger_m58_llm` 事件）是两条独立流水线，用途不同，见第 6 节。

## 1. 开启环境

云原生开发环境**不能**用 `POST /-/build/start` 拉起。该接口只接受 `api_trigger` 或
`api_trigger_` 前缀的事件，传 `vscode` 会被拒：

```
{"errcode":400,"errmsg":"[API_BUILD_START_FAIL]Event must be 'api_trigger' or start with 'api_trigger_'."}
```

开发环境走的是另一个接口 `StartWorkspace`，`cnb` CLI 已经封装：

```bash
cnb workspace start-workspace \
  --repo wu2305/metadata-checker \
  --branch codex/m58-slm-eval-foundation
```

返回里的 `sn`（形如 `cnb-ni8-1jv4a7j9k`）是后续所有操作的句柄。

**接口只有 `repo` / `branch` / `ref` 三个入参，没有 `env` 字段。** 也就是说开发环境无法在
启动时注入环境变量——像 `M58_CNB_REASONING_EFFORT` 这种参数只能连上去之后在 shell 里
`export`。需要用环境变量参数化的跑法，用第 6 节的 API 触发流水线。

同一分支重复调用不会重复创建：已存在就直接返回原环境。

## 2. 取 SSH 地址

```bash
cnb workspace get-workspace-detail \
  --repo wu2305/metadata-checker \
  --sn cnb-ni8-1jv4a7j9k
```

环境就绪前会返回 `404 [WORKSPACE_NOT_FOUND]workspace not found.`，**这是正常的启动中状态，
不是错误**，隔 20 秒重试即可，实测第二次（约 20~40 秒）就能拿到。

就绪后返回里有多种接入方式，命令行只需要 `remoteSsh`：

```
remoteSsh: cnb-<sn>-001.<uuid>-adv@cnb.space
ssh: ssh cnb-<sn>-001.<uuid>-adv@cnb.space
```

其余字段是 IDE 的一键跳转协议（`vscode:` / `cursor:` / `webide` 等），本地用 VSCode
Remote-SSH 直连时可以直接点 `vscode` 那一条。

免密登录已由 CNB 侧配置好，不需要额外传密钥；第一次连接加 `-o StrictHostKeyChecking=no`
省掉指纹确认。

> `cnb workspace list-workspaces` 在 cnb-cli 1.9.7 下是坏的：不执行请求，直接把约 65 KB
> 的压缩打包代码吐到终端（和 `cnb build start-build --data` 的 `tryReadFileRef is not defined`
> 是同一类问题）。要列环境请走网页端。`start-workspace` 与 `get-workspace-detail` 正常。

## 3. 环境里有什么

- 仓库在 `/workspace`，已经 checkout 到启动时指定的分支，`origin` 指向 CNB 仓库，
  凭据已配好，可以直接 `git fetch` / `git pull`。
- 工具链来自 CI 镜像（`.cnb/images/browser-wasm-ci.Dockerfile`）：rust 1.95.0、
  `cargo-llvm-cov`、`wasm-bindgen`、`bencher`、node。
- `.cnb.yml` 里声明为 volume 的目录跨环境重启保留，因此依赖缓存是热的：
  `/usr/local/cargo/registry`、`/usr/local/cargo/git`、`./target/cnb/workspace`。
- 开发环境的 `CARGO_TARGET_DIR` 是 `target/cnb/workspace`；分支 CI 用的是
  `target/cnb/coverage`。要完全复刻 CI 的产物布局就显式覆盖（见第 5 节）。

## 4. 必须用 login shell（最容易踩的坑）

`ssh host '<命令>'` 起的是**非** login shell，不会 source `/etc/profile`，于是
`CARGO_HOME` / `RUSTUP_HOME` 都没设置，rustup 找不到工具链：

```
error: rustup could not choose a version of rustc to run, because one wasn't
specified explicitly, and no default is configured.
```

镜像里只装了一个工具链，rustup 却因为环境变量缺失而无法解析它。解法是走 login shell：

```bash
ssh $HOST 'bash -lc "cd /workspace && cargo test --lib"'
# 或者在脚本内部先 source
ssh $HOST 'cd /workspace && . /etc/profile && cargo test --lib'
# 跑脚本文件时用 bash -l
ssh $HOST 'cd /workspace && setsid nohup bash -l ./remote_ci.sh > ci.log 2>&1 &'
```

> **`/etc/profile` 里有密钥**（`ACC_PRODUCT_CONFIG_V2` 内嵌鉴权 token、`TWINE_PASSWORD`、
> `CNB_TOKEN`）。source 它没问题，但**绝对不要** `cat /etc/profile`、`env`、`export -p`
> 或把整个环境变量打进日志——这些内容会直接进对话记录和 CI 日志。需要确认某个变量存在时，
> 用 `grep -o` 只取变量名，或 `test -n "$VAR" && echo set`。

## 5. 跑长任务：脱离 SSH 会话

编译加全量测试要几十分钟，不能挂在 SSH 前台——断线就全废。用 `setsid nohup` 脱离会话，
输出重定向到文件，然后轮询日志：

```bash
HOST=cnb-<sn>-001.<uuid>-adv@cnb.space

# 1) 把脚本传上去
scp remote_ci.sh $HOST:/workspace/remote_ci.sh

# 2) 脱离会话启动
ssh $HOST 'cd /workspace && rm -f ci.log && \
  setsid nohup bash -l ./remote_ci.sh > ci.log 2>&1 < /dev/null &'

# 3) 轮询（一次连接看进度 + 存活）
ssh $HOST 'cd /workspace && tail -3 ci.log; \
  pgrep -f remote_ci.sh >/dev/null && echo RUNNING || echo STOPPED'
```

三个重定向（`> ci.log`、`2>&1`、`< /dev/null`）都要带上，避免子进程继续持有 SSH 的
标准流。即便如此，启动用的那条 `ssh` 有时仍然不会立刻返回（实测出现过挂满 2 分钟才退出，
而任务本身早已在跑）。因此**把启动当成「发射后不管」**：给它加超时，退出码不作为判据，
任务是否起来一律由下一步的轮询确认。

复刻分支 CI（`.rust_ci_branch_push`）的脚本内容：

```bash
#!/bin/bash
set -euo pipefail
cd /workspace
export CARGO_TARGET_DIR=target/cnb/coverage
cargo fmt --check
cargo check --benches
rm -rf target/debug
mkdir -p target target/cnb/coverage/llvm-cov-target
ln -sfn cnb/coverage/llvm-cov-target/debug target/debug
cargo llvm-cov test --workspace --lcov --output-path lcov.info -- --test-threads=1
test -s lcov.info
cargo check --no-default-features --features browser-wasm --target wasm32-unknown-unknown
echo "REMOTE_CI_OK"
```

结尾的哨兵 `REMOTE_CI_OK` 配合 `set -e`，让「跑完了」和「中途挂了」在日志里一眼可分。

## 6. 快速迭代循环

**编辑和提交留在本地**，远端只负责编译和跑测试。一轮迭代：

```bash
# 本地
git push cnb HEAD

# 远端
ssh $HOST 'bash -lc "cd /workspace && git pull --ff-only && \
  cargo test --features cli-local --test m58_cnb_ai_runner_tests"'
```

提速要点：

- **只跑目标 test target**：`cargo test --test <name>` 比 `--workspace` 快一个数量级。
  全量 `llvm-cov` 留到推 PR 前跑一次。
- **别在迭代循环里用 `cargo llvm-cov`**：插桩会让整个 target 目录失效重编。
- **保持 `CARGO_TARGET_DIR` 稳定**：在 `target/cnb/workspace`（volume）和
  `target/cnb/coverage` 之间来回切会让增量缓存互相作废。迭代期固定用默认的
  `target/cnb/workspace`，只在复刻 CI 时才切。
- **一次 SSH 干完一串事**：每次 `ssh` 都要重新握手，把 `git pull && cargo test` 串在
  一条命令里比分两次快。
- 需要交互式排查时直接开会话：`ssh $HOST`，或用 VSCode Remote-SSH 连
  `get-workspace-detail` 给出的 `vscode:` 地址，在 `/workspace` 里直接编辑。注意这样改的
  代码只在远端，**必须记得 commit + push 回来**，否则环境回收即丢失。

## 7. 心跳与回收

`.cnb.yml` 里 vscode 服务设了 `keepAliveTimeout: 1h`（默认只有 10 分钟）：远程 SSH 下两次
操作之间很容易超过 10 分钟，放宽到 1 小时才不会做着做着环境被回收。即便如此，长时间无
http/ssh 心跳仍会关闭环境——挂长任务时保持轮询，轮询本身就是心跳。

手动收尾：

```bash
cnb workspace workspace-stop    --repo <repo> --sn <sn>   # 停止
cnb workspace delete-workspace  --repo <repo> --sn <sn>   # 删除
```

## 8. 什么时候不该用开发环境

开发环境适合**交互式排查**和**快速重跑目标测试**。以下情况仍然走 API 触发的流水线：

```bash
curl -s -X POST "https://api.cnb.cool/<repo>/-/build/start" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"branch":"<branch>","sha":"<40 位完整 sha>","event":"api_trigger_m58_llm",
       "env":{"M58_CNB_REASONING_EFFORT":"high"}}'
```

理由有两条：

1. **`StartWorkspace` 没有 `env` 字段**（第 1 节），而 M58 评测的关键维度就是靠构建环境
   变量注入的；流水线的 `env` 是一个普通 string map，能传。
2. **手敲 shell 的结果不可复现**。流水线固定了工具链、`M58_AI_EVAL_TRIALS=3`、输出目录，
   报告可以完全从 CI 日志重建；开发环境里的一次手跑做不到这一点，不能当作评测基线。

一句话：开发环境用来**让测试变绿**，流水线用来**产出可信的结论**。
