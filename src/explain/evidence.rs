/// 从详情项中提取字符串字段。
fn detail_str<'a>(item: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    item.get(key).and_then(|v| v.as_str())
}

/// 将 reads/writes/triggered_by/affects 之类的详情项转换为 evidence。
pub(super) fn push_relation_evidence(
    output: &mut crate::output::AiOutput,
    relation: &str,
    items: &[serde_json::Value],
) {
    for item in items.iter().take(5) {
        let node_id = detail_str(item, "id")
            .or_else(|| detail_str(item, "from"))
            .or_else(|| detail_str(item, "to"));
        let edge_type = detail_str(item, "edge_type")
            .or_else(|| detail_str(item, "type"))
            .unwrap_or("unknown");
        let raw_expr = detail_str(item, "raw_expr").or_else(|| detail_str(item, "field_path"));
        let source_file = detail_str(item, "source_file");
        let json_path = detail_str(item, "json_path");
        let json_path_str = json_path.unwrap_or("<graph-edge-derived>");

        let has_real_path = json_path.is_some() && json_path != Some("<graph-edge-derived>");
        let (confidence, reason) = if has_real_path && source_file.is_some() {
            (
                crate::output::Confidence::High,
                "Relation extracted from graph edge with source metadata and json_path",
            )
        } else if source_file.is_some() {
            (
                crate::output::Confidence::Medium,
                "Relation extracted from graph edge; raw json_path is not available",
            )
        } else {
            (
                crate::output::Confidence::Low,
                "Relation inferred from graph traversal without direct JSON path",
            )
        };

        let claim = if let Some(id) = node_id {
            format!("{} relation on {}", relation, id)
        } else {
            format!("{} relation (target unidentified)", relation)
        };

        let mut ev = crate::output::Evidence::new(claim, reason)
            .with_confidence(confidence)
            .with_edge_type(edge_type);
        if let Some(id) = node_id {
            ev = ev.with_node_id(id);
        }
        if let Some(expr) = raw_expr {
            ev = ev.with_raw_expr(expr);
        }
        ev = ev.with_json_path(json_path_str);
        if let Some(sf) = source_file {
            ev = ev.with_source_file(sf);
        }
        output.evidence.push(ev);

        if node_id.is_none() || source_file.is_none() {
            let mut diag = crate::diagnostics::envelope_diagnostic(
                "EVIDENCE_LOCATION_MISSING",
                1,
                crate::output::Location::new(),
                format!(
                    "Evidence for {} relation lacks node_id or source_file; confidence reduced",
                    relation
                ),
            );
            diag.severity = crate::output::DiagnosticSeverity::Info;
            diag.suggestion = Some("Verify graph edge metadata completeness".to_string());
            output.diagnostics.push(diag);
        }
    }
}

/// 将 lineage 中内嵌 evidence 提升为顶层 evidence。
pub(super) fn push_lineage_evidence(
    output: &mut crate::output::AiOutput,
    lineage: &[serde_json::Value],
) {
    for item in lineage.iter().take(5) {
        let target_field = detail_str(item, "target_field").unwrap_or("?");
        let evidence = item.get("evidence");
        let source_file = evidence
            .and_then(|e| e.get("source_file"))
            .and_then(|v| v.as_str());
        let node_id = evidence
            .and_then(|e| e.get("node_id"))
            .and_then(|v| v.as_str());
        let edge_type = evidence
            .and_then(|e| e.get("edge_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("Contains");
        let raw_expr = evidence
            .and_then(|e| e.get("raw_expr"))
            .and_then(|v| v.as_str())
            .or_else(|| detail_str(item, "source_expr"));
        let json_path = evidence
            .and_then(|e| e.get("json_path"))
            .and_then(|v| v.as_str());

        let is_graph_derived = node_id.is_none() || json_path.is_none() || raw_expr.is_none();
        let confidence = if is_graph_derived {
            crate::output::Confidence::Medium
        } else {
            match detail_str(item, "confidence") {
                Some("high") => crate::output::Confidence::High,
                Some("low") => crate::output::Confidence::Low,
                _ => crate::output::Confidence::Medium,
            }
        };

        let reason = if is_graph_derived {
            "Field lineage partially derived from graph traversal; some metadata missing"
        } else {
            "Field lineage derived from metadata/source expressions"
        };

        let mut ev = crate::output::Evidence::new(format!("Lineage for {}", target_field), reason)
            .with_confidence(confidence)
            .with_edge_type(edge_type);
        if let Some(id) = node_id {
            ev = ev.with_node_id(id);
        }
        if let Some(expr) = raw_expr {
            ev = ev.with_raw_expr(expr);
        }
        if let Some(path) = json_path {
            ev = ev.with_json_path(path);
        }
        if let Some(sf) = source_file {
            ev = ev.with_source_file(sf);
        }
        output.evidence.push(ev);
    }
}
