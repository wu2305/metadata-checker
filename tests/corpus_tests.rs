#![cfg(feature = "cli-local")]

use metadata_checker::parser::parse_file;
use metadata_checker::superpage::parse_superpage;
use std::collections::HashSet;
use std::path::PathBuf;

/// ============================================================
/// M9-B Regression Corpus 测试
/// ============================================================
fn load_selection() -> serde_json::Value {
    let path = PathBuf::from("tests/fixtures/corpus/selection.json");
    let content = std::fs::read_to_string(&path).expect("selection.json must exist");
    serde_json::from_str(&content).expect("selection.json must be valid JSON")
}

/// M9-B-1: selection.json schema 完整性
#[test]
fn test_selection_schema_complete() {
    let selection = load_selection();
    assert!(selection.get("schema_version").is_some());
    assert!(selection.get("scope").is_some());
    assert!(selection.get("total_count").is_some());
    let entries = selection
        .get("entries")
        .and_then(|v| v.as_array())
        .expect("entries must be array");
    let total = selection
        .get("total_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert_eq!(
        entries.len() as u64,
        total,
        "total_count must match entries length"
    );

    // Verify required fields for each entry
    let required_fields = [
        "stable_id",
        "entry_type",
        "source_path",
        "copied_path",
        "file_type",
        "size_bytes",
        "coverage_tags",
        "selection_reason",
        "risk_points",
        "expected_commands",
    ];
    for entry in entries {
        for field in &required_fields {
            assert!(
                entry.get(field).is_some(),
                "entry {} missing field {}",
                entry
                    .get("stable_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?"),
                field
            );
        }
    }
}

/// M9-B-2: copied 样本文件存在
#[test]
fn test_copied_samples_exist() {
    let selection = load_selection();
    let entries = selection["entries"].as_array().unwrap();
    for entry in entries {
        if entry["entry_type"] != "copied" {
            continue;
        }
        let copied_path = entry["copied_path"].as_str().unwrap();
        let path = PathBuf::from(copied_path);
        assert!(path.exists(), "copied sample must exist: {}", copied_path);
    }
}

/// M9-B-3: referenced 样本文件存在
#[test]
fn test_referenced_samples_exist() {
    let selection = load_selection();
    let entries = selection["entries"].as_array().unwrap();
    for entry in entries {
        if entry["entry_type"] != "referenced" {
            continue;
        }
        let copied_path = entry["copied_path"].as_str().unwrap();
        let path = PathBuf::from(copied_path);
        assert!(
            path.exists(),
            "referenced fixture must exist: {}",
            copied_path
        );
    }
}

/// M9-B-4: source_path 不得引用 /Users/wuhaocheng/Downloads/bi
#[test]
fn test_source_path_not_bi_downloads() {
    let selection = load_selection();
    let entries = selection["entries"].as_array().unwrap();
    for entry in entries {
        let source = entry["source_path"].as_str().unwrap_or("");
        assert!(
            !source.contains("/Downloads/bi"),
            "source_path must not reference Downloads/bi: {}",
            source
        );
    }
}

/// M9-B-5: coverage_tags 覆盖 M9-B 要求的风险点
#[test]
fn test_coverage_tags_meet_requirements() {
    let selection = load_selection();
    let entries = selection["entries"].as_array().unwrap();

    let required_tags: HashSet<&str> = [
        "actions",
        "readonly_page",
        "submit_write",
        "dialog",
        "visibility",
        "DataFlow",
        "conditionExp",
        "waitPrev",
        "link_param",
        "duplicate_action_id",
        "embedded_page",
        "refresh_action",
    ]
    .iter()
    .cloned()
    .collect();

    let mut covered: HashSet<String> = HashSet::new();
    for entry in entries {
        let tags_arr = entry["coverage_tags"].as_array().unwrap_or(&vec![]).clone();
        let tags = tags_arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()));
        for tag in tags {
            covered.insert(tag);
        }
    }

    // Report missing tags as info, not hard failure (some may be covered by referenced fixtures)
    let missing: Vec<&str> = required_tags
        .iter()
        .filter(|tag| !covered.contains(**tag))
        .copied()
        .collect();
    println!("Covered tags: {:?}", covered);
    println!(
        "Missing required tags (acceptable if covered by existing tests): {:?}",
        missing
    );
    // All 12 tags should be covered by the union of corpus + existing test fixtures
    assert!(
        missing.is_empty(),
        "Missing coverage tags in corpus selection: {:?}",
        missing
    );
}

/// M9-B-6: 每个 .spg copied 样本可解析（parse_superpage）
#[test]
fn test_copied_spg_parseable() {
    let selection = load_selection();
    let entries = selection["entries"].as_array().unwrap();
    for entry in entries {
        if entry["entry_type"] != "copied" {
            continue;
        }
        let file_type = entry["file_type"].as_str().unwrap();
        if file_type != "spg" {
            continue;
        }
        let copied_path = entry["copied_path"].as_str().unwrap();
        let path = PathBuf::from(copied_path);
        let meta = parse_superpage(&path)
            .unwrap_or_else(|_| panic!("copied .spg must parse: {}", copied_path));
        // At least canvas or components should be present, or it's a valid empty page
        assert!(
            !meta.components.is_empty() || meta.expressions.is_empty(),
            "parsed .spg should have components or be valid empty: {}",
            copied_path
        );
    }
}

/// M9-B-7: 每个 .tbl copied 样本可解析（parser::parse_file）
#[test]
fn test_copied_tbl_parseable() {
    let selection = load_selection();
    let entries = selection["entries"].as_array().unwrap();
    for entry in entries {
        if entry["entry_type"] != "copied" {
            continue;
        }
        let file_type = entry["file_type"].as_str().unwrap();
        if file_type != "tbl" {
            continue;
        }
        let copied_path = entry["copied_path"].as_str().unwrap();
        let path = PathBuf::from(copied_path);
        let parsed =
            parse_file(&path).unwrap_or_else(|_| panic!("copied .tbl must parse: {}", copied_path));
        // .tbl files are parsed as PageMetadata with raw JSON preserved
        assert!(
            parsed.raw.is_object(),
            "parsed .tbl should have raw JSON object: {}",
            copied_path
        );
    }
}

/// M9-B-8: referenced test_project fixture 仍可解析
#[test]
fn test_referenced_fixtures_parseable() {
    let selection = load_selection();
    let entries = selection["entries"].as_array().unwrap();
    for entry in entries {
        if entry["entry_type"] != "referenced" {
            continue;
        }
        let copied_path = entry["copied_path"].as_str().unwrap();
        let path = PathBuf::from(copied_path);
        let file_type = entry["file_type"].as_str().unwrap();
        match file_type {
            "spg" => {
                let _meta = parse_superpage(&path)
                    .unwrap_or_else(|_| panic!("referenced .spg must parse: {}", copied_path));
            }
            "tbl" => {
                let parsed = parse_file(&path)
                    .unwrap_or_else(|_| panic!("referenced .tbl must parse: {}", copied_path));
                assert!(
                    parsed.raw.is_object(),
                    "referenced .tbl should parse with raw JSON: {}",
                    copied_path
                );
            }
            _ => {}
        }
    }
}

/// M9-B-9: 总 corpus 体积受控（<50KB）
#[test]
fn test_corpus_total_size_controlled() {
    let selection = load_selection();
    let entries = selection["entries"].as_array().unwrap();
    let mut total: u64 = 0;
    for entry in entries {
        let size = entry["size_bytes"].as_u64().unwrap_or(0);
        total += size;
        assert!(
            size < 10 * 1024,
            "individual sample should be <10KB: {} is {} bytes",
            entry["stable_id"].as_str().unwrap_or("?"),
            size
        );
    }
    assert!(
        total < 50 * 1024,
        "total corpus size should be <50KB: {} bytes",
        total
    );
}

/// M9-B-10: selection.json 中 entry_type 只包含 copied/referenced
#[test]
fn test_entry_type_enum_valid() {
    let selection = load_selection();
    let entries = selection["entries"].as_array().unwrap();
    for entry in entries {
        let et = entry["entry_type"].as_str().unwrap_or("");
        assert!(
            et == "copied" || et == "referenced",
            "entry_type must be copied or referenced: got {}",
            et
        );
    }
}

/// M9-B-11: 对 test_project 执行 scanner build-graph，验证不 panic 且图节点/边合理
#[test]
fn test_corpus_build_graph() {
    let project_dir = PathBuf::from("tests/fixtures/test_project");
    let db_path = PathBuf::from("tests/fixtures/corpus/test_graph.db");

    // 清理旧图数据库（避免增量逻辑干扰）
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_dir_all(db_path.with_extension("db.lock"));

    metadata_checker::scanner::scan_project(&project_dir, &db_path)
        .expect("scan_project must succeed on test_project corpus");

    let graph = metadata_checker::graph::GraphDB::open(&db_path).expect("GraphDB must open");

    // 图应该有节点（至少一些 page/model/component 节点）
    assert!(
        !graph.node_indices.is_empty(),
        "test_project graph should have nodes, got 0"
    );

    // 验证关键 fixture 对应的节点存在于图中
    let expected_nodes = [
        "page:app/actions_test.spg",
        "model:df_a",
        "model:df_b",
        "model:physical_x",
    ];
    for node_id in &expected_nodes {
        assert!(
            graph.get_node(node_id).is_some(),
            "graph must contain node: {}",
            node_id
        );
    }

    // 验证 actions_test.spg 的 button1 组件有 action 边
    let comp_id = "comp:app/actions_test.spg|button1";
    if let Some((outgoing, _)) = graph.get_node_edges(comp_id) {
        let has_action_edge = outgoing.iter().any(|(_n, e)| {
            matches!(
                e.edge_type,
                metadata_checker::graph::EdgeType::Triggers
                    | metadata_checker::graph::EdgeType::ActionWrites
            )
        });
        assert!(has_action_edge, "button1 should have action edges in graph");
    } else {
        panic!("button1 node should exist in graph");
    }

    // 验证 DataFlow 链式关系：df_a -> physical_x
    if let Some((outgoing, _)) = graph.get_node_edges("model:df_a") {
        let has_output_edge = outgoing
            .iter()
            .any(|(_n, e)| matches!(e.edge_type, metadata_checker::graph::EdgeType::OutputsTo));
        assert!(has_output_edge, "df_a should have OutputsTo edge");
    } else {
        panic!("df_a node should exist in graph");
    }

    // 清理图数据库
    let _ = std::fs::remove_file(&db_path);
}
