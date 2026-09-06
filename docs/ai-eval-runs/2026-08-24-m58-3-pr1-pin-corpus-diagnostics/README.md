# M58.3 PR1 pin 语料诊断实测（验收 1 登记证据）

- 日期：2026-08-24
- 语料：`https://cnb.cool/wu2305/succbi_project_container.git` branch `xiaoshouyi-corpus`
  @ `6920ac514df2d79a3df73a6447cf851abdfc954b`，项目根 `projects/xiaoshouyi`
- 代码：本仓库 `codex/m58-slm-eval-foundation`，CNB 远端 debug 构建
  （`--features cli-local`，`target/cnb/workspace/debug/metadata-checker`）

## 重放命令

```bash
metadata-checker --non-human \
  --project-dir target/real-project-src/projects/xiaoshouyi \
  --graph-db-path /tmp/m58-3-pr1-pin.graphdb \
  --build-graph
```

## 结果（build-output.txt 为原始 stdout）

- 统计行：`Indexed 1329 files | Unchanged: 0 | Dirty: 78127 | Deleted: 0 | Nodes: 78127 | Edges: 150165`
  （节点/边数与 M58.2 验证 graphdb 一致）
- `GRAPH_DB_*` 五类 hydrate 诊断：均为 0（正常路径无丢行）。
- `SCANNER_UNRECOGNIZED_CONTAINER_KEY` = 943，逐键分布：
  `moreFields` 279、`columns` 250、`panel` 241、`tabs` 60、`attrFields` 38、
  `params` 33、`grid` 22、`operateButtons` 20。
  其中 `columns`/`tabs`/`panel`（数组形态）是 F1 已诊断的真实组件容器，
  该计数是 PR1 安全网按设计捕获「白名单丢组件」缺口，PR2 形态感知递归落地后应显著下降。
- `SCANNER_DUPLICATE_COMPONENT_ID` = 218：真实语料跨容器复制产生的重复组件 id，数据实情信号。

逐键分布重放（与扫描器同口径：排除 `actions`/`effectStyles`/`conditionStyles`/`labelFields`/`stateFields`，
跳过 `components`/`panels`/`steps`/`comps`/`id`/`type`，统计其余非空且含对象元素的数组键）：

```bash
python3 - <<'EOF'
import json, glob, collections
EXCLUDED = {"actions", "effectStyles", "conditionStyles", "labelFields", "stateFields"}
KNOWN = {"components", "panels", "steps", "comps", "id", "type"}
counter = collections.Counter()
def walk(node):
    if not isinstance(node, dict):
        return
    for key, value in node.items():
        if key in KNOWN or key in EXCLUDED:
            continue
        if isinstance(value, list) and value and any(isinstance(i, dict) for i in value):
            counter[key] += 1
    for key in ("components", "panels", "steps", "comps"):
        children = node.get(key)
        if isinstance(children, list):
            for child in children:
                walk(child)
for f in glob.glob("target/real-project-src/projects/xiaoshouyi/**/*.spg", recursive=True):
    try:
        data = json.load(open(f))
    except Exception:
        continue
    canvas = data.get("canvas")
    if canvas:
        walk(canvas)
print(sum(counter.values()), counter.most_common(15))
EOF
```
