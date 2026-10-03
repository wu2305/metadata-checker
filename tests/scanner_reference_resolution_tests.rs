#![cfg(feature = "cli-local")]

//! 跨页引用解析与未落表数据流的回归测试（真实语料 autocrm 暴露的两个建图阻塞缺陷）。
//!
//! 固定的行为：
//! - `$TAPP:` 是**当前应用**目录，`$APP:` 是 `app/` 目录，`$ANA:` / `$DATA:` 是项目根的同级目录，
//!   相对路径做 `..` 归一；
//! - 解析不了的引用（未知前缀、绝对路径、越出项目根、目标不是 `.spg`、下标越界）
//!   **不建边也不造 Page 节点**，并各记一条 `SCANNER_UNRESOLVED_REFERENCE`；
//! - `dbTableName` 为空串（未落表数据流）不再使建图中止：模型照常入图并带 `landed: false`，
//!   没有输出边，也不产生任何诊断。

use metadata_checker::graph::{EdgeType, GraphDB, NodeType};
use metadata_checker::scanner::indexer::ProjectIndexer;
use metadata_checker::scanner::{ReferenceUnresolved, resolve_reference_target};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const CODE_UNRESOLVED: &str = "SCANNER_UNRESOLVED_REFERENCE";

fn unique_temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "metadata-checker-ref-resolution-{tag}-{}-{nanos}",
        std::process::id()
    ))
}

const CURRENT_PAGE: &str = "app/售后.app/工单/首页.spg";

/// 各前缀与相对路径的展开结果（对照真实语料确认过的语义）。
#[test]
fn resolver_expands_each_prefix_against_the_current_app() {
    let cases = [
        ("$TAPP:/预约/编辑.spg", "app/售后.app/预约/编辑.spg"),
        ("$APP:/销售.app/合同/审批.spg", "app/销售.app/合同/审批.spg"),
        ("../备份/旧页.spg", "app/售后.app/备份/旧页.spg"),
        ("./同级.spg", "app/售后.app/工单/同级.spg"),
        ("子页/详情.spg", "app/售后.app/工单/子页/详情.spg"),
        // 路径中段的 `..` 必须被消解，不能原样留在页面路径里
        ("ys/../任务/全部.spg", "app/售后.app/工单/任务/全部.spg"),
    ];
    for (reference, expected) in cases {
        assert_eq!(
            resolve_reference_target(CURRENT_PAGE, reference),
            Ok(expected.to_string()),
            "{reference}"
        );
    }
}

/// 扫描根上一层时（`xiaoshouyi/app/...`）同样能定位当前应用，并保留根前缀。
#[test]
fn resolver_keeps_the_project_root_prefix() {
    assert_eq!(
        resolve_reference_target("xiaoshouyi/app/售后.app/工单/首页.spg", "$TAPP:/a.spg"),
        Ok("xiaoshouyi/app/售后.app/a.spg".to_string())
    );
    assert_eq!(
        resolve_reference_target(
            "xiaoshouyi/app/售后.app/工单/首页.spg",
            "$APP:/销售.app/b.spg"
        ),
        Ok("xiaoshouyi/app/销售.app/b.spg".to_string())
    );
}

/// 解析不了的形态各有各的原因，且都不产出路径。
#[test]
fn resolver_reports_why_a_reference_cannot_resolve() {
    assert_eq!(
        resolve_reference_target(CURRENT_PAGE, "$ANA:/价审/政策.rpt"),
        Err(ReferenceUnresolved::NotAPage {
            target: "ana/价审/政策.rpt".to_string()
        }),
        "$ANA: 指向项目根同级的 ana/，目标是报表不是页面"
    );
    assert_eq!(
        resolve_reference_target(CURRENT_PAGE, "/sysdata/公共/页.spg"),
        Err(ReferenceUnresolved::AbsolutePath {
            reference: "/sysdata/公共/页.spg".to_string()
        })
    );
    assert_eq!(
        resolve_reference_target(CURRENT_PAGE, "$ICON:/logo.png"),
        Err(ReferenceUnresolved::UnknownPrefix {
            reference: "$ICON:/logo.png".to_string()
        })
    );
    assert_eq!(
        resolve_reference_target(CURRENT_PAGE, "../../../../逃逸.spg"),
        Err(ReferenceUnresolved::EscapesRoot {
            reference: "../../../../逃逸.spg".to_string()
        })
    );
    assert_eq!(
        resolve_reference_target("单文件.spg", "$TAPP:/a.spg"),
        Err(ReferenceUnresolved::NoAppRoot { prefix: "$TAPP:" }),
        "文件不在 app/<name>.app/ 之下时，$TAPP: 无从展开"
    );
    // Windows 盘符路径不能被当成相对路径拼到当前目录之后
    for drive in ["C:\\outside\\page.spg", "D:/outside/page.spg"] {
        assert_eq!(
            resolve_reference_target(CURRENT_PAGE, drive),
            Err(ReferenceUnresolved::AbsolutePath {
                reference: drive.to_string()
            }),
            "{drive}"
        );
    }
    assert_eq!(
        resolve_reference_target(CURRENT_PAGE, "附件/说明.docx"),
        Err(ReferenceUnresolved::NotAPage {
            target: "app/售后.app/工单/附件/说明.docx".to_string()
        })
    );
}

/// 一页里混合可解析与不可解析的引用：只有可解析的建边，其余各记一条诊断，且不出幽灵页面。
#[test]
fn unresolved_references_create_no_edge_no_ghost_page_and_one_record_each() -> anyhow::Result<()> {
    let project_dir = unique_temp_dir("spg");
    let page_dir = project_dir.join("app/售后.app/工单");
    std::fs::create_dir_all(&page_dir)?;
    let page = serde_json::json!({
        "referenceResources": [
            "$TAPP:/预约/编辑.spg",
            "$ICON:/logo.png",
            "/sysdata/公共/页.spg",
            "$ANA:/价审/政策.rpt"
        ],
        "canvas": {"id": "canvas", "type": "canvas", "components": [
            {"id": "embed_ok", "type": "embedsuperpage", "resPath": 0},
            {"id": "embed_icon", "type": "embedsuperpage", "resPath": 1},
            {"id": "btn", "type": "button", "actions": [
                {"id": "go_abs", "actionType": "link", "triggerType": "click",
                 "targetType": "app", "path": 2},
                {"id": "go_rpt", "actionType": "link", "triggerType": "click",
                 "targetType": "app", "path": 3},
                {"id": "go_missing_index", "actionType": "link", "triggerType": "click",
                 "targetType": "app", "path": 9}
            ]}
        ]}
    });
    std::fs::write(page_dir.join("首页.spg"), serde_json::to_string(&page)?)?;
    let db_path = project_dir.join("graph.db");
    ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;

    let graph = GraphDB::open(&db_path)?;
    let mut page_paths: Vec<String> = graph
        .graph
        .node_weights()
        .filter(|node| node.node_type == NodeType::Page)
        .map(|node| node.path.clone())
        .collect();
    page_paths.sort();
    assert_eq!(
        page_paths,
        vec![
            "app/售后.app/工单/首页.spg".to_string(),
            "app/售后.app/预约/编辑.spg".to_string()
        ],
        "只有本页和唯一可解析的目标；其余引用不得造出页面节点"
    );
    let embed_edges = graph
        .graph
        .edge_weights()
        .filter(|edge| edge.edge_type == EdgeType::EmbedsPage)
        .count();
    assert_eq!(embed_edges, 1, "只有 embed_ok 能建 EmbedsPage 边");
    let navigates = graph
        .graph
        .edge_weights()
        .filter(|edge| edge.edge_type == EdgeType::ActionNavigates)
        .count();
    assert_eq!(
        navigates, 0,
        "三个 link 动作都解析不了，不应有 ActionNavigates 边"
    );

    let entries = graph.load_scanner_diagnostic_entries()?;
    let report = ProjectIndexer::merge_scanner_occurrence_entries(&entries)?;
    let mut records: Vec<(String, String)> = report
        .occurrences
        .iter()
        .filter(|record| record.code == CODE_UNRESOLVED)
        .map(|record| {
            (
                record.location.node_id.clone().unwrap_or_default(),
                record.location.json_path.clone().unwrap_or_default(),
            )
        })
        .collect();
    records.sort();
    let page_key = "app/售后.app/工单/首页.spg";
    assert_eq!(
        records,
        vec![
            (
                format!("action:{page_key}|btn|go_abs"),
                "referenceResources[2]".to_string()
            ),
            (
                format!("action:{page_key}|btn|go_missing_index"),
                "referenceResources[9]".to_string()
            ),
            (
                format!("action:{page_key}|btn|go_rpt"),
                "referenceResources[3]".to_string()
            ),
            (
                format!("comp:{page_key}|embed_icon"),
                "referenceResources[1]".to_string()
            ),
        ],
        "每处不可解析的引用各一条记录，位置指向 referenceResources 下标"
    );
    let aggregate = ProjectIndexer::merge_scanner_diagnostic_entries(&entries)?;
    let envelope = aggregate
        .iter()
        .find(|diagnostic| diagnostic.code == CODE_UNRESOLVED)
        .expect("聚合信封应包含未解析引用");
    assert_eq!(envelope.count, Some(4), "聚合计数与逐次记录一致");

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}

/// 未落表数据流（`dbTableName` 为空串）：建图成功，模型带 `landed: false`，没有输出边，
/// 也没有因它产生的诊断；同批里正常落表的表照旧有输出边。
#[test]
fn unlanded_dataflow_builds_with_landed_false_and_no_output_edge() -> anyhow::Result<()> {
    let project_dir = unique_temp_dir("tbl");
    std::fs::create_dir_all(&project_dir)?;
    let flow = |db_table_name: &str| {
        serde_json::json!({
            "properties": {"dbTableName": db_table_name},
            "dimensions": [{"name": "金额"}],
            "dataFlow": {"nodes": {}}
        })
    };
    std::fs::write(
        project_dir.join("即时取数.tbl"),
        serde_json::to_string(&flow(""))?,
    )?;
    std::fs::write(
        project_dir.join("落表流.tbl"),
        serde_json::to_string(&flow("t_landed"))?,
    )?;
    let db_path = project_dir.join("graph.db");
    ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;

    let graph = GraphDB::open(&db_path)?;
    let unlanded = graph
        .get_node("model:即时取数")
        .expect("未落表数据流本身必须入图");
    let meta = unlanded.meta.clone().expect("模型节点应有 meta");
    assert_eq!(meta["landed"], serde_json::json!(false), "{meta}");
    let landed = graph.get_node("model:落表流").expect("落表流入图");
    assert_eq!(
        landed.meta.as_ref().and_then(|m| m.get("landed")),
        None,
        "只有显式空串才标 landed:false"
    );
    assert!(graph.get_node("model:").is_none(), "不得出现空名物理表节点");
    let outputs: Vec<(String, String)> = graph
        .graph
        .raw_edges()
        .iter()
        .filter(|edge| edge.weight.edge_type == EdgeType::OutputsTo)
        .map(|edge| (edge.weight.from.clone(), edge.weight.to.clone()))
        .collect();
    assert_eq!(
        outputs,
        vec![("model:落表流".to_string(), "model:t_landed".to_string())]
    );

    let entries = graph.load_scanner_diagnostic_entries()?;
    let report = ProjectIndexer::merge_scanner_occurrence_entries(&entries)?;
    assert_eq!(report.occurrences.len(), 0, "{report:?}");

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}

/// `referenceResources` 缺失时，页面里仍引用下标：每处都越界，建图跳过，诊断必须照记。
#[test]
fn references_without_a_resource_list_are_reported_as_out_of_range() -> anyhow::Result<()> {
    let project_dir = unique_temp_dir("no-list");
    let page_dir = project_dir.join("app/售后.app/工单");
    std::fs::create_dir_all(&page_dir)?;
    let page = serde_json::json!({
        "canvas": {"id": "canvas", "type": "canvas", "components": [
            {"id": "embed_orphan", "type": "embedsuperpage", "resPath": 0}
        ]}
    });
    std::fs::write(page_dir.join("首页.spg"), serde_json::to_string(&page)?)?;
    let db_path = project_dir.join("graph.db");
    ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;

    let entries = GraphDB::open(&db_path)?.load_scanner_diagnostic_entries()?;
    let report = ProjectIndexer::merge_scanner_occurrence_entries(&entries)?;
    let records: Vec<_> = report
        .occurrences
        .iter()
        .filter(|record| record.code == CODE_UNRESOLVED)
        .collect();
    assert_eq!(records.len(), 1, "{report:?}");
    assert_eq!(
        records[0].location.node_id.as_deref(),
        Some("comp:app/售后.app/工单/首页.spg|embed_orphan")
    );
    assert!(
        records[0]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("out of range")),
        "{records:?}"
    );

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}

/// 数据流有输入节点时，建图会重建并再次写入模型 meta；`landed: false` 不能在那一步丢失。
#[test]
fn landed_flag_survives_dataflow_metadata_enrichment() -> anyhow::Result<()> {
    let project_dir = unique_temp_dir("tbl-nodes");
    std::fs::create_dir_all(&project_dir)?;
    let flow = serde_json::json!({
        "properties": {"dbTableName": ""},
        "dimensions": [{"name": "金额"}],
        "dataFlow": {"nodes": {
            "n1": {"moduleTablePath": "$DATA:/源/订单.tbl"}
        }}
    });
    std::fs::write(
        project_dir.join("即时取数.tbl"),
        serde_json::to_string(&flow)?,
    )?;
    let db_path = project_dir.join("graph.db");
    ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;

    let graph = GraphDB::open(&db_path)?;
    let meta = graph
        .get_node("model:即时取数")
        .and_then(|node| node.meta)
        .expect("模型节点应有 meta");
    assert_eq!(meta["landed"], serde_json::json!(false), "{meta}");
    assert!(
        meta.get("nodeFields").is_some(),
        "增强后的 meta 仍在（确认走到了增强分支）: {meta}"
    );

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}

/// 单文件解析与项目扫描口径一致：空串 `dbTableName` 不产生输出，也不当作表 id。
#[test]
fn single_file_parser_treats_empty_db_table_name_as_unlanded() -> anyhow::Result<()> {
    let raw = serde_json::json!({
        "properties": {"dbTableName": ""},
        "dimensions": [{"name": "金额"}],
        "dataFlow": {"nodes": {}}
    });
    let meta = metadata_checker::tbl_single::parse_tbl(std::path::Path::new("即时取数.tbl"), raw)?;
    assert_eq!(meta.db_table_name, None);
    assert_eq!(meta.table_id.as_deref(), Some("即时取数"), "回退到文件名");
    assert_eq!(meta.dataflow_outputs.len(), 0);
    Ok(())
}

/// 升级后增量扫描必须重扫：文件指纹带扫描语义版本，旧格式指纹（纯内容哈希）判为脏，
/// 当前格式指纹判为干净。否则规则变了、文件没变，旧图会原样保留旧规则的结果。
#[test]
fn old_format_fingerprint_forces_a_rescan_after_a_semantics_change() -> anyhow::Result<()> {
    use metadata_checker::storage_provider::LocalDocumentProvider;

    let project_dir = unique_temp_dir("fingerprint");
    std::fs::create_dir_all(&project_dir)?;
    let page = serde_json::json!({"canvas": {"components": [{"id": "ok1", "type": "button"}]}});
    std::fs::write(project_dir.join("a.spg"), serde_json::to_string(&page)?)?;
    let db_path = project_dir.join("graph.db");
    ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;

    let states = GraphDB::open(&db_path)?.load_file_states()?;
    let current = states.get("a.spg").expect("a.spg 的文件状态").clone();
    assert!(
        current.file_hash.starts_with("s2-"),
        "指纹应带扫描语义版本: {}",
        current.file_hash
    );
    let files = vec![project_dir.join("a.spg")];
    let provider = LocalDocumentProvider;
    let plan = ProjectIndexer::diff_file_states(&files, &states, &project_dir, &provider)?;
    assert_eq!(plan.dirty.len(), 0, "当前格式指纹 = 干净");

    let mut legacy = states.clone();
    legacy.get_mut("a.spg").expect("a.spg").file_hash =
        current.file_hash["s2-".len()..].to_string();
    let plan = ProjectIndexer::diff_file_states(&files, &legacy, &project_dir, &provider)?;
    assert_eq!(plan.dirty.len(), 1, "旧格式指纹 = 脏，需要按新规则重扫");

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}
