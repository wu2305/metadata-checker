//! M58.3 PR1 refix：`Diagnostic` 序列化只输出 `sample_location`，不再同时输出 `location`。
//!
//! 此前手写 `Serialize` 同时输出 `sample_location` 与 `location` 两个键，
//! 而 `Deserialize` 侧两者映射到同一字段（rename + alias），
//! 导致自身序列化结果无法 round-trip（duplicate field）。
//! 本测试钉住：序列化仅含 `sample_location`、round-trip 成功、旧名 `location` 别名仍兼容、双名冲突报错。

use metadata_checker::diagnostics::{CODE_GRAPH_DB_NODE_DECODE_FAILED, envelope_diagnostic};
use metadata_checker::output::{Diagnostic, Location};

/// 构造一条带完整信封字段的诊断
fn sample_diag() -> Diagnostic {
    envelope_diagnostic(
        CODE_GRAPH_DB_NODE_DECODE_FAILED,
        2,
        Location {
            source_file: Some("app/a.spg".to_string()),
            node_id: Some("n1".to_string()),
            json_path: Some("$.canvas".to_string()),
        },
        "test message",
    )
}

/// 序列化输出必须包含 `sample_location`，且不再包含旧名 `location`
#[test]
fn refix_serialize_emits_sample_location_only() -> anyhow::Result<()> {
    let value = serde_json::to_value(sample_diag())?;
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("诊断序列化结果必须是对象：{value}"));
    assert!(
        object.contains_key("sample_location"),
        "缺少 sample_location：{value}"
    );
    assert_eq!(
        object.contains_key("location"),
        false,
        "序列化不得再输出旧名 location：{value}"
    );
    Ok(())
}

/// 自身序列化结果可以无损反序列化回来，location 字段值保留
#[test]
fn refix_serialized_json_roundtrips() -> anyhow::Result<()> {
    let value = serde_json::to_value(sample_diag())?;
    let back: Diagnostic = serde_json::from_value(value)?;
    assert_eq!(back.location.source_file.as_deref(), Some("app/a.spg"));
    assert_eq!(back.location.node_id.as_deref(), Some("n1"));
    assert_eq!(back.location.json_path.as_deref(), Some("$.canvas"));
    assert_eq!(back.count, Some(2));
    Ok(())
}

/// 兼容性：只含旧名 `location`（无 `sample_location`）的 JSON 仍能反序列化（alias 生效）
#[test]
fn refix_legacy_location_alias_still_deserializes() -> anyhow::Result<()> {
    let legacy = serde_json::json!({
        "severity": "warning",
        "code": "LEGACY_CODE",
        "message": "legacy",
        "location": {
            "source_file": "legacy.spg",
            "node_id": null,
            "json_path": null
        }
    });
    let diag: Diagnostic = serde_json::from_value(legacy)?;
    assert_eq!(diag.code, "LEGACY_CODE");
    assert_eq!(diag.location.source_file.as_deref(), Some("legacy.spg"));
    assert_eq!(diag.location.node_id, None);
    Ok(())
}

/// 同时含 `sample_location` 与 `location` 的 JSON 必须报 duplicate field 错误
#[test]
fn refix_both_names_rejected_as_duplicate_field() {
    let both = serde_json::json!({
        "severity": "warning",
        "code": "DUP_CODE",
        "message": "dup",
        "sample_location": {
            "source_file": "a.spg",
            "node_id": null,
            "json_path": null
        },
        "location": {
            "source_file": "b.spg",
            "node_id": null,
            "json_path": null
        }
    });
    let result: Result<Diagnostic, _> = serde_json::from_value(both);
    assert!(
        result.is_err(),
        "双名必须报 duplicate field，实际：{result:?}"
    );
}
