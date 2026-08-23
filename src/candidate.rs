//! 候选节点匹配与排序的共享实现。

use crate::graph::{Node, levenshtein};

const ID_PREFIXES: [&str; 5] = ["model", "page", "comp", "action", "field"];

fn node_prefix(id: &str) -> Option<&'static str> {
    ID_PREFIXES
        .iter()
        .copied()
        .find(|prefix| id.starts_with(&format!("{prefix}:")))
}

fn bare_id(id: &str) -> &str {
    ID_PREFIXES
        .iter()
        .find_map(|prefix| id.strip_prefix(&format!("{prefix}:")))
        .unwrap_or(id)
}

fn numeric_suffix_variant(target_bare: &str, node_bare: &str) -> bool {
    node_bare
        .strip_prefix(target_bare)
        .filter(|suffix| !suffix.is_empty())
        .is_some_and(|suffix| suffix.parse::<f64>().is_ok())
}

fn candidate_score(node: &Node, target_id: &str) -> Option<(f64, &'static str)> {
    let target_lower = target_id.to_lowercase();
    let target_prefix = node_prefix(target_id);
    let target_bare = bare_id(target_id);
    let target_bare_lower = target_bare.to_lowercase();

    if node.id == target_id || node.id.trim().is_empty() || node.name.trim().is_empty() {
        return None;
    }

    let node_lower = node.id.to_lowercase();
    let node_prefix_value = node_prefix(&node.id);
    let node_bare = bare_id(&node.id);
    let node_bare_lower = node_bare.to_lowercase();

    let mut score = 0.0;
    let mut reason = "substring match";

    if let Some(prefix) = target_prefix {
        if node_prefix_value == Some(prefix) {
            if node_bare_lower == target_bare_lower {
                score = 100.0;
                reason = "exact bare name match";
            } else if node_bare_lower.contains(&target_bare_lower)
                || target_bare_lower.contains(&node_bare_lower)
            {
                score = 80.0;
                reason = "bare name substring match";
            } else if node_lower.contains(&target_lower) || target_lower.contains(&node_lower) {
                score = 60.0;
                reason = "full id substring match";
            } else {
                let id_distance = levenshtein(&node_lower, &target_lower);
                let bare_distance = levenshtein(&node_bare_lower, &target_bare_lower);
                let distance = id_distance.min(bare_distance);
                if distance == 1 {
                    score = 55.0;
                    reason = "typo edit distance 1";
                } else if distance == 2 {
                    score = 45.0;
                    reason = "typo edit distance 2";
                } else if numeric_suffix_variant(target_bare, node_bare) {
                    score = 40.0;
                    reason = "numbered suffix variant";
                }
            }

            if score == 0.0 && prefix == "comp" && node_prefix_value == Some("comp") {
                score = 5.0;
                reason = "same prefix (component)";
            }
            if numeric_suffix_variant(target_bare, node_bare) {
                score = score.max(40.0);
                reason = "numbered suffix variant";
            }
        } else if !target_bare.is_empty() && node_bare.eq_ignore_ascii_case(target_bare) {
            score = 70.0;
            reason = "bare name match with different prefix";
        }
    } else if node.name.to_lowercase() == target_lower
        || node_lower == target_lower
        || node_bare_lower == target_bare_lower
    {
        score = 100.0;
        reason = "exact match";
    } else if node.name.to_lowercase().contains(&target_lower)
        || node_lower.contains(&target_lower)
        || node_bare_lower.contains(&target_bare_lower)
    {
        score = 80.0;
        reason = "name/id substring match";
    } else if node.path.to_lowercase().contains(&target_lower) {
        score = 50.0;
        reason = "path substring match";
    }

    (score > 0.0).then_some((score, reason))
}

/// 对一组节点应用统一候选匹配、去重和稳定排序策略。
pub(crate) fn rank_candidates(
    nodes: impl IntoIterator<Item = Node>,
    target_id: &str,
    limit: usize,
) -> Vec<(Node, String)> {
    let mut candidates: Vec<(Node, f64, &str)> = nodes
        .into_iter()
        .filter_map(|node| {
            candidate_score(&node, target_id).map(|(score, reason)| (node, score, reason))
        })
        .collect();

    candidates.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.id.cmp(&right.0.id))
    });

    candidates
        .into_iter()
        .take(limit)
        .map(|(node, _, reason)| (node, reason.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::rank_candidates;
    use crate::graph::{Node, NodeType};

    fn node(id: &str, name: &str) -> Node {
        Node {
            id: id.to_string(),
            node_type: NodeType::Component,
            path: "page.spg".to_string(),
            name: name.to_string(),
            meta: None,
        }
    }

    #[test]
    fn rank_candidates_uses_one_numeric_suffix_rule() {
        let candidates = rank_candidates(
            vec![node("comp:page|input12", "input12")],
            "comp:page|input1",
            5,
        );

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].1, "numbered suffix variant");
    }

    #[test]
    fn rank_candidates_has_stable_tie_order() {
        let candidates = rank_candidates(
            vec![node("model:user_b", "user"), node("model:user_a", "user")],
            "user",
            5,
        );

        assert_eq!(
            candidates
                .iter()
                .map(|(candidate, _)| candidate.id.as_str())
                .collect::<Vec<_>>(),
            vec!["model:user_a", "model:user_b"]
        );
    }
}
