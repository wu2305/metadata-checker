use metadata_checker::output::AiOutput;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Mutex;

static CLI_LOCK: Mutex<()> = Mutex::new(());

/// ============================================================
/// ============================================================
/// 读取 snapshot_cases.json
fn load_snapshot_cases() -> Vec<SnapshotCase> {
    let path = PathBuf::from("tests/fixtures/corpus/snapshots/snapshot_cases.json");
    let content = std::fs::read_to_string(&path).expect("snapshot_cases.json must exist");
    let val: serde_json::Value =
        serde_json::from_str(&content).expect("snapshot_cases.json must be valid JSON");
    let cases = val["cases"].as_array().expect("cases must be array");
    cases
        .iter()
        .map(|c| SnapshotCase {
            case_id: c["case_id"].as_str().unwrap().to_string(),
            command_kind: c["command_kind"].as_str().unwrap().to_string(),
            target: c["target"].as_str().unwrap().to_string(),
            budget: c["budget"].as_str().map(|s| s.to_string()),
            snapshot_path: c["snapshot_path"].as_str().unwrap().to_string(),
        })
        .collect()
}

#[derive(Debug, Clone)]
struct SnapshotCase {
    case_id: String,
    command_kind: String,
    target: String,
    budget: Option<String>,
    snapshot_path: String,
}

/// 运行 CLI 命令并返回 stdout
fn run_cli(args: &[&str]) -> String {
    let _guard = CLI_LOCK.lock().unwrap();
    let bin = std::env::current_dir()
        .unwrap()
        .join("target/debug/metadata-checker");
    let cmd_output = std::process::Command::new(&bin)
        .args(args)
        .output()
        .expect("Failed to run metadata-checker binary");
    String::from_utf8(cmd_output.stdout).expect("Invalid UTF-8")
}

/// 将 AiOutput 规范化为稳定的 snapshot 格式
fn normalize_ai_output(output: &AiOutput) -> serde_json::Value {
    let mut details_counts = serde_json::Map::new();
    if let Some(ref d) = output.details
        && let Some(obj) = d.as_object()
    {
        // Count arrays
        for key in [
            "action_flows",
            "entrypoints",
            "write_targets",
            "data_sources",
            "visibility_rules",
            "navigation",
            "page_inputs",
            "reads",
            "writes",
            "triggered_by",
            "affects",
            "lineage",
            "upstream",
            "downstream",
            "related_actions",
            "related_models",
            "related_pages",
            "related_components",
            "internal_nodes",
            "inputs",
            "outputs",
        ] {
            if let Some(arr) = obj.get(key).and_then(|v| v.as_array()) {
                details_counts.insert(format!("{}_count", key), json!(arr.len()));
            }
        }
        // Special: internal_topology nodes/edges count
        if let Some(topo) = obj.get("internal_topology").and_then(|v| v.as_object()) {
            let node_count = topo
                .get("nodes")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let edge_count = topo
                .get("edges")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            details_counts.insert(
                "internal_topology_nodes_count".to_string(),
                json!(node_count),
            );
            details_counts.insert(
                "internal_topology_edges_count".to_string(),
                json!(edge_count),
            );
        }
    }

    // Summary: keep keys + stable scalar fields, remove volatile strings
    let summary_filtered = if let Some(obj) = output.summary.as_object() {
        let mut filtered = serde_json::Map::new();
        for (k, v) in obj {
            // Keep scalar fields (numbers, booleans, short strings <= 100 chars)
            // and known stable enum fields
            let keep = match v {
                serde_json::Value::Number(_) | serde_json::Value::Bool(_) => true,
                serde_json::Value::String(s) => {
                    k == "page_role"
                        || k == "importance"
                        || k == "action_category"
                        || s.len() <= 100
                }
                serde_json::Value::Array(arr) => arr.len() <= 5, // small arrays only
                _ => false,
            };
            if keep {
                filtered.insert(k.clone(), v.clone());
            }
        }
        json!(filtered)
    } else {
        output.summary.clone()
    };

    let mut diagnostics_sorted: Vec<_> = output
        .diagnostics
        .iter()
        .map(|d| {
            json!({
                "code": &d.code,
                "severity": format!("{:?}", d.severity),
            })
        })
        .collect();
    diagnostics_sorted.sort_by(|a, b| {
        let a_code = a.get("code").and_then(|v| v.as_str()).unwrap_or("");
        let b_code = b.get("code").and_then(|v| v.as_str()).unwrap_or("");
        a_code.cmp(b_code)
    });

    let mut evidence_sorted: Vec<_> = output
        .evidence
        .iter()
        .map(|e| {
            json!({
                "claim": &e.claim,
                "edge_type": &e.edge_type,
                "confidence": format!("{:?}", e.confidence),
            })
        })
        .collect();
    evidence_sorted.sort_by(|a, b| {
        let a_key = (
            a.get("claim").and_then(|v| v.as_str()).unwrap_or(""),
            a.get("edge_type").and_then(|v| v.as_str()).unwrap_or(""),
        );
        let b_key = (
            b.get("claim").and_then(|v| v.as_str()).unwrap_or(""),
            b.get("edge_type").and_then(|v| v.as_str()).unwrap_or(""),
        );
        a_key.cmp(&b_key)
    });

    json!({
        "schema_version": &output.schema_version,
        "kind": format!("{:?}", output.kind),
        "query_target": output.query_target,
        "summary": summary_filtered,
        "diagnostics": diagnostics_sorted,
        "evidence": evidence_sorted,
        "details_counts": details_counts,
    })
}

/// 执行一个 case 并返回规范化输出
fn execute_case(case: &SnapshotCase) -> serde_json::Value {
    let project_dir = "tests/fixtures/test_project";
    let args: Vec<String> = match case.command_kind.as_str() {
        "query-page-logic" => vec![
            "--project-dir".to_string(),
            project_dir.to_string(),
            "--query-page-logic".to_string(),
            case.target.clone(),
        ],
        "explain" => vec![
            "--project-dir".to_string(),
            project_dir.to_string(),
            "--explain".to_string(),
            case.target.clone(),
        ],
        "context" => {
            let mut a = vec![
                "--project-dir".to_string(),
                project_dir.to_string(),
                "--context".to_string(),
                case.target.clone(),
            ];
            if let Some(ref b) = case.budget {
                a.push("--budget".to_string());
                a.push(b.clone());
            }
            a
        }
        "query-dataflow" => vec![
            "--project-dir".to_string(),
            project_dir.to_string(),
            "--query-dataflow".to_string(),
            case.target.clone(),
        ],
        _ => panic!("unknown command_kind: {}", case.command_kind),
    };

    let raw_output = run_cli(&args.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    let ai: AiOutput = serde_json::from_str(&raw_output).unwrap_or_else(|e| {
        panic!(
            "case {} output must be AiOutput: {}\nraw: {}",
            case.case_id,
            e,
            &raw_output[..raw_output.len().min(200)]
        )
    });
    normalize_ai_output(&ai)
}

/// 为每个 case 生成或验证 snapshot
#[test]
fn test_snapshot_cases() {
    let update_mode = std::env::var("UPDATE_CORPUS_SNAPSHOTS")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);

    // P0: 确保图数据库已构建，不依赖被 git 忽略的 .metadata-checker.graphdb
    let project_dir = PathBuf::from("tests/fixtures/test_project");
    let db_path = PathBuf::from("tests/fixtures/test_project/.metadata-checker.graphdb");
    if !db_path.exists() || update_mode {
        metadata_checker::scanner::scan_project(&project_dir, &db_path)
            .expect("scan_project must succeed on test_project");
    }

    let cases = load_snapshot_cases();
    assert!(!cases.is_empty(), "snapshot_cases.json must have cases");

    let mut failures = Vec::new();
    for case in &cases {
        let actual = execute_case(case);
        let snapshot_path = PathBuf::from(&case.snapshot_path);

        if update_mode {
            let content = serde_json::to_string_pretty(&actual).unwrap();
            std::fs::write(&snapshot_path, content).unwrap_or_else(|e| {
                panic!(
                    "failed to write snapshot {}: {}",
                    snapshot_path.display(),
                    e
                )
            });
            println!("Updated snapshot: {}", snapshot_path.display());
            continue;
        }

        if !snapshot_path.exists() {
            failures.push(format!(
                "MISSING snapshot for {}: {}. Run with UPDATE_CORPUS_SNAPSHOTS=1 to generate.",
                case.case_id,
                snapshot_path.display()
            ));
            continue;
        }

        let expected_content = std::fs::read_to_string(&snapshot_path)
            .unwrap_or_else(|_| panic!("snapshot must be readable: {}", snapshot_path.display()));
        let expected: serde_json::Value = serde_json::from_str(&expected_content)
            .unwrap_or_else(|_| panic!("snapshot must be valid JSON: {}", snapshot_path.display()));

        if expected != actual {
            let diff = format!(
                "SNAPSHOT MISMATCH for {}\n  expected: {}\n  actual:   {}",
                case.case_id,
                serde_json::to_string(&expected).unwrap(),
                serde_json::to_string(&actual).unwrap()
            );
            failures.push(diff);
        }
    }

    if !failures.is_empty() {
        for f in &failures {
            eprintln!("{}", f);
        }
        panic!(
            "{} snapshot case(s) failed. Set UPDATE_CORPUS_SNAPSHOTS=1 to regenerate.",
            failures.len()
        );
    }
}
