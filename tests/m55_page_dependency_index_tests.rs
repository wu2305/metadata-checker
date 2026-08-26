#![cfg(feature = "cli-local")]
//! M55 Task 11：PageDependencyIndex 覆盖 Model/Field/DataFlow
//!
//! 覆盖：共享 Model 多页失效、物理字段失效、DataFlow 输出字段失效、
//! orchestrator 共享 Model 变化导致多页失效、coverage=partial 保守失效。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use metadata_checker::diff_refresh::{
    DiffRefreshCheckpoint, DiffRefreshOrchestrator, FixtureMetaFilesChangeSource, SourceCursor,
};
use metadata_checker::graph::{GraphDB, NodeType};
use metadata_checker::graph_store::{GraphReadStore, IndexCommit, IndexStateStore};
use metadata_checker::query::{PageDependencyIndex, PageDependencyIndexCoverage};
use metadata_checker::remote_metadata::{
    MetadataContentType, RemoteFileContent, RemoteFileInfo, RemoteFileRef,
};
use metadata_checker::runtime::{GraphRuntime, RuntimeMode};
use metadata_checker::scanner::indexer::ProjectIndexer;
use metadata_checker::session::manifest::SessionManifest;
use metadata_checker::session::remote_provider::{
    RemoteChangeSet, RemoteMetafileEntry, RemoteProjectInfo,
};
use metadata_checker::session::sync::{
    SessionSyncItem, SessionSyncMode, project_mirror_root, sync_remote_files_to_session,
};
use metadata_checker::session::{RemoteSessionProvider, SessionManager};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

fn test_root(name: &str) -> PathBuf {
    let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "metadata-checker-m55-page-dep-{name}-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// 引用指定 tbl 的最小页面（input 绑定 + text 读取）。
///
/// `model_id` 必须按页面区分：本地模型节点 id 为 `model:<model_id>`，
/// 多页面共用一个 id 会造成节点合并，干扰失效归属断言。
fn page_spg(model_id: &str, table_path: &str) -> String {
    serde_json::json!({
        "version": "4.19.7",
        "theme": "default",
        "sources": [
            {"id": model_id, "modelType": "dwtable", "path": table_path}
        ],
        "canvas": {
            "id": "canvas",
            "type": "canvas",
            "components": [
                {"id": "input1", "type": "input", "submitField": format!("{model_id}.name")},
                {"id": "text1", "type": "text", "value": format!("${{{model_id}.name}}")}
            ]
        }
    })
    .to_string()
}

const TABLE_V1: &str = r#"{"dimensions": [{"name": "id"}, {"name": "name"}]}"#;

/// 共享 Model 项目：page_a/page_b 引用同一 data/shared.tbl，page_c 用 data/other.tbl。
fn build_shared_model_project(name: &str) -> (PathBuf, PathBuf) {
    let project_dir = test_root(name);
    std::fs::create_dir_all(project_dir.join("app")).expect("create app dir");
    std::fs::create_dir_all(project_dir.join("data")).expect("create data dir");
    std::fs::write(
        project_dir.join("app/page_a.spg"),
        page_spg("model_a", "data/shared.tbl"),
    )
    .expect("write page_a");
    std::fs::write(
        project_dir.join("app/page_b.spg"),
        page_spg("model_b", "data/shared.tbl"),
    )
    .expect("write page_b");
    std::fs::write(
        project_dir.join("app/page_c.spg"),
        page_spg("model_c", "data/other.tbl"),
    )
    .expect("write page_c");
    std::fs::write(project_dir.join("data/shared.tbl"), TABLE_V1).expect("write shared tbl");
    std::fs::write(project_dir.join("data/other.tbl"), TABLE_V1).expect("write other tbl");
    let db_path = project_dir.join("graph.redb");
    ProjectIndexer::scan(&project_dir, &db_path).expect("scan shared model project");
    (project_dir, db_path)
}

fn node_ids_of(graph: &GraphDB, node_type: NodeType, path: &str) -> Vec<String> {
    GraphReadStore::iter_nodes(graph)
        .expect("iter nodes")
        .filter(|node| node.node_type == node_type && node.path == path)
        .map(|node| node.id)
        .collect()
}

fn expected_pages(ids: &[&str]) -> HashSet<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

/// 共享 Model：model 变化时 affected pages 恰为 {page_a, page_b}，page_c 不在集合中。
#[test]
fn m55_page_dependency_shared_model_affects_all_referencing_pages() {
    let (project_dir, db_path) = build_shared_model_project("shared-model");
    let graph = GraphDB::open(&db_path).expect("open graph");
    let index = PageDependencyIndex::build(&graph).expect("build index");
    assert_eq!(index.coverage(), PageDependencyIndexCoverage::Full);

    // 物理模型节点（tbl 文件变更时的 dirty 节点）
    let physical_models = node_ids_of(&graph, NodeType::Model, "data/shared.tbl");
    assert!(
        physical_models.iter().any(|id| id == "model:shared"),
        "physical model node should exist: {physical_models:?}"
    );

    let affected = index.affected_pages(&["model:shared".to_string()]);
    assert_eq!(
        affected,
        expected_pages(&["page:app/page_a.spg", "page:app/page_b.spg"])
    );
    assert!(!affected.contains("page:app/page_c.spg"));

    let _ = std::fs::remove_dir_all(project_dir);
}

/// 物理字段：shared.tbl 的字段变化同样只影响 page_a/page_b。
#[test]
fn m55_page_dependency_physical_field_affects_referencing_pages() {
    let (project_dir, db_path) = build_shared_model_project("shared-field");
    let graph = GraphDB::open(&db_path).expect("open graph");
    let index = PageDependencyIndex::build(&graph).expect("build index");

    let fields = node_ids_of(&graph, NodeType::Field, "data/shared.tbl");
    assert!(!fields.is_empty(), "shared tbl should have field nodes");

    let affected = index.affected_pages(&fields);
    assert_eq!(
        affected,
        expected_pages(&["page:app/page_a.spg", "page:app/page_b.spg"])
    );
    assert!(!affected.contains("page:app/page_c.spg"));

    let _ = std::fs::remove_dir_all(project_dir);
}

/// DataFlow 输出字段：合成项目（dataflow 消费页 + 无关页，本地模型 id 不同），
/// real_dataflow.tbl 的字段变化只影响消费页。
///
/// 不用 tests/fixtures/test_project 直接测：fixture 全部页面共用本地模型 id
/// `model1`，图中本地模型节点合并，任何经该节点的字段都会保守关联到所有页，
/// 无法表达「无关页不出现」的断言。
#[test]
fn m55_page_dependency_dataflow_output_field_affects_consuming_page() {
    let project_dir = test_root("dataflow-field");
    std::fs::create_dir_all(project_dir.join("app")).expect("create app dir");
    std::fs::create_dir_all(project_dir.join("data")).expect("create data dir");

    // dataflow 消费页：复用 fixture 的 p34 页面，但把本地模型 id 改为 dfmodel1
    let p34 =
        std::fs::read_to_string("tests/fixtures/test_project/app/p34_value_source_dataflow.spg")
            .expect("read p34 fixture");
    std::fs::write(
        project_dir.join("app/page_df.spg"),
        p34.replace("model1", "dfmodel1"),
    )
    .expect("write page_df");
    std::fs::copy(
        "tests/fixtures/test_project/app/real_dataflow.tbl",
        project_dir.join("app/real_dataflow.tbl"),
    )
    .expect("copy real_dataflow tbl");
    // 无关页：引用独立 tbl、不同本地模型 id
    std::fs::write(
        project_dir.join("app/page_unrelated.spg"),
        page_spg("model_c", "data/other.tbl"),
    )
    .expect("write unrelated page");
    std::fs::write(project_dir.join("data/other.tbl"), TABLE_V1).expect("write other tbl");

    let db_path = project_dir.join("graph.redb");
    ProjectIndexer::scan(&project_dir, &db_path).expect("scan dataflow project");
    let graph = GraphDB::open(&db_path).expect("open graph");
    let index = PageDependencyIndex::build(&graph).expect("build index");

    let dataflow_fields = node_ids_of(&graph, NodeType::Field, "app/real_dataflow.tbl");
    assert!(
        !dataflow_fields.is_empty(),
        "real_dataflow.tbl should have output field nodes"
    );

    let affected = index.affected_pages(&dataflow_fields);
    assert!(
        affected.contains("page:app/page_df.spg"),
        "dataflow consumer page should be affected: {affected:?}"
    );
    assert!(
        !affected.contains("page:app/page_unrelated.spg"),
        "unrelated page must not be affected: {affected:?}"
    );

    // DataFlow 模型节点本身变化同样只影响消费页
    let model_affected = index.affected_pages(&["model:real_dataflow".to_string()]);
    assert!(
        model_affected.contains("page:app/page_df.spg"),
        "dataflow model change should affect consumer page: {model_affected:?}"
    );
    assert!(
        !model_affected.contains("page:app/page_unrelated.spg"),
        "unrelated page must not be affected by dataflow model: {model_affected:?}"
    );

    let _ = std::fs::remove_dir_all(project_dir);
}

/// 内容 fetch stub provider。
struct StubProvider {
    contents: std::collections::HashMap<String, RemoteFileContent>,
}

impl RemoteSessionProvider for StubProvider {
    fn list_projects(&self) -> anyhow::Result<Vec<RemoteProjectInfo>> {
        Err(anyhow::anyhow!("stub"))
    }

    fn list_metafiles(&self, _p: &str) -> anyhow::Result<Vec<RemoteMetafileEntry>> {
        Err(anyhow::anyhow!("stub"))
    }

    fn fetch_metafile_info(&self, _f: &RemoteFileRef) -> anyhow::Result<RemoteFileInfo> {
        Err(anyhow::anyhow!("stub"))
    }

    fn fetch_metafile_content(
        &self,
        file_ref: &RemoteFileRef,
    ) -> anyhow::Result<RemoteFileContent> {
        self.contents
            .get(&file_ref.source_path)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no content for {}", file_ref.source_path))
    }

    fn fetch_changed_since(&self, _p: &str, _r: &str) -> anyhow::Result<RemoteChangeSet> {
        Err(anyhow::anyhow!("stub"))
    }
}

fn seed_file(
    session_dir: &Path,
    manifest: &mut SessionManifest,
    path: &str,
    file_id: &str,
    revision: &str,
    raw_text: &str,
) {
    let content = RemoteFileContent {
        source_path: path.to_string(),
        file_id: Some(file_id.to_string()),
        revision: Some(revision.to_string()),
        content_type: MetadataContentType::from_extension(path.rsplit('.').next().unwrap_or("")),
        raw_text: raw_text.to_string(),
    };
    sync_remote_files_to_session(
        session_dir,
        manifest,
        &[SessionSyncItem::new(content)],
        SessionSyncMode::Partial,
    )
    .expect("seed mirror file");
}

/// orchestrator 等价测试：共享 Model（table1.tbl）变化导致 page_a/page_b 同时失效。
#[test]
fn m55_page_dependency_orchestrator_shared_model_invalidates_multiple_pages() {
    let root = test_root("orchestrator-shared");
    let manager = SessionManager::new(&root);
    manager
        .create_session("s1", "https://bi.test", "proj", "proj", "remote")
        .expect("create session");
    let session_dir = manager.session_dir("s1");
    let mut manifest = manager.read_manifest("s1").expect("read manifest");
    seed_file(
        &session_dir,
        &mut manifest,
        "app/page_a.spg",
        "file-a",
        "1",
        &page_spg("model_a", "data/table1.tbl"),
    );
    seed_file(
        &session_dir,
        &mut manifest,
        "app/page_b.spg",
        "file-b",
        "1",
        &page_spg("model_b", "data/table1.tbl"),
    );
    seed_file(
        &session_dir,
        &mut manifest,
        "data/table1.tbl",
        "file-t",
        "1",
        TABLE_V1,
    );
    let db_path = session_dir.join("graph.redb");
    ProjectIndexer::scan(&project_mirror_root(&session_dir), &db_path).expect("initial scan");
    manifest.graph_db_path = db_path.to_string_lossy().to_string();
    manager.write_manifest(&manifest).expect("write manifest");

    // 预置 checkpoint 走 poll 路径；fixture 只有 table1.tbl 一个变更事件
    {
        let mut graph = GraphDB::open(&db_path).expect("open graph");
        let commit = IndexCommit {
            file_states: graph.load_file_states().expect("load states"),
            dirty_nodes: Vec::new(),
            deleted_nodes: Vec::new(),
            checkpoint: Some(DiffRefreshCheckpoint {
                active: SourceCursor::new(500, Vec::new()),
                deleted: SourceCursor::new(0, Vec::new()),
            }),
            delta: None,
            scanner_entries: Vec::new(),
            scanner_deleted_paths: Vec::new(),
        };
        IndexStateStore::persist_index(&mut graph, commit).expect("seed checkpoint");
    }
    let fixture = serde_json::json!({
        "schema_version": 1,
        "events": [
            {"event_id": "active:file-t:2", "file_id": "file-t",
             "source_path": "data/table1.tbl", "previous_source_path": null,
             "content_type": "table", "updated_at_ms": 1000, "deleted": false}
        ]
    });
    let source =
        FixtureMetaFilesChangeSource::from_json_str(&fixture.to_string()).expect("fixture source");

    let tbl_v2 = r#"{"dimensions": [{"name": "id"}, {"name": "name"}, {"name": "extra"}]}"#;
    let mut provider = StubProvider {
        contents: std::collections::HashMap::new(),
    };
    provider.contents.insert(
        "data/table1.tbl".to_string(),
        RemoteFileContent {
            source_path: "data/table1.tbl".to_string(),
            file_id: Some("file-t".to_string()),
            revision: Some("2".to_string()),
            content_type: MetadataContentType::Table,
            raw_text: tbl_v2.to_string(),
        },
    );

    let mut runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load runtime");
    runtime
        .warm_page_logic_availability("page:app/page_a.spg", "normal")
        .expect("warm page_a");
    runtime
        .warm_page_logic_availability("page:app/page_b.spg", "normal")
        .expect("warm page_b");

    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir,
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    );
    let report = orchestrator.refresh_once().expect("refresh once");

    // 共享 Model 变化 → 两个引用页同时失效
    let invalidated: HashSet<&String> = report.invalidated_pages.iter().collect();
    assert!(invalidated.contains(&"page:app/page_a.spg".to_string()));
    assert!(invalidated.contains(&"page:app/page_b.spg".to_string()));
    assert_eq!(
        report.page_dep_index_coverage,
        PageDependencyIndexCoverage::Full
    );
    assert_eq!(report.warm_failures, Vec::<String>::new());

    let _ = std::fs::remove_dir_all(root);
}

/// coverage=partial：旧索引降级（empty）时保守失效全部 warm 页面并输出标记。
#[test]
fn m55_page_dependency_partial_coverage_conservatively_invalidates_all_warm_pages() {
    let (project_dir, db_path) = build_shared_model_project("partial-coverage");
    let mut runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(&project_dir),
        RuntimeMode::LongLived,
    )
    .expect("load runtime");
    runtime
        .warm_page_logic_availability("page:app/page_a.spg", "normal")
        .expect("warm page_a");
    runtime
        .warm_page_logic_availability("page:app/page_b.spg", "normal")
        .expect("warm page_b");

    // 把旧索引降级为 empty（Partial），保留 warm cache
    let degraded_model = {
        let current = runtime.read_model.as_ref().expect("read model");
        metadata_checker::runtime::RuntimeReadModel {
            dense_graph: current.dense_graph.clone(),
            availability_facts: current.availability_facts.clone(),
            page_logic_availability: current.page_logic_availability.clone(),
            page_dependency_index: std::sync::Arc::new(PageDependencyIndex::empty()),
        }
    };
    runtime.read_model = Some(std::sync::Arc::new(degraded_model));

    // 候选图 = 同一份 graph 重开（无 dirty）
    let candidate = GraphDB::open(&db_path).expect("open candidate");
    let prepared = runtime
        .prepare_replacement(&candidate, &[])
        .expect("prepare replacement");

    assert_eq!(
        prepared.page_dep_index_coverage,
        PageDependencyIndexCoverage::Partial
    );
    // 覆盖无法证明：全部 warm 页面都被保守失效
    let invalidated: HashSet<&String> = prepared.invalidated_pages.iter().collect();
    assert!(invalidated.contains(&"page:app/page_a.spg".to_string()));
    assert!(invalidated.contains(&"page:app/page_b.spg".to_string()));

    let _ = std::fs::remove_dir_all(project_dir);
}
