#[path = "common/bench_config.rs"]
mod bench_config;
#[path = "common/runtime_request.rs"]
mod runtime_request;

use bench_config::real_project_criterion_config;
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use metadata_checker::remote_metadata::{MetadataContentType, RemoteFileContent};
use metadata_checker::runtime::GraphRuntime;
use metadata_checker::session::remote_provider::{
    InMemoryRemoteSessionProvider, RemoteMetafileEntry, RemoteProjectInfo,
};
use metadata_checker::session::{
    SessionManager, SessionRefreshOptions, SessionSyncMode, refresh_session_from_remote,
};
use metadata_checker::tool_contract::ToolCommand;
use runtime_request::runtime_query_request;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SESSION_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
}

fn fixture_text(relative: &str) -> String {
    fs::read_to_string(fixture_path(relative))
        .unwrap_or_else(|error| panic!("read fixture {relative}: {error}"))
}

fn temp_session_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = SESSION_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "metadata-checker-session-bench-{label}-{nanos}-{seq}"
    ))
}

fn build_recorded_provider(project_ref: &str) -> InMemoryRemoteSessionProvider {
    let tbl_raw = fixture_text("test_project/app/dataflow_output.tbl");
    let spg_raw = fixture_text("test_project/app/page_relations.spg");
    let mut provider = InMemoryRemoteSessionProvider::new();
    provider
        .register_project(RemoteProjectInfo {
            project_ref: project_ref.to_string(),
            project_name: "bench-project".to_string(),
            source_origin: "recorded".to_string(),
        })
        .expect("register remote project");
    for (source_path, raw_text, content_type) in [
        (
            "app/page_relations.spg",
            spg_raw,
            MetadataContentType::SuperPage,
        ),
        (
            "app/dataflow_output.tbl",
            tbl_raw,
            MetadataContentType::Table,
        ),
    ] {
        provider
            .add_metafile(
                RemoteMetafileEntry {
                    project_ref: project_ref.to_string(),
                    source_path: source_path.to_string(),
                    file_id: Some(format!("id-{source_path}")),
                    revision: Some("1".to_string()),
                    etag: None,
                    mtime: Some(1_715_000_000_000),
                    size: Some(raw_text.len() as u64),
                    deleted: false,
                },
                RemoteFileContent {
                    source_path: source_path.to_string(),
                    file_id: Some(format!("id-{source_path}")),
                    revision: Some("1".to_string()),
                    content_type,
                    raw_text,
                },
            )
            .expect("register recorded remote file");
    }
    provider
}

/// 衡量 remote fetch -> mirror -> graph build 的端到端成本。
fn bench_session_remote_sync_build_graph(c: &mut Criterion) {
    let project_ref = "bench-session-project";
    let provider = build_recorded_provider(project_ref);

    c.bench_function("session_remote_sync_build_graph", |bench| {
        bench.iter_batched(
            || {
                let root = temp_session_root("sync-build");
                let manager = SessionManager::new(&root);
                (root, manager)
            },
            |(root, manager)| {
                let session_id = "bench-session";
                manager
                    .create_session(
                        session_id,
                        "https://bi.test",
                        project_ref,
                        "bench-project",
                        "remote",
                    )
                    .expect("create session");
                let graph_db_path = root.join(session_id).join("graph.redb");
                let report = refresh_session_from_remote(
                    &provider,
                    &manager,
                    SessionRefreshOptions {
                        session_id: session_id.to_string(),
                        remote_server: "https://bi.test".to_string(),
                        project_ref: project_ref.to_string(),
                        project_name: "bench-project".to_string(),
                        sync_mode: SessionSyncMode::Full,
                        create_if_missing: false,
                        filter: None,
                        graph_db_path: Some(graph_db_path.clone()),
                    },
                )
                .expect("refresh session from recorded provider");
                assert!(report.ok, "session refresh should succeed");
                assert!(report.index.dirty > 0 || report.index.indexed > 0);
                assert!(graph_db_path.exists(), "session graphdb should be created");
                let _ = fs::remove_dir_all(&root);
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量 remote sync 后 warm query 成本。
fn bench_session_remote_sync_then_query(c: &mut Criterion) {
    let project_ref = "bench-session-query";
    let provider = build_recorded_provider(project_ref);
    let root = temp_session_root("sync-query");
    let manager = SessionManager::new(&root);
    let session_id = "bench-session";
    manager
        .create_session(
            session_id,
            "https://bi.test",
            project_ref,
            "bench-project",
            "remote",
        )
        .expect("create session");
    let graph_db_path = root.join(session_id).join("graph.redb");
    let session_dir = manager.session_dir(session_id);
    let project_mirror = session_dir.join("project");
    refresh_session_from_remote(
        &provider,
        &manager,
        SessionRefreshOptions {
            session_id: session_id.to_string(),
            remote_server: "https://bi.test".to_string(),
            project_ref: project_ref.to_string(),
            project_name: "bench-project".to_string(),
            sync_mode: SessionSyncMode::Full,
            create_if_missing: false,
            filter: None,
            graph_db_path: Some(graph_db_path.clone()),
        },
    )
    .expect("prepare session graph for query benchmark");
    let mut runtime = GraphRuntime::load_with_project_dir(&graph_db_path, Some(&project_mirror))
        .expect("load session graph runtime");
    let request = runtime_query_request(
        ToolCommand::QueryPage,
        "page:app/page_relations.spg",
        "compact",
        None,
        None,
        false,
    );

    c.bench_function("session_remote_sync_then_query_page", |bench| {
        bench.iter(|| {
            let response = runtime
                .query(black_box(request.clone()))
                .expect("session graph query should succeed");
            assert!(response.result.get("kind").is_some());
            black_box(response);
        });
    });
}

fn bench_session_sync_scenarios(c: &mut Criterion) {
    bench_session_remote_sync_build_graph(c);
    bench_session_remote_sync_then_query(c);
}

criterion_group! {
    name = session_sync_benches;
    config = real_project_criterion_config();
    targets = bench_session_sync_scenarios
}
criterion_main!(session_sync_benches);
