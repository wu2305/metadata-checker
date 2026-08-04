//! 三动词命令表面路由。
//!
//! M58 的评测数据显示：115 条被拒命令里约八成不是模型选错了，而是命令表面本身无法从
//! 问题里判定。最集中的两处——
//!
//! - `--explain` 与 `--explain-condition` 在同一个按钮上互为镜像（34 条拒绝，占 30%）：
//!   两个问法几乎一样的 case，模型把各自的答案发给了对方。
//! - `--context` 在 6 轮 18 次尝试里被接受 **0 次**；`--query-page`、`--query-cross`、
//!   `--find-model`、`--advise-query` 一次都没被选中。
//!
//! 结论是动词太多，且动词承担了本该由 target 前缀承担的判别工作。这里把 12 个查询动词
//! 收敛成 3 个，让**前缀决定去哪、动词只决定问什么**：
//!
//! | 动词 | 回答的问题 | 吸收 |
//! |---|---|---|
//! | `--find` | 我说的这个名字在哪？ | `--find-page` / `--find-model` / `--find-component` |
//! | `--explain` | 它是什么、做什么、为什么这样 | `--explain` / `--explain-condition` / `--context` |
//! | `--relations` | 谁读它、谁写它、它连到哪 | `--query-model` / `--query-page` / `--query-page-logic` / `--query-dataflow` / `--query-cross` |
//!
//! 旧动词全部保留为隐藏别名，直接映射到原有 `ToolCommand`，外部调用方不受影响。

use crate::tool_contract::{ToolCommand, ToolError, ToolErrorCode};

/// 合法的 target 类型前缀。路由完全由它决定，动词不再参与判别。
pub const TARGET_PREFIXES: &[&str] =
    &["comp:", "action:", "field:", "model:", "page:", "dataflow:"];

/// 新命令表面的三个动词。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// 名字 -> 规范 target。
    Find,
    /// 单个节点的身份、行为与成因。
    Explain,
    /// 单个节点的读写与连接关系。
    Relations,
}

impl Surface {
    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Find => "--find",
            Surface::Explain => "--explain",
            Surface::Relations => "--relations",
        }
    }
}

/// 路由展开出的一次内部调用。
///
/// 一个表面动词可以展开成多次内部调用，输出按顺序合并；这正是 `--explain` 能同时给出
/// 节点语义和条件成因、从而让两个动词的区分变得不可观测的方式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedCall {
    pub command: ToolCommand,
    pub target: String,
    /// 合并进主输出 `details` 时使用的子键；`None` 表示这一条就是主输出。
    pub merge_key: Option<&'static str>,
    /// `false` 时该调用失败只记 diagnostic，不让整次表面调用失败。
    ///
    /// 补充调用天然可能不适用于当前节点类型（例如对 `field:` 求 DataFlow 子图），
    /// 让它把整条命令拖失败，等于用新的方式重新制造「选错动词就一无所获」。
    pub required: bool,
}

impl RoutedCall {
    fn primary(command: ToolCommand, target: &str) -> Self {
        Self {
            command,
            target: target.to_string(),
            merge_key: None,
            required: true,
        }
    }

    fn supplement(command: ToolCommand, target: &str, merge_key: &'static str) -> Self {
        Self {
            command,
            target: target.to_string(),
            merge_key: Some(merge_key),
            required: false,
        }
    }
}

/// 一次表面调用的完整展开。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePlan {
    pub surface: Surface,
    pub calls: Vec<RoutedCall>,
}

/// 把表面动词 + target 展开成内部调用序列。
///
/// 纯函数，不接触图；target 必须已经带类型前缀（裸名先经 [`resolve_bare_target`] 归一）。
pub fn route(surface: Surface, target: &str, depth: Option<usize>) -> Result<RoutePlan, ToolError> {
    let target = target.trim();
    if target.is_empty() {
        return Err(ToolError::new(
            ToolErrorCode::MissingTarget,
            format!("{} 需要一个 target", surface.as_str()),
        ));
    }

    let calls = match surface {
        // 三个 find 动词只差一个 node_type 过滤器，而模型在问「button1 在哪」时并不知道
        // button1 是组件还是页面——知道了就不用找了。合成一个不带过滤器的搜索。
        Surface::Find => vec![RoutedCall::primary(ToolCommand::Find, target)],
        Surface::Explain => route_explain(target, depth)?,
        Surface::Relations => route_relations(target)?,
    };

    Ok(RoutePlan { surface, calls })
}

/// `--explain`：节点语义 + 条件成因（+ 显式 `--depth` 时补上下游）。
fn route_explain(target: &str, depth: Option<usize>) -> Result<Vec<RoutedCall>, ToolError> {
    require_prefix(Surface::Explain, target)?;

    let mut calls = vec![
        RoutedCall::primary(ToolCommand::Explain, target),
        RoutedCall::supplement(ToolCommand::ExplainCondition, target, "condition_facts"),
    ];

    // `--context` 单独作为动词时 6 轮 18 次尝试 0 次被选中，模型一律改用 `--explain`。
    // 保留能力、去掉动词：只有显式要了深度才展开邻居。
    if depth.is_some_and(|value| value >= 1) {
        calls.push(RoutedCall::supplement(
            ToolCommand::Context,
            target,
            "neighbor_context",
        ));
    }

    Ok(calls)
}

/// `--relations`：由前缀决定问哪张关系表。
fn route_relations(target: &str) -> Result<Vec<RoutedCall>, ToolError> {
    // 两个页面用逗号连接是既有的跨页协议，保持不变。
    if target.contains(',') {
        crate::tool_contract::parse_query_cross_target(target)?;
        return Ok(vec![RoutedCall::primary(ToolCommand::QueryCross, target)]);
    }

    if target.starts_with("page:") {
        return Ok(vec![
            RoutedCall::primary(ToolCommand::QueryPageLogic, target),
            RoutedCall::supplement(ToolCommand::QueryPage, target, "page_dependencies"),
        ]);
    }

    if target.starts_with("dataflow:") {
        return Ok(vec![RoutedCall::primary(
            ToolCommand::QueryDataflow,
            target,
        )]);
    }

    if target.starts_with("model:") {
        // 模型是不是 DataFlow 要读图才知道，而模型问「这张表怎么来的」时并不区分。
        // 两条都发，DataFlow 子图取不到就只是缺一个补充块。
        return Ok(vec![
            RoutedCall::primary(ToolCommand::QueryModel, target),
            RoutedCall::supplement(ToolCommand::QueryDataflow, target, "dataflow_subgraph"),
        ]);
    }

    require_prefix(Surface::Relations, target)?;

    // comp: / action: / field: 没有独立的关系表，它们的上下游属于 --explain 的职责。
    // 明确说出该走哪条，而不是丢一个 INVALID_TARGET 让模型再猜一轮。
    Err(ToolError::new(
        ToolErrorCode::InvalidTarget,
        format!(
            "--relations 不接受 '{target}'：组件/动作/字段的关系请用 --explain（--relations 只接受 page: / model: / dataflow:）"
        ),
    ))
}

/// 校验 target 带合法类型前缀，并在报错时把可选前缀写清楚。
fn require_prefix(surface: Surface, target: &str) -> Result<(), ToolError> {
    if TARGET_PREFIXES
        .iter()
        .any(|prefix| target.starts_with(prefix))
    {
        return Ok(());
    }
    Err(ToolError::new(
        ToolErrorCode::InvalidTarget,
        format!(
            "{} 的 target '{}' 缺少类型前缀，可选：{}（或用 --find 先定位）",
            surface.as_str(),
            target,
            TARGET_PREFIXES.join(" "),
        ),
    ))
}

/// 裸名归一的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BareTargetResolution {
    /// 唯一命中，可直接当 target 使用。
    Resolved { target: String, diagnostic: String },
    /// 多个候选；调用方应把候选作为 diagnostics 返回，而不是让模型盲猜。
    Ambiguous { candidates: Vec<String> },
    /// 没有命中。
    NotFound,
}

/// 判断 target 是否已经带了类型前缀。
pub fn has_type_prefix(target: &str) -> bool {
    TARGET_PREFIXES
        .iter()
        .any(|prefix| target.starts_with(prefix))
}

/// 从一次 `--find` 输出里把裸名归一成规范 target。
///
/// M58 里 `context_button1_neighbors` 的问题是「button1 周围还有哪些依赖」——没有页面。
/// 模型的应对是 `--find-component button1`（8 次）、`comp:app/未知页面.spg|button1`（3 次），
/// 以及把 bootstrap 里的占位符原样抄成 `comp:app/<relative-file>.spg|button1`（2 次）。
/// 定位是确定性的图查询，不该由模型来编路径：这里在 Rust 里做掉。
///
/// 输入取 `--find` 的输出而不是图本身，是为了让归一和搜索走同一条通路——搜得到的东西
/// 一定能被寻址，不会出现「find 说存在、explain 说 target 非法」这种自相矛盾。
pub fn resolve_bare_target(bare: &str, find_output: &serde_json::Value) -> BareTargetResolution {
    let matches = find_output
        .get("details")
        .and_then(|details| details.get("matches"))
        .and_then(|matches| matches.as_array())
        .cloned()
        .unwrap_or_default();

    // 只认精确命中。子串命中（score 80）在「button1」这种通用 id 上会一次带回几十个节点，
    // 拿第一个当答案就是换个地方猜。
    let exact: Vec<String> = matches
        .iter()
        .filter(|entry| {
            entry
                .get("match_score")
                .and_then(serde_json::Value::as_f64)
                .is_some_and(|score| score >= 100.0)
        })
        .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect();

    match exact.len() {
        0 => BareTargetResolution::NotFound,
        1 => {
            let target = exact[0].clone();
            BareTargetResolution::Resolved {
                diagnostic: format!("RESOLVED_TARGET: '{bare}' -> '{target}'"),
                target,
            }
        }
        _ => BareTargetResolution::Ambiguous { candidates: exact },
    }
}

/// 带前缀但没写全的 target 的归一结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefixedTargetResolution {
    /// target 本身就是一个真实节点 id，不需要动。
    Exact,
    /// 唯一命中，可直接替换。
    Resolved { target: String, diagnostic: String },
    /// 多个候选；交回候选，不猜。
    Ambiguous { candidates: Vec<String> },
    /// 没有结构性命中；`candidates` 是尾段同名的近似项，可能为空。
    NotFound { candidates: Vec<String> },
}

/// 候选列表的硬上限：交回候选是为了让模型下一轮能直接用，几十条candidates 和不给
/// 一样没用。
const MAX_CANDIDATES: usize = 12;

/// 可以被省略的元数据文件扩展名。
const METADATA_EXTENSIONS: &[&str] = &[".spg", ".tbl"];

fn strip_metadata_extension(value: &str) -> &str {
    METADATA_EXTENSIONS
        .iter()
        .find_map(|extension| {
            value
                .len()
                .checked_sub(extension.len())
                .filter(|split| value[*split..].eq_ignore_ascii_case(extension))
                .map(|split| &value[..split])
        })
        .unwrap_or(value)
}

/// 拆成 (前缀, `|` 分隔的段)。没有合法前缀时返回 `None`。
fn split_prefixed(target: &str) -> Option<(&'static str, Vec<&str>)> {
    let prefix = TARGET_PREFIXES
        .iter()
        .find(|prefix| target.starts_with(**prefix))?;
    Some((prefix, target[prefix.len()..].split('|').collect()))
}

/// 第一段之外的段必须逐段相等——那是节点自身的身份，不允许模糊。
fn tail_matches(have: &[&str], want: &[&str]) -> bool {
    have.iter()
        .zip(want.iter())
        .skip(1)
        .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

/// 文件段按「路径后缀 + 忽略扩展名」匹配。
///
/// `actions_test`、`actions_test.spg`、`app/actions_test.spg` 都应该指向
/// `app/actions_test.spg`：模型知道页面叫什么，不知道它在仓库里的哪一层。
fn file_matches(have: &str, want: &str) -> bool {
    let have = strip_metadata_extension(have).to_lowercase();
    let want = strip_metadata_extension(want).to_lowercase();
    have == want || have.ends_with(&format!("/{want}"))
}

/// 把「带前缀但没写全」的 target 归一成真实节点 id。
///
/// M59 的评测暴露了一个新问题：三动词收敛之后模型几乎总能选对动词，但 109 条拒绝里
/// 有 91 条（83%）栽在 target 的写法上——`page:actions_test`、`comp:actions_test|button1`、
/// `comp:actions_test.spg|button1`。[`resolve_bare_target`] 只在完全没有前缀时才触发，
/// 而新表面恰恰教会了模型总是写前缀，于是归一在 117 次 trial 里只生效了 1 次。
///
/// 这里补上另一半：前缀写对了、文件路径没写全的，同样由 Rust 确定性地定位。分三档，
/// 每一档都要么唯一命中、要么如实交回候选，不存在「取第一个」：
///
/// 1. target 就是真实 id —— 原样通过。
/// 2. 文件段按路径后缀匹配、其余段完全一致 —— `page:actions_test`。
/// 3. 文件路径根本不存在，但其余段唯一确定一个节点 —— `comp:app/未知页面.spg|button1`
///    这类模型凭空编出来的路径。评测里 19 条拒绝属于此类，此前一个候选都拿不到。
pub fn normalize_prefixed_target<'a>(
    target: &str,
    known_ids: impl IntoIterator<Item = &'a str>,
) -> PrefixedTargetResolution {
    let Some((prefix, want)) = split_prefixed(target) else {
        return PrefixedTargetResolution::NotFound {
            candidates: Vec::new(),
        };
    };

    // 文件段后缀命中，其余段完全一致。
    let mut by_file: Vec<String> = Vec::new();
    // 文件段对不上，但其余段完全一致——模型编了路径，节点身份是对的。
    let mut by_identity: Vec<String> = Vec::new();
    // 尾段同名的近似项，仅在前两档都空时作为 candidates 交回。
    let mut by_tail: Vec<String> = Vec::new();
    let last_want = want.last().copied().unwrap_or_default();

    for id in known_ids {
        if id == target {
            return PrefixedTargetResolution::Exact;
        }
        let Some((id_prefix, have)) = split_prefixed(id) else {
            continue;
        };

        if id_prefix == prefix && have.len() == want.len() && tail_matches(&have, &want) {
            if file_matches(have[0], want[0]) {
                by_file.push(id.to_string());
                continue;
            }
            // 只有一段的 target（`page:`、`model:`）没有可用来定身份的尾段，
            // 放进这一档等于把整个仓库的同类节点都当候选。
            if want.len() >= 2 {
                by_identity.push(id.to_string());
                continue;
            }
        }

        if have
            .last()
            .is_some_and(|segment| file_matches(segment, last_want))
        {
            by_tail.push(id.to_string());
        }
    }

    for (tier, note) in [
        (&mut by_file, "补全文件路径"),
        (&mut by_identity, "target 里的文件路径不存在，按节点身份定位"),
    ] {
        tier.sort();
        match tier.len() {
            0 => continue,
            1 => {
                let resolved = tier[0].clone();
                return PrefixedTargetResolution::Resolved {
                    diagnostic: format!("RESOLVED_TARGET: '{target}' -> '{resolved}'（{note}）"),
                    target: resolved,
                };
            }
            _ => {
                tier.truncate(MAX_CANDIDATES);
                return PrefixedTargetResolution::Ambiguous {
                    candidates: std::mem::take(tier),
                };
            }
        }
    }

    by_tail.sort();
    by_tail.truncate(MAX_CANDIDATES);
    PrefixedTargetResolution::NotFound {
        candidates: by_tail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(plan: &RoutePlan) -> Vec<ToolCommand> {
        plan.calls.iter().map(|call| call.command).collect()
    }

    /// `--explain` 必须同时展开语义和条件两路。
    ///
    /// 这是整次收敛的核心：只要一次调用两路都给，`--explain` 与 `--explain-condition`
    /// 之间那 34 条镜像拒绝就没有了可观测的区分点，模型也就无从选错。
    #[test]
    fn test_explain_expands_semantics_and_conditions() {
        let plan = route(Surface::Explain, "comp:app/actions_test.spg|button2", None).unwrap();
        assert_eq!(
            commands(&plan),
            vec![ToolCommand::Explain, ToolCommand::ExplainCondition]
        );
        assert!(!plan.calls[1].required, "条件块取不到不应让整条命令失败");
    }

    /// M58 里互为镜像的两个 case，收敛后必须展开成同一组内部调用。
    #[test]
    fn test_mirrored_button_targets_route_identically() {
        let by_component =
            route(Surface::Explain, "comp:app/actions_test.spg|button2", None).unwrap();
        let by_action = route(
            Surface::Explain,
            "action:app/actions_test.spg|button2|action1",
            None,
        )
        .unwrap();
        assert_eq!(commands(&by_component), commands(&by_action));
    }

    /// 只有显式给了 depth 才补邻居，否则 `--explain` 每次都要多付一次上下文的钱。
    #[test]
    fn test_context_only_on_explicit_depth() {
        let without = route(Surface::Explain, "comp:app/a.spg|b", None).unwrap();
        assert!(!commands(&without).contains(&ToolCommand::Context));

        let with = route(Surface::Explain, "comp:app/a.spg|b", Some(2)).unwrap();
        assert!(commands(&with).contains(&ToolCommand::Context));
    }

    /// `--relations` 完全按前缀分流。
    #[test]
    fn test_relations_dispatches_by_prefix() {
        assert_eq!(
            commands(&route(Surface::Relations, "page:app/a.spg", None).unwrap()),
            vec![ToolCommand::QueryPageLogic, ToolCommand::QueryPage]
        );
        assert_eq!(
            commands(&route(Surface::Relations, "model:m1", None).unwrap()),
            vec![ToolCommand::QueryModel, ToolCommand::QueryDataflow]
        );
        assert_eq!(
            commands(&route(Surface::Relations, "dataflow:df", None).unwrap()),
            vec![ToolCommand::QueryDataflow]
        );
        assert_eq!(
            commands(&route(Surface::Relations, "page:app/a.spg,page:app/b.spg", None).unwrap()),
            vec![ToolCommand::QueryCross]
        );
    }

    /// 组件走 `--relations` 时要直接说清该用哪个动词，而不是只回一个 INVALID_TARGET。
    #[test]
    fn test_relations_rejects_component_with_actionable_hint() {
        let err = route(Surface::Relations, "comp:app/a.spg|b", None).unwrap_err();
        assert_eq!(err.code, ToolErrorCode::InvalidTarget);
        assert!(err.message.contains("--explain"), "{}", err.message);
    }

    /// 缺前缀的报错必须列出可选前缀并指向 --find，否则模型只能继续编路径。
    #[test]
    fn test_missing_prefix_error_lists_options() {
        let err = route(Surface::Explain, "button1", None).unwrap_err();
        assert_eq!(err.code, ToolErrorCode::InvalidTarget);
        assert!(err.message.contains("comp:"), "{}", err.message);
        assert!(err.message.contains("--find"), "{}", err.message);
    }

    /// `--find` 不带类型过滤：问「这个名字在哪」的人不知道它是什么类型。
    #[test]
    fn test_find_is_type_agnostic() {
        let plan = route(Surface::Find, "button1", None).unwrap();
        assert_eq!(commands(&plan), vec![ToolCommand::Find]);
    }

    #[test]
    fn test_empty_target_is_missing_target() {
        let err = route(Surface::Explain, "   ", None).unwrap_err();
        assert_eq!(err.code, ToolErrorCode::MissingTarget);
    }

    fn find_output(matches: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "details": { "matches": matches } })
    }

    /// 唯一精确命中直接归一，并留下可审计的 diagnostic。
    #[test]
    fn test_bare_target_resolves_to_canonical_id() {
        let output = find_output(serde_json::json!([
            { "id": "comp:app/actions_test.spg|button1", "match_score": 100.0 },
            { "id": "comp:app/other.spg|button12", "match_score": 80.0 },
        ]));
        match resolve_bare_target("button1", &output) {
            BareTargetResolution::Resolved { target, diagnostic } => {
                assert_eq!(target, "comp:app/actions_test.spg|button1");
                assert!(diagnostic.contains("RESOLVED_TARGET"), "{diagnostic}");
            }
            other => panic!("expected resolved, got {other:?}"),
        }
    }

    /// 子串命中不参与归一。
    ///
    /// `button1` 在真实项目里会子串命中几十个节点，取第一个当答案只是把猜测从模型
    /// 挪进了 Rust。宁可返回候选让上层如实报告歧义。
    #[test]
    fn test_substring_matches_never_resolve() {
        let output = find_output(serde_json::json!([
            { "id": "comp:app/a.spg|button10", "match_score": 80.0 },
            { "id": "comp:app/b.spg|button11", "match_score": 80.0 },
        ]));
        assert_eq!(
            resolve_bare_target("button1", &output),
            BareTargetResolution::NotFound
        );
    }

    /// 同名节点必须如实报歧义并交出候选。
    #[test]
    fn test_duplicate_exact_matches_report_candidates() {
        let output = find_output(serde_json::json!([
            { "id": "comp:app/a.spg|button1", "match_score": 100.0 },
            { "id": "comp:app/b.spg|button1", "match_score": 100.0 },
        ]));
        match resolve_bare_target("button1", &output) {
            BareTargetResolution::Ambiguous { candidates } => assert_eq!(candidates.len(), 2),
            other => panic!("expected ambiguous, got {other:?}"),
        }
    }

    /// 评测里真实出现过的节点 id 子集。
    const KNOWN: &[&str] = &[
        "page:app/actions_test.spg",
        "page:app/page_relations.spg",
        "comp:app/actions_test.spg|button1",
        "comp:app/actions_test.spg|button2",
        "comp:app/page_relations.spg|button1",
        "comp:app/actions_test.spg|input_chain_a",
        "action:app/actions_test.spg|button2|action1",
        "model:model1",
    ];

    fn normalize(target: &str) -> PrefixedTargetResolution {
        normalize_prefixed_target(target, KNOWN.iter().copied())
    }

    /// 真实 id 原样通过，不能被归一改写。
    #[test]
    fn test_exact_prefixed_target_is_untouched() {
        assert_eq!(
            normalize("comp:app/actions_test.spg|button1"),
            PrefixedTargetResolution::Exact
        );
    }

    /// 省掉目录和扩展名的页面 target 必须能定位。
    ///
    /// M59 评测里 `page:actions_test` 这一类占了拒绝的大头：模型知道页面叫什么，
    /// 不知道它在仓库的哪一层——而那是一次确定性图查询。
    #[test]
    fn test_page_target_without_directory_or_extension_resolves() {
        for target in ["page:actions_test", "page:actions_test.spg"] {
            match normalize(target) {
                PrefixedTargetResolution::Resolved { target: resolved, .. } => {
                    assert_eq!(resolved, "page:app/actions_test.spg");
                }
                other => panic!("{target} 应当归一，实际 {other:?}"),
            }
        }
    }

    /// 组件 target 的文件段同样可以只写文件名。
    #[test]
    fn test_component_target_with_bare_file_resolves() {
        match normalize("comp:actions_test|button2") {
            PrefixedTargetResolution::Resolved { target, .. } => {
                assert_eq!(target, "comp:app/actions_test.spg|button2");
            }
            other => panic!("expected resolved, got {other:?}"),
        }
    }

    /// 文件段没写、组件名跨页面重名时，交回真实候选而不是猜一个。
    #[test]
    fn test_ambiguous_file_segment_returns_real_candidates() {
        match normalize("comp:unknown_page.spg|button1") {
            PrefixedTargetResolution::Ambiguous { candidates } => {
                assert_eq!(
                    candidates,
                    vec![
                        "comp:app/actions_test.spg|button1".to_string(),
                        "comp:app/page_relations.spg|button1".to_string(),
                    ]
                );
            }
            other => panic!("expected ambiguous, got {other:?}"),
        }
    }

    /// 模型编出来的路径 + 唯一的节点身份 = 可以确定性定位。
    ///
    /// `comp:app/售后.app/首页.spg|button1` 这种凭空捏造的路径在评测里出现 19 次，
    /// 此前一条候选都拿不到。只要 `|` 后面的身份唯一，路径写错并不妨碍定位。
    #[test]
    fn test_hallucinated_path_resolves_by_node_identity() {
        match normalize("comp:app/does_not_exist.spg|input_chain_a") {
            PrefixedTargetResolution::Resolved { target, diagnostic } => {
                assert_eq!(target, "comp:app/actions_test.spg|input_chain_a");
                assert!(diagnostic.contains("RESOLVED_TARGET"), "{diagnostic}");
            }
            other => panic!("expected resolved, got {other:?}"),
        }
    }

    /// 尾段身份不同的节点不能互相归一。
    ///
    /// 归一只补路径，不能改节点——否则等于在 Rust 里替模型换了个问题回答。
    #[test]
    fn test_normalization_never_changes_node_identity() {
        match normalize("comp:app/actions_test.spg|no_such_component") {
            PrefixedTargetResolution::NotFound { candidates } => assert!(candidates.is_empty()),
            other => panic!("expected not found, got {other:?}"),
        }
    }

    /// 前缀写错时，交回尾段同名的真实节点，让模型下一轮能换前缀。
    #[test]
    fn test_wrong_prefix_still_yields_tail_candidates() {
        match normalize("model:button2") {
            PrefixedTargetResolution::NotFound { candidates } => {
                assert!(
                    candidates.contains(&"comp:app/actions_test.spg|button2".to_string()),
                    "{candidates:?}"
                );
            }
            other => panic!("expected not found with candidates, got {other:?}"),
        }
    }

    /// 单段 target 不走身份档，否则一个 `page:` 会把全仓库的页面当候选。
    #[test]
    fn test_single_segment_target_does_not_match_every_page() {
        match normalize("page:nowhere") {
            PrefixedTargetResolution::NotFound { candidates } => assert!(candidates.is_empty()),
            other => panic!("expected not found, got {other:?}"),
        }
    }

    #[test]
    fn test_has_type_prefix() {
        assert!(has_type_prefix("comp:app/a.spg|b"));
        assert!(has_type_prefix("dataflow:df"));
        assert!(!has_type_prefix("button1"));
    }
}
