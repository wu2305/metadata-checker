use serde_json::json;

/// 构建截断数组结构
///
/// 输出: { total_count, shown_count, truncated, remaining_count, items: [...] }
pub fn truncated_array(items: &[serde_json::Value], limit: usize) -> serde_json::Value {
    truncated_array_with_total(items, limit, items.len())
}

/// 使用完整集合计数包装已裁剪的缓存条目，保持与未裁剪数组相同的截断信封。
pub(crate) fn truncated_array_with_total(
    items: &[serde_json::Value],
    limit: usize,
    total: usize,
) -> serde_json::Value {
    let shown = items.len().min(limit);
    let remaining = total.saturating_sub(shown);
    json!({
        "total_count": total,
        "shown_count": shown,
        "truncated": remaining > 0,
        "remaining_count": remaining,
        "items": items.iter().take(shown).cloned().collect::<Vec<_>>(),
    })
}

/// 构建 evidence 摘要
///
/// 统计 total、shown、confidence 分布、是否有 graph-derived、是否被采样
pub fn evidence_summary(evidence: &[super::Evidence], shown: usize) -> serde_json::Value {
    let total = evidence.len();
    let mut high = 0usize;
    let mut medium = 0usize;
    let mut low = 0usize;
    let mut source_files = std::collections::HashSet::new();
    let mut has_graph_derived = false;

    for ev in evidence {
        match ev.confidence {
            super::Confidence::High => high += 1,
            super::Confidence::Medium => medium += 1,
            super::Confidence::Low => low += 1,
        }
        if let Some(ref sf) = ev.source_file {
            source_files.insert(sf.clone());
        }
        if ev.json_path.as_deref() == Some("<graph-edge-derived>") {
            has_graph_derived = true;
        }
    }

    json!({
        "total_count": total,
        "shown_count": shown.min(total),
        "sampled": shown < total,
        "confidence_counts": {
            "high": high,
            "medium": medium,
            "low": low,
        },
        "source_file_count": source_files.len(),
        "has_graph_derived": has_graph_derived,
    })
}

/// 构建 key_findings 数组
///
/// 从 summary 和 diagnostics 中提取关键发现
pub fn build_key_findings(
    summary: &serde_json::Value,
    diagnostics: &[super::Diagnostic],
) -> Vec<serde_json::Value> {
    let mut findings = Vec::new();

    // 从 summary 提取关键计数
    if let Some(what) = summary.get("what_is_it").and_then(|v| v.as_str()) {
        findings.push(json!({
            "claim": what,
            "category": "summary",
            "evidence_level": "high",
        }));
    }

    // 提取 risk 诊断
    for diag in diagnostics.iter().filter(|d| {
        matches!(
            d.severity,
            super::DiagnosticSeverity::Error | super::DiagnosticSeverity::Warning
        )
    }) {
        findings.push(json!({
            "claim": diag.message.clone(),
            "category": "risk",
            "evidence_level": "high",
            "code": diag.code.clone(),
        }));
    }

    // 提取 info 诊断中的截断提示
    for diag in diagnostics
        .iter()
        .filter(|d| d.code == "OUTPUT_TRUNCATED" || d.code == "EVIDENCE_SAMPLED")
    {
        findings.push(json!({
            "claim": diag.message.clone(),
            "category": "truncation",
            "evidence_level": "medium",
            "code": diag.code.clone(),
        }));
    }

    findings
}

/// 构建 brief 模式的 details
///
/// 将长数组替换为截断结构，保留原始字段名
pub fn brief_details(
    details: &serde_json::Value,
    limits: &std::collections::HashMap<String, usize>,
) -> serde_json::Value {
    let mut out = details.clone();
    if let Some(obj) = out.as_object_mut() {
        for (key, limit) in limits {
            if let Some(arr) = obj.get(key).and_then(|v| v.as_array()) {
                obj.insert(key.clone(), truncated_array(arr, *limit));
            }
        }
    }
    out
}
