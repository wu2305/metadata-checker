#!/usr/bin/env python3
"""M58.3 附录 A 语料形态体检（PR2 后重跑口径）。

按 spec 的 F1 形态规则测量 `.spg` 语料，产出三类可复核数字：

1. **容器形态**：PR2 形态感知递归（白名单四键 + extra 形态吻合键，排除列表
   优先）漏掉哪些候选容器键，逐键给出行为证据（带表达式 / 带 actions /
   带子容器），并输出 spec 验收 2 的判据「漏掉候选 − 排除列表命中数」。
2. **表达式引用**：沿真实调用链
   `classify_identifier → resolve_expression_refs_with_context → scanner match`
   统计各终态，含 `ComponentProperty` 建 comp→comp DependsOn 边的量与垃圾
   model 名。
3. **DataFlow 深度**：`.spg` 内联 dataFlow 与 `.tbl` 解析深度差的原始结构计数。

**测量边界（必须与判读一起引用）**：本脚本读原始 JSON、复刻解析器的遍历与分类规则，
不构建 graphdb。它给出的是**候选量**，不是图中实际节点/边数——凡需断言图内事实
（如「垃圾 model 节点数」），须在 pin 语料重建后从图里复测。

用法：
    python3 tools/corpus-shape-audit.py --corpus <dir> [--json out.json]
"""

import argparse
import collections
import hashlib
import json
import os
import re
import subprocess
import sys

# 与 superpage/mod.rs:415-457 的两张分类表同源；改那里须同步改这里。
ALWAYS_EXPR_FIELDS = {
    "exp", "itemFilter", "visibleCondition", "validExp", "calcCondition",
    "calcExp", "maskCondition", "submitCondition", "submitPageCondition",
    "defaultPanelCondition", "disableCondition", "defaultValueExp",
}
CONDITIONAL_EXPR_FIELDS = {
    "value", "defaultValue", "visible", "enable", "text", "formula", "html",
    "desc", "placeholder", "url", "documentTitle", "inputTitle", "labelValue",
    "panelName", "rootPath", "selectedCaption", "confirmCaption", "tip",
    "badge", "count", "attrCaption", "caption", "defaultSelect", "defaultCheck",
    "maxLevel",
}
# superpage/mod.rs:349-364 的白名单子容器键（RawComponent 声明字段，Vec<RawComponent> 链）。
WHITELIST_CONTAINER_KEYS = ("components", "panels", "steps", "comps")
# superpage/mod.rs:318-324 的已知非组件容器键排除列表（M58.3 F1，附录 A 实测初值）；
# 排除优先于形态判定。改那里须同步改这里。
NON_COMPONENT_CONTAINER_KEYS = {
    "actions", "effectStyles", "conditionStyles", "labelFields", "stateFields",
}
# expr_ast.rs:820 的 ${} 分支只做 split('.')，纯点分路径才是它的良性输入。
DOTTED_PATH = re.compile(r"^[A-Za-z_$@][\w$@]*(\.[\w$@]+)*$")


def is_component_object(node):
    """spec F1 形态规则的元素判据：带非空字符串 id 与非空字符串 type 的对象。"""
    return (
        isinstance(node, dict)
        and isinstance(node.get("id"), str) and node["id"]
        and isinstance(node.get("type"), str) and node["type"]
    )


def is_component_array_lenient(value):
    """复刻 superpage/mod.rs:334-347 的 is_component_array（递归遍历口径）。

    非空数组且每个元素都是带字符串 id 与字符串 type 的对象——**允许空串**，
    与 serde 的 `String` 反序列化对齐（空串 id/type 的元素在 extract_components
    里走透传分支，不append但继续递归子树）。候选审计仍用上面的严格判据。
    """
    return (
        isinstance(value, list)
        and len(value) > 0
        and all(
            isinstance(item, dict)
            and isinstance(item.get("id"), str)
            and isinstance(item.get("type"), str)
            for item in value
        )
    )


def is_component_array(value):
    """spec F1 形态规则（严格口径）：数组、非空、且**每个**元素都满足元素判据。

    混合形态（部分元素缺 id/type）整体判非组件，对应 spec 的
    `SCANNER_UNRECOGNIZED_CONTAINER_KEY` 计数分支。
    """
    return (
        isinstance(value, list)
        and len(value) > 0
        and all(is_component_object(item) for item in value)
    )


def would_fail_raw_component_deserialize(node):
    """廉价复刻 serde 反序列化 RawComponent 会失败的形态。

    覆盖三类失败：声明为 Vec 的字段不是数组；标量字段类型不符
    （`submitField` 须为字符串、`submitData` 须为布尔）；`actions` 元素非对象
    （RawAction 是 struct）。仍是近似：`Option<serde_json::Value>` 字段来者不拒，
    RawAction 内部的 Vec<String>/Vec<RawFieldValue> 等嵌套类型不再下钻。
    pin 语料未命中任何一类，现有结论不受影响。

    superpage 侧对 extra 数组元素逐个 `serde_json::from_value::<RawComponent>`，
    失败即静默跳过整棵子树（extract_child_components 的已知取舍，mod.rs:375-384）；
    scanner 侧走裸 Value 不受此限。命中的元素不计入「现役遍历可达」，walk 仍会把
    它们算进漏掉候选。
    """
    if not isinstance(node, dict):
        return True
    for key in WHITELIST_CONTAINER_KEYS + ("actions",):
        if key in node and not isinstance(node[key], list):
            return True
    if "submitField" in node and not isinstance(node["submitField"], str):
        return True
    # 注意 Python bool 是 int 子类：JSON true/false 之外（数字、字符串）都判失败，
    # 与 serde 的 Option<bool> 行为一致。
    if "submitData" in node and not isinstance(node["submitData"], bool):
        return True
    if any(not isinstance(item, dict) for item in node.get("actions") or []):
        return True
    return False


def iter_current_children(raw, known_props):
    """复刻 extract_child_components（mod.rs:351-385）的子节点序列。

    白名单四键先行；extra 键（未声明、不在排除列表）形态吻合才递归。
    """
    for key in WHITELIST_CONTAINER_KEYS:
        for child in raw.get(key) or []:
            yield child
    for key, value in raw.items():
        if key in WHITELIST_CONTAINER_KEYS or key in NON_COMPONENT_CONTAINER_KEYS:
            continue
        if known_props is not None and key in known_props:
            # 已声明字段不进 flatten extra，不参与形态感知递归。
            continue
        if not is_component_array_lenient(value):
            continue
        for item in value:
            if would_fail_raw_component_deserialize(item):
                continue
            yield item


def extract_current(raw, out, known_props):
    """复刻 PR2 后的形态感知递归（mod.rs 的 extract_components），含空 id 透传规则。

    实际递归入口分散在多处：空 id 透传分支 mod.rs:394、正常分支末尾 :612，
    子容器遍历在 extract_child_components（:351-385，对应 iter_current_children）。
    """
    if not isinstance(raw, dict):
        return
    if not raw.get("id") or not raw.get("type"):
        for child in iter_current_children(raw, known_props):
            extract_current(child, out, known_props)
        return
    out.append(raw)
    for child in iter_current_children(raw, known_props):
        extract_current(child, out, known_props)


def is_expression_value(field, value):
    """复刻 superpage/mod.rs:499-512 的 is_expr 判定（仅字符串值参与）。"""
    if not isinstance(value, str) or not value:
        return False
    if field in ALWAYS_EXPR_FIELDS:
        return True
    if field in CONDITIONAL_EXPR_FIELDS:
        return value.startswith("=") or "${" in value
    return False


def audit_containers(canvas, reached_ids):
    """容器形态审计，两条遍历分开记：

    - **漏掉候选猎手**（`walk_all`）：走遍全树（含排除列表子树），凡是严格
      组件形态且未被形态感知递归到达的对象都计数；`under_excluded` 标记是否
      位于排除列表子树内（如 actions 内部的 buttons 配置），供 spec 验收 2 的
      「漏掉候选 − 排除列表命中数 == 0」判据使用。
    - **混合形态计数**（`walk_scanner`）：复刻 scanner/spg.rs:200-258 的安全网
      口径，只走 scanner 实际到达的节点——白名单四键递归 + 形态吻合 extra 键
      递归；其余非空数组含对象元素则计 `SCANNER_UNRECOGNIZED_CONTAINER_KEY`，
      不递归进去。
    """
    stats = collections.defaultdict(
        lambda: {
            "missed": 0, "under_excluded": 0,
            "with_expr": 0, "with_actions": 0, "with_children": 0,
        }
    )
    mixed_shape_keys = collections.Counter()

    def walk_all(node, container_key, under_excluded):
        if isinstance(node, dict):
            if is_component_object(node) and id(node) not in reached_ids:
                entry = stats[container_key]
                entry["missed"] += 1
                if under_excluded:
                    entry["under_excluded"] += 1
                if any(is_expression_value(k, v) for k, v in node.items()):
                    entry["with_expr"] += 1
                if node.get("actions"):
                    entry["with_actions"] += 1
                if any(is_component_array(v) for v in node.values()):
                    entry["with_children"] += 1
            for key, value in node.items():
                walk_all(value, key, under_excluded or key in NON_COMPONENT_CONTAINER_KEYS)
        elif isinstance(node, list):
            for value in node:
                walk_all(value, container_key, under_excluded)

    def walk_scanner(node):
        if not isinstance(node, dict):
            return
        for key, value in node.items():
            if key in NON_COMPONENT_CONTAINER_KEYS or key in ("id", "type"):
                continue
            if key in WHITELIST_CONTAINER_KEYS:
                if isinstance(value, list):
                    for child in value:
                        walk_scanner(child)
                continue
            if not isinstance(value, list) or not value:
                continue
            if is_component_array_lenient(value):
                for child in value:
                    walk_scanner(child)
            elif any(isinstance(i, dict) for i in value):
                mixed_shape_keys[key] += 1

    walk_all(canvas, "canvas", False)
    walk_scanner(canvas)
    return stats, mixed_shape_keys


def audit_expressions(components, source_ids, all_object_ids):
    """沿真实调用链统计 ${a.b} 引用的终态。

    链路：classify_identifier（`${}` 分支无条件 ModelField）
        → resolve_expression_refs_with_context（head 命中已抽取组件 id 则改写为
          ComponentProperty）
        → scanner/spg.rs:766-783 的 match（PR2 说明 D：ComponentProperty 与
          ComponentValue 一样建 comp→comp DependsOn 边，属性名进 edge meta）
    """
    extracted_ids = {c["id"] for c in components}
    buckets = collections.Counter()
    dropped_suffixes = collections.Counter()
    junk_model_names = set()
    junk_occurrences = 0

    for comp in components:
        for field, value in comp.items():
            if not is_expression_value(field, value):
                continue
            for match in re.finditer(r"\$\{([^}]*)\}", value):
                inner = match.group(1)
                if not DOTTED_PATH.match(inner):
                    head = inner.split(".")[0]
                    if head in extracted_ids:
                        # 上下文解析会纠正，不产生垃圾节点。
                        buckets["malformed_but_resolved"] += 1
                    else:
                        buckets["malformed_stays_modelfield"] += 1
                        junk_occurrences += 1
                        junk_model_names.add(head)
                    continue
                parts = inner.split(".")
                if len(parts) < 2:
                    continue
                head = parts[0]
                if head in source_ids or head.startswith("model"):
                    buckets["modelfield_correct"] += 1
                elif head in extracted_ids:
                    buckets["componentproperty_builds_comp_depends_on"] += 1
                    dropped_suffixes["." + ".".join(parts[1:])] += 1
                elif head in all_object_ids:
                    # head 是当前不可达的候选对象：上下文解析看不到它，仍是 ModelField。
                    buckets["head_unreachable_stays_modelfield"] += 1
                    junk_occurrences += 1
                    junk_model_names.add(head)
                else:
                    buckets["head_unknown_stays_modelfield"] += 1

    return {
        "buckets": dict(buckets),
        "componentproperty_edge_suffixes": dict(dropped_suffixes.most_common(20)),
        "junk_model_name_occurrences": junk_occurrences,
        "junk_model_names": junk_model_names,
    }


def audit_properties(components, known_props):
    """统计已抽取组件上的属性键：未被 RawComponent 覆盖、且带表达式的有多少。

    **适用范围**：覆盖形态感知递归可达的组件（PR2 后含 columns/grid/tabs 等
    新进入类型）。排除列表键下的对象不可达，不在本审计范围内。
    """
    unknown_with_expr = collections.Counter()
    unknown_any = collections.Counter()
    skipped_nonstring_with_expr = collections.Counter()
    container_like = {"actions", "components", "panels", "steps", "comps"}

    for comp in components:
        for key, value in comp.items():
            if key in known_props:
                if isinstance(value, (dict, list)) and key not in container_like:
                    if "${" in json.dumps(value, ensure_ascii=False):
                        skipped_nonstring_with_expr[key] += 1
                continue
            if is_component_array_lenient(value):
                # 子组件容器，不是属性——归容器审计，避免污染属性层结论。
                continue
            unknown_any[key] += 1
            text = value if isinstance(value, str) else json.dumps(value, ensure_ascii=False)
            if isinstance(text, str) and (text.startswith("=") or "${" in text):
                unknown_with_expr[key] += 1

    return unknown_with_expr, unknown_any, skipped_nonstring_with_expr


def load_known_component_props(repo_root):
    """从 raw_types.rs 的 RawComponent 定义提取已覆盖的属性键（serde rename 优先）。"""
    path = os.path.join(repo_root, "src", "superpage", "raw_types.rs")
    try:
        text = open(path, encoding="utf-8").read()
        block = text.split("pub struct RawComponent {")[1].split("\n}")[0]
    except (OSError, IndexError):
        return None
    names = set()
    for match in re.finditer(
        r'(?:#\[serde\((?:rename = "([^"]+)", )?default\)\]\s*)?pub (\w+):', block
    ):
        names.add(match.group(1) or match.group(2))
    # #[serde(flatten)] 的 extra 不是真实 JSON 键：计入已声明字段会让语料里的字面量
    # "extra" 键被误判跳过形态感知递归，必须从返回集合剔除。
    names.discard("extra")
    return names


def audit_dataflow(document):
    """统计 .spg 内联 dataFlow 中 tbl.rs 会解析、spg.rs 不解析的结构。"""
    result = collections.Counter()
    for source in document.get("sources") or []:
        content = source.get("content") or {}
        nodes = (content.get("dataFlow") or {}).get("nodes") or {}
        if not nodes:
            if source.get("path"):
                result["dwtable_sources"] += 1
            continue
        result["inline_dataflow_sources"] += 1
        result["inline_dataflow_nodes"] += len(nodes)
        for node in nodes.values():
            for key in ("fields", "alias", "inputNodes", "joinConditions", "steps"):
                if node.get(key):
                    result["node_attr_" + key] += 1
    return result


def corpus_revision(corpus_dir):
    try:
        out = subprocess.run(
            ["git", "-C", corpus_dir, "rev-parse", "--short", "HEAD"],
            capture_output=True, text=True, check=True,
        )
        return out.stdout.strip()
    except Exception:
        return "unknown"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", required=True, help="语料根目录（含 .spg 的项目目录）")
    parser.add_argument("--json", help="结构化结果输出路径")
    args = parser.parse_args()

    if not os.path.isdir(args.corpus):
        print("corpus not found: %s" % args.corpus, file=sys.stderr)
        return 2

    spg_files = sorted(
        os.path.join(root, name)
        for root, _, names in os.walk(args.corpus)
        for name in names if name.endswith(".spg")
    )

    totals = collections.Counter()
    container_stats = collections.defaultdict(
        lambda: {
            "missed": 0, "under_excluded": 0,
            "with_expr": 0, "with_actions": 0, "with_children": 0,
        }
    )
    mixed_shape = collections.Counter()
    expr_buckets = collections.Counter()
    dropped_suffixes = collections.Counter()
    junk_names = set()
    junk_occ = 0
    dataflow = collections.Counter()
    unknown_expr_props = collections.Counter()
    unknown_props = collections.Counter()
    skipped_nonstring = collections.Counter()

    repo_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    known_props = load_known_component_props(repo_root)

    for path in spg_files:
        with open(path, encoding="utf-8") as handle:
            document = json.load(handle)
        canvas = document.get("canvas") or {}

        reached = []
        extract_current(canvas, reached, known_props)
        reached_ids = {id(c) for c in reached}
        totals["reached_by_current_traversal"] += len(reached)

        all_object_ids = set()

        def collect_ids(node):
            if isinstance(node, dict):
                if is_component_object(node):
                    all_object_ids.add(node["id"])
                for value in node.values():
                    collect_ids(value)
            elif isinstance(node, list):
                for value in node:
                    collect_ids(value)

        collect_ids(document)

        stats, mixed = audit_containers(canvas, reached_ids)
        for key, entry in stats.items():
            for field, value in entry.items():
                container_stats[key][field] += value
        mixed_shape.update(mixed)

        source_ids = {s.get("id") for s in (document.get("sources") or []) if s.get("id")}
        expr = audit_expressions(reached, source_ids, all_object_ids)
        expr_buckets.update(expr["buckets"])
        dropped_suffixes.update(expr["componentproperty_edge_suffixes"])
        junk_occ += expr["junk_model_name_occurrences"]
        junk_names |= expr["junk_model_names"]
        dataflow.update(audit_dataflow(document))
        if known_props is not None:
            a, b, c = audit_properties(reached, known_props)
            unknown_expr_props.update(a)
            unknown_props.update(b)
            skipped_nonstring.update(c)

    missed_total = sum(e["missed"] for e in container_stats.values())
    # 排除命中按子树继承口径：位于排除列表键子树内（任意深度）的漏掉候选都算，
    # 例如 actions 排除后其内部的 buttons 配置随之不可达。
    exclusion_hits = sum(e["under_excluded"] for e in container_stats.values())
    acceptance_gap = missed_total - exclusion_hits
    behavioral = {
        k: e for k, e in container_stats.items()
        if e["with_expr"] or e["with_actions"] or e["with_children"]
    }
    inert = {k: e for k, e in container_stats.items() if k not in behavioral}

    report = {
        "measurement_boundary": (
            "读原始 JSON 并复刻解析器遍历/分类规则，不构建 graphdb；"
            "所有数字是候选量，图内事实须重建后复测"
        ),
        "corpus_dir": args.corpus,
        "corpus_revision": corpus_revision(args.corpus),
        "script_sha256": hashlib.sha256(
            open(__file__, "rb").read()
        ).hexdigest()[:16],
        "spg_files": len(spg_files),
        "containers": {
            "traversal_rule": "PR2 形态感知递归（白名单四键 + extra 形态吻合键，排除列表优先）",
            "reached_by_current_traversal": totals["reached_by_current_traversal"],
            "missed_candidates_total": missed_total,
            "missed_with_behavior_evidence": sum(e["missed"] for e in behavioral.values()),
            "missed_without_behavior_evidence": sum(e["missed"] for e in inert.values()),
            "exclusion_list_keys": sorted(NON_COMPONENT_CONTAINER_KEYS),
            "missed_under_exclusion_subtrees": exclusion_hits,
            "acceptance_gap_missed_minus_exclusion": acceptance_gap,
            "by_key": dict(sorted(
                container_stats.items(), key=lambda kv: -kv[1]["missed"]
            )),
            "mixed_shape_arrays_by_key": dict(mixed_shape.most_common(20)),
        },
        "expressions": {
            "terminal_states": dict(expr_buckets),
            "componentproperty_edge_suffixes": dict(dropped_suffixes.most_common(20)),
            "junk_model_name_occurrences": junk_occ,
            "junk_model_names_distinct": len(junk_names),
        },
        "dataflow": dict(dataflow),
        "properties": {
            "scope_limit": (
                "覆盖形态感知递归可达的组件（PR2 后含 columns/grid/tabs 等新进入类型）"
            ),
            "raw_component_known_keys": len(known_props) if known_props else None,
            "unknown_keys_with_expression": dict(unknown_expr_props.most_common(20)),
            "unknown_keys_top": dict(unknown_props.most_common(10)),
            "known_keys_nonstring_value_containing_expr": dict(
                skipped_nonstring.most_common(10)
            ),
        },
    }

    if args.json:
        with open(args.json, "w", encoding="utf-8") as handle:
            json.dump(report, handle, ensure_ascii=False, indent=2, sort_keys=True)

    c = report["containers"]
    print("语料 %s @ %s，%d 个 .spg" % (
        args.corpus, report["corpus_revision"], report["spg_files"]))
    print("\n== 容器形态（附录 A，PR2 形态感知递归口径）==")
    print("现役遍历到达 %d ; 漏掉候选 %d（有行为证据 %d / 无行为证据 %d）" % (
        c["reached_by_current_traversal"], c["missed_candidates_total"],
        c["missed_with_behavior_evidence"], c["missed_without_behavior_evidence"]))
    print("验收口径：漏掉候选 %d − 排除列表子树命中 %d = %d（spec 验收 2 要求 == 0）" % (
        c["missed_candidates_total"], c["missed_under_exclusion_subtrees"],
        c["acceptance_gap_missed_minus_exclusion"]))
    print("%-16s %8s %8s %10s %10s" % ("容器键", "漏掉", "带表达式", "带actions", "带子容器"))
    for key, entry in c["by_key"].items():
        print("%-16s %8d %8d %10d %10d" % (
            key, entry["missed"], entry["with_expr"],
            entry["with_actions"], entry["with_children"]))
    if c["mixed_shape_arrays_by_key"]:
        print("\n混合形态数组（scanner 安全网口径，进 SCANNER_UNRECOGNIZED_CONTAINER_KEY）:")
        for key, count in c["mixed_shape_arrays_by_key"].items():
            print("  %6d  %s" % (count, key))

    print("\n== 表达式引用终态（沿真实调用链）==")
    print("  畸形 ${} 产生的垃圾 model 名：%d 次 / %d 个不同名（候选量，非图内实测）" % (
        report["expressions"]["junk_model_name_occurrences"],
        report["expressions"]["junk_model_names_distinct"]))
    for state, count in sorted(report["expressions"]["terminal_states"].items(),
                               key=lambda kv: -kv[1]):
        print("  %6d  %s" % (count, state))
    if report["expressions"]["componentproperty_edge_suffixes"]:
        print("ComponentProperty 建 comp→comp DependsOn 边的属性后缀:")
        for suffix, count in report["expressions"]["componentproperty_edge_suffixes"].items():
            print("  %6d  %s" % (count, suffix))

    props = report["properties"]
    if props["raw_component_known_keys"]:
        print("\n== 属性层（形态感知递归可达组件，PR2 后重跑）==")
        print("RawComponent 已覆盖 %d 个键" % props["raw_component_known_keys"])
        print("未覆盖且带表达式的键:")
        for key, count in props["unknown_keys_with_expression"].items():
            print("  %6d  %s" % (count, key))
        print("量最大的未覆盖键（多为样式，正确忽略）:")
        for key, count in list(props["unknown_keys_top"].items())[:6]:
            print("  %6d  %s" % (count, key))
        print("已知键但值为对象/数组且内含 ${}（被 `_ => continue` 跳过）: %d 处" % sum(
            props["known_keys_nonstring_value_containing_expr"].values()))

    print("\n== DataFlow 解析深度差 ==")
    for key, count in sorted(report["dataflow"].items(), key=lambda kv: -kv[1]):
        print("  %6d  %s" % (count, key))

    if args.json:
        print("\nJSON: %s" % args.json)
    return 0


if __name__ == "__main__":
    sys.exit(main())
