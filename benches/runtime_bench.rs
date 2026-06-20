#[path = "common/bench_config.rs"]
mod bench_config;
#[path = "common/external_graph_lock.rs"]
mod external_graph_lock;
#[path = "common/first_existing_target.rs"]
mod first_existing_target;
#[path = "common/real_project.rs"]
mod real_project;
#[path = "common/runtime_exec.rs"]
mod runtime_exec;
#[path = "common/runtime_request.rs"]
mod runtime_request;
#[path = "common/sandbox_create.rs"]
mod sandbox_create;

use anyhow::{Context, Result};
use bench_config::real_project_criterion_config;
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use external_graph_lock::acquire_external_graph_lock;
use first_existing_target::first_existing_target;
use metadata_checker::graph::set_graph_lock_timeout_ms;
use metadata_checker::runtime::GraphRuntime;
use metadata_checker::scanner::indexer::ProjectIndexer;
use metadata_checker::tool_contract::ToolCommand;
use real_project::require_real_project_dir;
use runtime_exec::run_runtime_query;
use runtime_request::runtime_query_request;
use sandbox_create::create_indexed_workspace;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;

const CONTRACT_PAGE_FILE: &str = "app/销售.app/销售/合同协议.spg";
const CONTRACT_PAGE: &str = "page:app/销售.app/销售/合同协议.spg";
const INPUT3: &str = "comp:app/销售.app/销售/合同协议.spg|input3";

fn write_dirty_variant(path: &Path, original: &[u8], iteration: usize) -> Result<()> {
    let mut content = original.to_vec();
    content.push(b'\n');
    content.extend(std::iter::repeat(b' ').take((iteration % 31) + 1));
    std::fs::write(path, content).with_context(|| format!("write dirty variant {}", path.display()))
}

fn lifecycle_request(command: ToolCommand) -> metadata_checker::runtime::RuntimeQueryRequest {
    metadata_checker::runtime::RuntimeQueryRequest {
        command,
        target: String::new(),
        budget: "normal".to_string(),
        human: false,
        intent: None,
        page_scope: None,
        depth: None,
        check_reload: false,
    }
}

fn corrupt_graphdb_header(source: &Path, destination: &Path) -> Result<()> {
    let mut bytes =
        std::fs::read(source).with_context(|| format!("read graphdb {}", source.display()))?;
    for byte in bytes.iter_mut().take(64) {
        *byte = 0;
    }
    std::fs::write(destination, bytes)
        .with_context(|| format!("write corrupt graphdb {}", destination.display()))
}

/// 衡量真实 graphdb 冷加载为 GraphRuntime 的成本。
fn bench_runtime_load(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("runtime-load", source_project_dir).expect("create workspace");
    c.bench_function("runtime_load_graphdb", |bench| {
        bench.iter(|| {
            let runtime = GraphRuntime::load_with_project_dir(
                black_box(&workspace.db_path),
                Some(black_box(&workspace.project_dir)),
            )
            .expect("runtime load should succeed");
            black_box(runtime);
        });
    });
}

/// 衡量 warm runtime status 查询成本。
fn bench_runtime_status(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("runtime-status", source_project_dir).expect("create workspace");
    let mut runtime =
        GraphRuntime::load_with_project_dir(&workspace.db_path, Some(&workspace.project_dir))
            .expect("runtime load should succeed");
    let request = lifecycle_request(ToolCommand::Status);

    c.bench_function("runtime_status_warm", |bench| {
        bench.iter(|| {
            let response = runtime
                .query(black_box(request.clone()))
                .expect("status should succeed");
            black_box(response);
        });
    });
}

/// 衡量 graphdb 未变化时 check_reload 的 fast path 成本。
fn bench_runtime_check_reload_unchanged(c: &mut Criterion, source_project_dir: &Path) {
    let workspace = create_indexed_workspace("runtime-check-unchanged", source_project_dir)
        .expect("create workspace");
    let mut runtime =
        GraphRuntime::load_with_project_dir(&workspace.db_path, Some(&workspace.project_dir))
            .expect("runtime load should succeed");
    let request = lifecycle_request(ToolCommand::CheckReload);

    c.bench_function("runtime_check_reload_unchanged", |bench| {
        bench.iter(|| {
            let response = runtime
                .query(black_box(request.clone()))
                .expect("check_reload should succeed");
            black_box(response);
        });
    });
}

/// 衡量强制 reload graphdb 的成本。
fn bench_runtime_reload_graph(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("runtime-reload", source_project_dir).expect("create workspace");
    let mut runtime =
        GraphRuntime::load_with_project_dir(&workspace.db_path, Some(&workspace.project_dir))
            .expect("runtime load should succeed");
    let request = lifecycle_request(ToolCommand::ReloadGraph);

    c.bench_function("runtime_reload_graph", |bench| {
        bench.iter(|| {
            let response = runtime
                .query(black_box(request.clone()))
                .expect("reload_graph should succeed");
            black_box(response);
        });
    });
}

/// 衡量 graphdb 文件变化后 check_reload 的 slow path 成本。
fn bench_runtime_check_reload_changed(c: &mut Criterion, source_project_dir: &Path) {
    let workspace = create_indexed_workspace("runtime-check-changed", source_project_dir)
        .expect("create workspace");
    let mut runtime =
        GraphRuntime::load_with_project_dir(&workspace.db_path, Some(&workspace.project_dir))
            .expect("runtime load should succeed");
    let target = first_existing_target(&workspace.project_dir, &[CONTRACT_PAGE_FILE])
        .expect("real project should contain target SPG");
    let original = std::fs::read(&target).expect("read target SPG");
    let request = lifecycle_request(ToolCommand::CheckReload);
    let mut iteration = 0_usize;

    c.bench_function("runtime_check_reload_changed", |bench| {
        bench.iter_batched(
            || {
                iteration += 1;
                write_dirty_variant(&target, &original, iteration).expect("write dirty SPG");
                ProjectIndexer::scan(&workspace.project_dir, &workspace.db_path)
                    .expect("dirty scan should succeed");
            },
            |_| {
                let response = runtime
                    .query(black_box(request.clone()))
                    .expect("check_reload changed should succeed");
                black_box(response);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量 graphdb 被外部锁占用时的加载失败成本。
fn bench_runtime_load_locked_graphdb(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("runtime-locked", source_project_dir).expect("create workspace");
    set_graph_lock_timeout_ms(500);
    let db_path = workspace.db_path.clone();
    let project_dir = workspace.project_dir.clone();

    c.bench_function("runtime_load_locked_graphdb", |bench| {
        bench.iter(|| {
            let barrier = Arc::new(Barrier::new(2));
            let db_path_thread = db_path.clone();
            let wait = barrier.clone();
            let lock_thread = thread::spawn(move || {
                let _lock =
                    acquire_external_graph_lock(&db_path_thread).expect("hold external graph lock");
                wait.wait();
                wait.wait();
            });
            barrier.wait();
            let result = GraphRuntime::load_with_project_dir(&db_path, Some(&project_dir));
            barrier.wait();
            lock_thread.join().expect("join lock thread");
            assert!(result.is_err(), "locked graphdb load should fail");
            let _ = black_box(result);
        });
    });
}

/// 衡量损坏 graphdb 的诊断路径成本。
fn bench_runtime_load_corrupt_graphdb(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("runtime-corrupt", source_project_dir).expect("create workspace");
    let corrupt_path = workspace.root.join("corrupt.graphdb");

    c.bench_function("runtime_load_corrupt_graphdb", |bench| {
        bench.iter_batched(
            || {
                corrupt_graphdb_header(&workspace.db_path, &corrupt_path)
                    .expect("create corrupt graphdb");
            },
            |_| {
                let result = GraphRuntime::load_with_project_dir(&corrupt_path, None::<&Path>);
                assert!(result.is_err(), "corrupt graphdb load should fail");
                let _ = black_box(result);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量业务查询前执行 check_reload 的成本。
fn bench_runtime_query_page_with_check_reload(c: &mut Criterion, source_project_dir: &Path) {
    let workspace = create_indexed_workspace("runtime-query-reload", source_project_dir)
        .expect("create workspace");
    let mut runtime =
        GraphRuntime::load_with_project_dir(&workspace.db_path, Some(&workspace.project_dir))
            .expect("runtime load should succeed");
    let request = runtime_query_request(
        ToolCommand::QueryPage,
        CONTRACT_PAGE,
        "compact",
        None,
        None,
        true,
    );

    c.bench_function("runtime_query_page_with_check_reload", |bench| {
        bench.iter(|| {
            let response = run_runtime_query(&mut runtime, black_box(request.clone()))
                .expect("query with check_reload should succeed");
            black_box(response);
        });
    });
}

/// 衡量 explain 查询前执行 check_reload 的成本。
fn bench_runtime_explain_condition_with_check_reload(c: &mut Criterion, source_project_dir: &Path) {
    let workspace = create_indexed_workspace("runtime-explain-reload", source_project_dir)
        .expect("create workspace");
    let mut runtime =
        GraphRuntime::load_with_project_dir(&workspace.db_path, Some(&workspace.project_dir))
            .expect("runtime load should succeed");
    let request = runtime_query_request(
        ToolCommand::ExplainCondition,
        INPUT3,
        "compact",
        Some("writer"),
        None,
        true,
    );

    c.bench_function("runtime_explain_condition_with_check_reload", |bench| {
        bench.iter(|| {
            let response = run_runtime_query(&mut runtime, black_box(request.clone()))
                .expect("explain with check_reload should succeed");
            black_box(response);
        });
    });
}

/// 注册真实项目 runtime lifecycle benchmark。
fn bench_runtime_scenarios(c: &mut Criterion) {
    let Some(source_project_dir) = require_real_project_dir("runtime_bench") else {
        return;
    };

    bench_runtime_load(c, &source_project_dir);
    bench_runtime_status(c, &source_project_dir);
    bench_runtime_check_reload_unchanged(c, &source_project_dir);
    bench_runtime_reload_graph(c, &source_project_dir);
    bench_runtime_check_reload_changed(c, &source_project_dir);
    bench_runtime_load_locked_graphdb(c, &source_project_dir);
    bench_runtime_load_corrupt_graphdb(c, &source_project_dir);
    bench_runtime_query_page_with_check_reload(c, &source_project_dir);
    bench_runtime_explain_condition_with_check_reload(c, &source_project_dir);
}

criterion_group! {
    name = runtime_benches;
    config = real_project_criterion_config();
    targets = bench_runtime_scenarios
}
criterion_main!(runtime_benches);
