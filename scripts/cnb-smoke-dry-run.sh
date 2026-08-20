#!/usr/bin/env bash
# CNB kimi 冒烟 stage 的本地 dash 干跑。
#
# 为什么需要它：CNB stage 脚本由 sh 执行，镜像里 sh = dash。bash 下跑一遍
# 或 `bash -n` 都查不出 bash 专属语法——`IFS=$'\t'` 在 dash 下按字母 t 切分，
# 而退出码、记录数断言、token 检查会全部照常通过，是一次完整的静默假绿。
# 这个脚本因此坚持两件事：
#   1. stage 与 endStage 脚本从 .cnb.yml **逐字提取**，不手抄。手抄的副本会和
#      真身漂移，而漂移的方向恰好是「我以为我验过了」。
#   2. 用 dash 执行，不是 bash。
#
# 外部依赖用 stub 顶替（kimi / cargo / 网络都不真跑），只验 shell 语义与控制流。
# 本脚本自身是 bash（它是开发机工具，不进 CNB），被测的 stage 才是 dash。
#
# 用法：
#   scripts/cnb-smoke-dry-run.sh                 # 正常路径，期望退出 0、产出 6 条 record
#   scripts/cnb-smoke-dry-run.sh --fault empty   # 无 smoke.enabled，期望判红
#   scripts/cnb-smoke-dry-run.sh --fault dup     # smoke.order 撞号，期望判红
#   scripts/cnb-smoke-dry-run.sh --fault noeat   # 循环中途少产一条 record，期望判红
#   scripts/cnb-smoke-dry-run.sh --fault judgeleak # judge 输出 token，期望先脱敏再判红
set -euo pipefail

FAULT="none"
while [ $# -gt 0 ]; do
  case "$1" in
    --fault) FAULT="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
command -v dash >/dev/null || { echo "需要 dash：brew install dash / apt-get install dash" >&2; exit 2; }

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/cnb-smoke-dryrun.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT
STUB_BIN="$SANDBOX/bin"
mkdir -p "$STUB_BIN"

# --- stubs -------------------------------------------------------------
# macOS 的 date 没有 %3N，BSD 也没有 timeout；镜像里的 GNU coreutils 有。
# 这两个 stub 补的是 GNU 行为，不是改 stage 的写法。
cat > "$STUB_BIN/date" <<'STUB'
#!/usr/bin/env bash
if [ "${1:-}" = "+%s%3N" ]; then
  python3 -c 'import time;print(int(time.time()*1000))'
else
  exec /bin/date "$@"
fi
STUB

cat > "$STUB_BIN/timeout" <<'STUB'
#!/usr/bin/env bash
shift            # 丢掉时长；干跑不验超时行为
exec "$@"
STUB

# kimi stub：产出与真实 stream-json 同形的 transcript——assistant 行携带 tool_calls
# 数组（function.arguments 是 JSON 字符串），让 stage 的结构化解析真正跑起来。
# 每个问题都模拟一次 metadata-checker 调用；「入口」一问额外模拟绕过工具直接读
# 原始 .spg，覆盖 raw fallback 分类路径。
cat > "$STUB_BIN/kimi" <<'STUB'
#!/usr/bin/env bash
if [ "${1:-}" = "--version" ]; then
  echo "kimi-stub 0.0.0"
  exit 0
fi
QUESTION=""
while [ $# -gt 0 ]; do
  case "$1" in
    -p) QUESTION="$2"; shift 2 ;;
    *) shift ;;
  esac
done
printf '%s\n' '{"role":"assistant","content":"先查图状态","tool_calls":[{"type":"function","id":"call_1","function":{"name":"Bash","arguments":"{\"command\":\"metadata-checker --check-graph\"}"}}]}'
case "$QUESTION" in
  *入口*)
    printf '%s\n' '{"role":"assistant","content":"绕开工具直接读文件","tool_calls":[{"type":"function","id":"call_2","function":{"name":"Bash","arguments":"{\"command\":\"grep -n input3 合同协议.spg | head\"}"}}]}'
    ;;
esac
printf '%s\n' "{\"role\":\"assistant\",\"content\":\"stub answer for: ${QUESTION:0:60}\"}"
STUB

# metadata-checker stub：stage 用 command -v 定位二进制算 sha256，开发机上没有真身。
cat > "$STUB_BIN/metadata-checker" <<'STUB'
#!/usr/bin/env bash
echo '{"kind":"Stub"}'
STUB

# sha256sum stub：macOS 没有 GNU coreutils，用 shasum 顶；CNB 镜像里有真身。
cat > "$STUB_BIN/sha256sum" <<'STUB'
#!/usr/bin/env bash
exec shasum -a 256 "$@"
STUB

# cargo stub：judge 要 CNB_TOKEN 和真模型，干跑不碰。伪造它的产出契约即可。
cat > "$STUB_BIN/cargo" <<'STUB'
#!/usr/bin/env bash
OUT="${KIMI_JUDGE_OUTPUT_DIR:-target/kimi-harness-smoke}"
mkdir -p "$OUT"
echo '[]' > "$OUT/judge.json"
if [ "${DRY_RUN_JUDGE_LEAK:-0}" = "1" ]; then
  printf '# stub judge %s\n' "${CNB_TOKEN:-}" > "$OUT/judge.md"
  printf '[stub cargo] model echoed %s\n' "${CNB_TOKEN:-}"
else
  echo '# stub judge' > "$OUT/judge.md"
fi
echo "[stub cargo] $*"
STUB

# ln stub：stage 会往 /usr/local/bin 链接二进制，开发机上不该写那里。
cat > "$STUB_BIN/ln" <<'STUB'
#!/usr/bin/env bash
echo "[stub ln] $*"
STUB

# git stub：干跑工作区不是 git 仓库，而 run.json 的 repo_sha 与 endStage 的附件
# 归档都以 `git rev-parse HEAD` 为准。给一个确定的假 SHA，让归档路径真的跑起来。
cat > "$STUB_BIN/git" <<'STUB'
#!/usr/bin/env bash
if [ "${1:-}" = "rev-parse" ] && [ "${2:-}" = "HEAD" ]; then
  echo "0000000000000000000000000000000000000abc"
  exit 0
fi
exit 1
STUB

# curl stub：模拟 CNB commit 附件三步协议（换一次性链接 → PUT 原字节 → 确认）。
# 预签名链接里塞入 STUBPRESIGNSECRET，用来验证 stage 全程不回显它——真实
# upload_url 的 query 可能带凭证，回显一次就是一次泄漏。
# 每次换链接把请求体追加到 $DRY_RUN_ARCHIVE_LOG，供外层断言上传了哪些附件。
cat > "$STUB_BIN/curl" <<'STUB'
#!/usr/bin/env bash
OUT_FILE=""; URL=""; DATA=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) OUT_FILE="$2"; shift 2 ;;
    -w|-X|-H) shift 2 ;;
    -d) DATA="$2"; shift 2 ;;
    --data-binary) shift 2 ;;
    -sS|-s|-S|-N) shift ;;
    http*) URL="$1"; shift ;;
    *) shift ;;
  esac
done
case "$URL" in
  *asset-upload-url)
    printf '%s\n' "$DATA" >> "${DRY_RUN_ARCHIVE_LOG:-/dev/null}"
    [ -n "$OUT_FILE" ] && cat > "$OUT_FILE" <<JSON
{"upload_url":"https://upload.stub.invalid/put?sig=STUBPRESIGNSECRET","verify_url":"https://api.stub.invalid/verify?sig=STUBPRESIGNSECRET"}
JSON
    printf '200'
    ;;
  *upload.stub.invalid*|*verify*)
    [ -n "$OUT_FILE" ] && : > "$OUT_FILE"
    printf '200'
    ;;
  *)
    [ -n "$OUT_FILE" ] && : > "$OUT_FILE"
    printf '404'
    ;;
esac
exit 0
STUB

chmod +x "$STUB_BIN"/*

# --- 工作区 ------------------------------------------------------------
WORK="$SANDBOX/work"
mkdir -p "$WORK/target/real-project/succbi_project_container/projects/xiaoshouyi"
mkdir -p "$WORK/target/cnb/kimi-harness-build/release"
touch "$WORK/target/cnb/kimi-harness-build/release/metadata-checker"
cp "$REPO_ROOT/SKILL.md" "$WORK/SKILL.md"
mkdir -p "$WORK/tests/fixtures/corpus/ai_eval"
CASES="$WORK/tests/fixtures/corpus/ai_eval/xiaoshouyi_large_real_cases.json"
cp "$REPO_ROOT/tests/fixtures/corpus/ai_eval/xiaoshouyi_large_real_cases.json" "$CASES"

# 预置上一轮的过期 transcript：stage 开跑时必须清掉它，否则旧文件会混进本轮报告。
mkdir -p "$WORK/target/kimi-harness-smoke"
printf '%s\n' '{"role":"assistant","content":"stale"}' > "$WORK/target/kimi-harness-smoke/smoke-transcript-q1.jsonl"

# 故障注入：改输入数据或外部命令行为，不改 stage 脚本本身。
DRY_RUN_JUDGE_LEAK=0
case "$FAULT" in
  none) ;;
  empty)
    python3 - "$CASES" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p, encoding="utf-8"))
for c in d["cases"]:
    c.setdefault("smoke", {})["enabled"] = False
json.dump(d, open(p, "w", encoding="utf-8"), indent=2, ensure_ascii=False)
PY
    ;;
  dup)
    python3 - "$CASES" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p, encoding="utf-8"))
enabled = [c for c in d["cases"] if c.get("smoke", {}).get("enabled")]
enabled[1]["smoke"]["order"] = enabled[0]["smoke"]["order"]
json.dump(d, open(p, "w", encoding="utf-8"), indent=2, ensure_ascii=False)
PY
    ;;
  noeat)
    # 让某一问的 record 写不出来，验证记录数硬断言真的判红。
    # record 发射调用形如 `node -e '<script>' CASE_ID ORDER ...`，故 $4 是 order。
    # 让 order=3 那次静默不写行，模拟循环中途丢记录。
    REAL_NODE="$(command -v node)"
    cat > "$STUB_BIN/node" <<STUB
#!/usr/bin/env bash
if [ "\${1:-}" = "-e" ] && printf '%s' "\${2:-}" | grep -q "JSON.stringify({" && [ "\${4:-}" = "3" ]; then
  exit 0
fi
exec "$REAL_NODE" "\$@"
STUB
    chmod +x "$STUB_BIN/node"
    ;;
  judgeleak)
    # 模拟 judge 将模型控制的 token 同时写进产物和 stdout。stage 必须先捕获 stdout，
    # 扫描并脱敏所有产物，再打印日志并判红。
    DRY_RUN_JUDGE_LEAK=1
    ;;
  *) echo "unknown fault: $FAULT" >&2; exit 2 ;;
esac

# --- 逐字提取 stage 脚本 -----------------------------------------------
STAGE_SH="$SANDBOX/stage.sh"
ENDSTAGE_SH="$SANDBOX/endstage.sh"
python3 - "$REPO_ROOT/.cnb.yml" "$STAGE_SH" "$ENDSTAGE_SH" <<'PY'
import sys, yaml
cnb_path, out_path, endstage_path = sys.argv[1], sys.argv[2], sys.argv[3]
doc = yaml.safe_load(open(cnb_path, encoding="utf-8"))
pipeline = doc["$"]["api_trigger_kimi_harness_smoke"][0]
stages = [s for s in pipeline["stages"] if s.get("name") == "kimi-code harness smoke run"]
if len(stages) != 1:
    raise SystemExit(f"expected exactly 1 smoke stage, found {len(stages)}")
open(out_path, "w", encoding="utf-8").write(stages[0]["script"])
print(f"extracted stage script: {len(stages[0]['script'].splitlines())} lines")
endstages = [s for s in pipeline.get("endStages", []) if s.get("name") == "report smoke summary"]
if len(endstages) != 1:
    raise SystemExit(f"expected exactly 1 report smoke summary endStage, found {len(endstages)}")
open(endstage_path, "w", encoding="utf-8").write(endstages[0]["script"])
print(f"extracted endStage script: {len(endstages[0]['script'].splitlines())} lines")
PY

# --- 执行 ---------------------------------------------------------------
echo "=== running stage under dash (fault=$FAULT) ==="
STAGE_LOG="$SANDBOX/stage.log"
set +e
(
  cd "$WORK"
  PATH="$STUB_BIN:$PATH" \
  KIMI_CODE_HOME="$SANDBOX/kimi-home" \
  CNB_TOKEN="stub-token-not-a-real-secret" \
  CNB_REPO_SLUG="wu2305/metadata-checker" \
  DRY_RUN_JUDGE_LEAK="$DRY_RUN_JUDGE_LEAK" \
  dash "$STAGE_SH"
) > "$STAGE_LOG" 2>&1
STAGE_EXIT=$?
set -e
if grep -qF "stub-token-not-a-real-secret" "$STAGE_LOG"; then
  echo "FAIL: CNB_TOKEN 出现在 stage 日志中" >&2
  exit 1
fi
if [ -d "$WORK/target/kimi-harness-smoke" ] \
  && grep -rqF "stub-token-not-a-real-secret" "$WORK/target/kimi-harness-smoke"; then
  echo "FAIL: CNB_TOKEN 在 stage 结束后仍留在产物中" >&2
  exit 1
fi
cat "$STAGE_LOG"
echo "=== stage exit=$STAGE_EXIT ==="

# endStage 与 CNB 行为一致：无论 main stage 退出码如何都执行——失败路径的报告
# 发布（judge.md、records 投影、run manifest）正是它的职责。同样 dash + 逐字脚本。
echo "=== running endStage under dash ==="
ENDSTAGE_LOG="$SANDBOX/endstage.log"
ARCHIVE_LOG="$SANDBOX/archive-uploads.log"
: > "$ARCHIVE_LOG"
set +e
(
  cd "$WORK"
  PATH="$STUB_BIN:$PATH" \
  CNB_TOKEN="stub-token-not-a-real-secret" \
  CNB_REPO_SLUG="wu2305/metadata-checker" \
  DRY_RUN_ARCHIVE_LOG="$ARCHIVE_LOG" \
  dash "$ENDSTAGE_SH"
) > "$ENDSTAGE_LOG" 2>&1
ENDSTAGE_EXIT=$?
set -e
# 预签名链接绝不能进日志：真实 upload_url 的 query 可能带凭证。
if grep -qF "STUBPRESIGNSECRET" "$ENDSTAGE_LOG"; then
  echo "FAIL: 预签名上传链接出现在 endStage 日志中" >&2
  exit 1
fi
if grep -qF "stub-token-not-a-real-secret" "$ENDSTAGE_LOG"; then
  echo "FAIL: CNB_TOKEN 出现在 endStage 日志中" >&2
  exit 1
fi
cat "$ENDSTAGE_LOG"
echo "=== endStage exit=$ENDSTAGE_EXIT ==="

RECORDS="$WORK/target/kimi-harness-smoke/records.jsonl"
if [ -s "$RECORDS" ]; then
  echo "=== records.jsonl ($(wc -l < "$RECORDS" | tr -d ' ') lines) ==="
  cat "$RECORDS"
fi

if [ "$FAULT" = "none" ]; then
  # 结构化解析真实生效：每个问题恰好 1 次 mc 调用；只有「入口」一问 raw_fallback=true。
  grep -q '"metadata_checker_invocations":1' "$RECORDS" || { echo "FAIL: mc 结构化计数异常" >&2; exit 1; }
  [ "$(grep -c '"raw_fallback":true' "$RECORDS")" = "1" ] || { echo "FAIL: 期望恰好 1 条 raw_fallback=true" >&2; exit 1; }
  [ "$(grep -c '"raw_fallback":false' "$RECORDS")" = "5" ] || { echo "FAIL: 期望 5 条 raw_fallback=false" >&2; exit 1; }
  # 卫生清理：预置的过期 transcript 必须被 stage 清掉。
  [ ! -e "$WORK/target/kimi-harness-smoke/smoke-transcript-q1.jsonl" ] || { echo "FAIL: 过期 transcript 未被清理" >&2; exit 1; }
  # 文件名带 variant/trial：只按 case_id 命名时，Phase 2 的第二个 trial 会覆盖第一个的
  # 答案，而 records 照常写满、报告照常好看。这条断言守的是那次静默丢数据。
  TRIAL1_COUNT="$(find "$WORK/target/kimi-harness-smoke" -name 'smoke-transcript-*__baseline__t1.jsonl' | wc -l | tr -d ' ')"
  [ "$TRIAL1_COUNT" = "6" ] || { echo "FAIL: 期望 6 份带 __baseline__t1 的 transcript，实际 $TRIAL1_COUNT" >&2; exit 1; }
  find "$WORK/target/kimi-harness-smoke" -name 'smoke-stderr-*__baseline__t1.log' | grep -q . \
    || { echo "FAIL: stderr 文件名未带 variant/trial" >&2; exit 1; }
  # endStage 的 dump 用 RUN_TAG 作标签，标签里必须能看到身份。
  grep -q '__baseline__t1 transcript tail' "$ENDSTAGE_LOG" \
    || { echo "FAIL: endStage dump 标签未带 variant/trial" >&2; exit 1; }
  # run.json 身份清单已产出且含版本字段。
  grep -q '"corpus_sha"' "$WORK/target/kimi-harness-smoke/run.json" || { echo "FAIL: run.json 缺少 corpus_sha" >&2; exit 1; }
  grep -q '"kimi_version": "kimi-stub 0.0.0"' "$WORK/target/kimi-harness-smoke/run.json" || { echo "FAIL: run.json kimi_version 异常" >&2; exit 1; }
  # endStage 无条件发布 judge.md（stub 内容）与 run manifest。
  grep -q "===== judge.md =====" "$ENDSTAGE_LOG" || { echo "FAIL: endStage 未发布 judge.md" >&2; exit 1; }
  grep -q "# stub judge" "$ENDSTAGE_LOG" || { echo "FAIL: endStage judge.md 内容缺失" >&2; exit 1; }
  grep -q "===== run manifest =====" "$ENDSTAGE_LOG" || { echo "FAIL: endStage 未发布 run.json" >&2; exit 1; }
  # 附件归档：四件产物都要上传。stage 日志会被从头截断，判分结果只有落成附件
  # 才真正留得住——这条断言守的是「结果只活在聊天记录里」那个失效模式。
  for ASSET in records.jsonl run.json judge.md transcripts.tar.gz; do
    grep -q "archive uploaded: m58-smoke-.*-${ASSET} " "$ENDSTAGE_LOG" \
      || { echo "FAIL: 未归档附件 ${ASSET}" >&2; exit 1; }
  done
  [ "$(wc -l < "$ARCHIVE_LOG" | tr -d ' ')" = "4" ] \
    || { echo "FAIL: 期望 4 次换取上传链接，实际 $(wc -l < "$ARCHIVE_LOG" | tr -d ' ')" >&2; exit 1; }
  # 请求体必须带真实字节数：size=0 会让服务端签出一个永远传不满的链接。
  if grep -q '"size":0' "$ARCHIVE_LOG"; then echo "FAIL: 归档请求体 size=0" >&2; exit 1; fi
  # transcript 包按 90 天过期，结论类产物永久保留。
  grep -q '"ttl":90' "$ARCHIVE_LOG" || { echo "FAIL: transcript 包未设 ttl=90" >&2; exit 1; }
  [ "$(grep -c '"ttl":0' "$ARCHIVE_LOG")" = "3" ] || { echo "FAIL: 期望 3 件 ttl=0 永久产物" >&2; exit 1; }
  # 附件挂在 run.json 记录的同一个 commit 上，下游用同一个 SHA 就能取回。
  grep -q "archive commit=0000000000000000000000000000000000000abc" "$ENDSTAGE_LOG" \
    || { echo "FAIL: 归档 commit 与 run.json repo_sha 不同源" >&2; exit 1; }
  # tar 里必须是逐 case 脱敏之后的 transcript，且带 variant/trial 身份。
  tar -tzf "$WORK"/target/kimi-harness-smoke/m58-smoke-*-transcripts.tar.gz 2>/dev/null \
    | grep -q 'smoke-transcript-.*__baseline__t1\.jsonl' \
    || { echo "FAIL: transcript 包内容缺失或未带 variant/trial" >&2; exit 1; }
fi

# 期望：正常路径绿，注入的故障必须红。红不了才是这个脚本要抓的东西。
case "$FAULT" in
  none)   EXPECT_OK=1 ;;
  *)      EXPECT_OK=0 ;;
esac
if [ "$EXPECT_OK" = "1" ] && [ "$STAGE_EXIT" != "0" ]; then
  echo "FAIL: 正常路径本应退出 0，实际 $STAGE_EXIT" >&2; exit 1
fi
if [ "$EXPECT_OK" = "0" ] && [ "$STAGE_EXIT" = "0" ]; then
  echo "FAIL: fault=$FAULT 本应判红，实际退出 0（静默假绿）" >&2; exit 1
fi
if [ "$FAULT" = "judgeleak" ]; then
  # 失败路径同样交付报告：main stage 判红后，endStage 仍须发布脱敏后的 judge.md——
  # 判分依据不能随非零退出码一起埋掉。
  grep -q "===== judge.md =====" "$ENDSTAGE_LOG" || { echo "FAIL: 失败路径 endStage 未发布 judge.md" >&2; exit 1; }
  grep -q "stub judge" "$ENDSTAGE_LOG" || { echo "FAIL: 失败路径 judge.md 内容缺失" >&2; exit 1; }
fi
# 花括号是必需的：紧跟其后的全角括号会被 bash 当成变量名的一部分。
echo "OK: fault=$FAULT 行为符合预期（exit=${STAGE_EXIT}）"
