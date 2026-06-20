#[path = "common/bench_config.rs"]
mod bench_config;
#[path = "common/first_existing_target.rs"]
mod first_existing_target;
#[path = "common/real_project.rs"]
mod real_project;
#[path = "common/restore_file.rs"]
mod restore_file;
#[path = "common/sandbox_create.rs"]
mod sandbox_create;

use anyhow::Context;
use bench_config::real_project_criterion_config;
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use first_existing_target::first_existing_target;
use metadata_checker::graph::{GraphDB, set_graph_lock_timeout_ms};
use metadata_checker::graph_store::IndexCommit;
use metadata_checker::scanner::indexer::ProjectIndexer;
use metadata_checker::storage_provider::LocalStorageProvider;
use real_project::require_real_project_dir;
use restore_file::restore_file;
use sandbox_create::create_indexed_workspace;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;

const CONTRACT_PAGE_FILE: &str = "app/销售.app/销售/合同协议.spg";

fn lock_file_path(db_path: &Path) -> PathBuf {
    db_path.with_extension("graphdb.lock")
}

fn acquire_external_graph_lock(db_path: &Path) -> anyhow::Result<std::fs::File> {
    let lock_path = lock_file_path(db_path);
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .with_context(|| format!("acquire external graph lock {}", lock_path.display()))
}

fn corrupt_graphdb_header(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let mut bytes =
        std::fs::read(source).with_context(|| format!("read graphdb {}", source.display()))?;
    for byte in bytes.iter_mut().take(64) {
        *byte = 0;
    }
    std::fs::write(destination, bytes)
        .with_context(|| format!("write corrupt graphdb {}", destination.display()))
}

fn write_dirty_variant(path: &Path, original: &[u8], iteration: usize) -> anyhow::Result<()> {
    let mut content = original.to_vec();
    content.push(b'\n');
    content.extend(std::iter::repeat(b' ').take((iteration % 31) + 1));
    std::fs::write(path, content).with_context(|| format!("write dirty variant {}", path.display()))
}

/// 衡量已有 graphdb 的只读打开成本。
fn bench_redb_cold_open_readonly(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("redb-open", source_project_dir).expect("create workspace");
    c.bench_function("redb_cold_open_readonly", |bench| {
        bench.iter(|| {
            let graph = GraphDB::open_readonly(black_box(&workspace.db_path))
                .expect("readonly open should succeed");
            black_box(graph);
        });
    });
}

/// 衡量 graphdb 锁竞争时的打开失败成本。
fn bench_redb_lock_contention_open(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("redb-lock", source_project_dir).expect("create workspace");
    set_graph_lock_timeout_ms(500);
    let db_path = workspace.db_path.clone();

    c.bench_function("redb_lock_contention_open", |bench| {
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
            let result = GraphDB::open_readonly(&db_path);
            barrier.wait();
            lock_thread.join().expect("join lock thread");
            assert!(result.is_err(), "locked graphdb open should fail");
            let _ = black_box(result);
        });
    });
}

/// 衡量损坏 graphdb 的诊断成本。
fn bench_redb_corrupt_check_graphdb(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("redb-corrupt", source_project_dir).expect("create workspace");
    let corrupt_path = workspace.root.join("corrupt.graphdb");

    c.bench_function("redb_corrupt_check_graphdb", |bench| {
        bench.iter_batched(
            || {
                corrupt_graphdb_header(&workspace.db_path, &corrupt_path)
                    .expect("create corrupt graphdb");
            },
            |_| {
                let status = GraphDB::check_graph_db(black_box(&corrupt_path));
                black_box(status);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量 graphdb 加载 file_states 索引元数据的成本。
fn bench_redb_load_file_states(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("redb-file-states", source_project_dir).expect("create workspace");
    c.bench_function("redb_load_file_states", |bench| {
        bench.iter(|| {
            let graph = GraphDB::open(black_box(&workspace.db_path)).expect("open graphdb");
            let states = graph
                .load_file_states()
                .expect("load file states should succeed");
            assert!(
                !states.is_empty(),
                "indexed graphdb should have file states"
            );
            black_box(states);
        });
    });
}

/// 衡量 discover + diff 阶段的纯扫描/hash 成本（不含 parse/apply/persist）。
fn bench_redb_scan_discover_diff(c: &mut Criterion, source_project_dir: &Path) {
    let workspace = create_indexed_workspace("redb-discover-diff", source_project_dir)
        .expect("create workspace");
    let target = first_existing_target(&workspace.project_dir, &[CONTRACT_PAGE_FILE])
        .expect("real project should contain target SPG");
    let original = std::fs::read(&target).expect("read target SPG");
    let mut iteration = 0_usize;
    let provider = LocalStorageProvider;

    c.bench_function("redb_scan_discover_diff", |bench| {
        bench.iter_batched(
            || {
                iteration += 1;
                write_dirty_variant(&target, &original, iteration).expect("write dirty SPG");
            },
            |_| {
                let graph = GraphDB::open(&workspace.db_path).expect("open graphdb");
                let prev_states = graph.load_file_states().unwrap_or_default();
                let discovered =
                    ProjectIndexer::discover_files(&workspace.project_dir).expect("discover files");
                let plan = ProjectIndexer::diff_file_states(
                    &discovered,
                    &prev_states,
                    &workspace.project_dir,
                    &provider,
                )
                .expect("diff file states");
                assert!(
                    plan.dirty.len() >= 1,
                    "dirty diff should detect changed SPG"
                );
                black_box(plan);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量脏文件 parse + apply_graph_updates 成本（不含 discover/diff/persist）。
fn bench_redb_incremental_parse_apply(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("redb-parse-apply", source_project_dir).expect("create workspace");
    let target = first_existing_target(&workspace.project_dir, &[CONTRACT_PAGE_FILE])
        .expect("real project should contain target SPG");
    let original = std::fs::read(&target).expect("read target SPG");
    let baseline_db = std::fs::read(&workspace.db_path).expect("read baseline graphdb");
    let mut iteration = 0_usize;
    let provider = LocalStorageProvider;

    c.bench_function("redb_incremental_parse_apply", |bench| {
        bench.iter_batched(
            || {
                iteration += 1;
                restore_file(&workspace.db_path, &baseline_db).expect("restore baseline graphdb");
                write_dirty_variant(&target, &original, iteration).expect("write dirty SPG");
            },
            |_| {
                let mut graph = GraphDB::open(&workspace.db_path).expect("open graphdb");
                let prev_states = graph.load_file_states().unwrap_or_default();
                let discovered =
                    ProjectIndexer::discover_files(&workspace.project_dir).expect("discover files");
                let plan = ProjectIndexer::diff_file_states(
                    &discovered,
                    &prev_states,
                    &workspace.project_dir,
                    &provider,
                )
                .expect("diff file states");
                let updates = ProjectIndexer::parse_dirty_files(
                    &prev_states,
                    &plan.dirty,
                    &workspace.project_dir,
                    &provider,
                )
                .expect("parse dirty files");
                let touched = ProjectIndexer::apply_graph_updates(&mut graph, &updates)
                    .expect("apply graph updates");
                assert!(!touched.is_empty(), "apply should touch dirty file nodes");
                black_box(touched);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量 persist_index 提交成本（parse/apply 在 setup 完成）。
fn bench_redb_incremental_persist_commit(c: &mut Criterion, source_project_dir: &Path) {
    let workspace = create_indexed_workspace("redb-persist-commit", source_project_dir)
        .expect("create workspace");
    let target = first_existing_target(&workspace.project_dir, &[CONTRACT_PAGE_FILE])
        .expect("real project should contain target SPG");
    let original = std::fs::read(&target).expect("read target SPG");
    let baseline_db = std::fs::read(&workspace.db_path).expect("read baseline graphdb");
    let mut iteration = 0_usize;
    let provider = LocalStorageProvider;

    c.bench_function("redb_incremental_persist_commit", |bench| {
        bench.iter_batched(
            || {
                iteration += 1;
                write_dirty_variant(&target, &original, iteration).expect("write dirty SPG");
                restore_file(&workspace.db_path, &baseline_db).expect("restore baseline graphdb");
            },
            |iter_index| {
                let mut graph = GraphDB::open(&workspace.db_path).expect("open graphdb");
                let prev_states = graph.load_file_states().unwrap_or_default();
                let discovered =
                    ProjectIndexer::discover_files(&workspace.project_dir).expect("discover files");
                let plan = ProjectIndexer::diff_file_states(
                    &discovered,
                    &prev_states,
                    &workspace.project_dir,
                    &provider,
                )
                .expect("diff file states");
                let updates = ProjectIndexer::parse_dirty_files(
                    &prev_states,
                    &plan.dirty,
                    &workspace.project_dir,
                    &provider,
                )
                .expect("parse dirty files");
                let touched = ProjectIndexer::apply_graph_updates(&mut graph, &updates)
                    .expect("apply graph updates");
                let mut new_states = prev_states.clone();
                for update in &updates {
                    let logical_path = update.logical_path.clone();
                    let node_ids = touched
                        .get(logical_path.as_str())
                        .cloned()
                        .unwrap_or_default();
                    new_states.insert(
                        logical_path.clone(),
                        metadata_checker::graph::FileState {
                            file_path: logical_path,
                            file_hash: update.file_hash.clone(),
                            mtime: update.mtime,
                            size: update.size,
                            node_ids,
                        },
                    );
                }
                let commit = IndexCommit {
                    file_states: new_states,
                    dirty_nodes: graph.dirty_nodes_set().iter().cloned().collect(),
                    deleted_nodes: graph.removed_nodes_set().iter().cloned().collect(),
                };
                let report = ProjectIndexer::persist_index(&mut graph, commit)
                    .expect("persist index commit");
                assert!(
                    report.dirty >= 1,
                    "persist commit should record dirty files"
                );
                black_box((report, iter_index));
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量单个 SPG dirty 后的增量 scan + persist 成本。
fn bench_redb_incremental_persist_dirty_spg(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("redb-persist", source_project_dir).expect("create workspace");
    let target = first_existing_target(&workspace.project_dir, &[CONTRACT_PAGE_FILE])
        .expect("real project should contain target SPG");
    let original = std::fs::read(&target).expect("read target SPG");
    let mut iteration = 0_usize;

    c.bench_function("redb_incremental_persist_dirty_spg", |bench| {
        bench.iter_batched(
            || {
                iteration += 1;
                write_dirty_variant(&target, &original, iteration).expect("write dirty SPG");
            },
            |_| {
                let report = ProjectIndexer::scan(
                    black_box(&workspace.project_dir),
                    black_box(&workspace.db_path),
                )
                .expect("dirty scan should succeed");
                assert!(report.dirty >= 1, "dirty scan should mark dirty files");
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 注册 redb 持久化与打开路径 benchmark。
fn bench_redb_persistence_scenarios(c: &mut Criterion) {
    let Some(source_project_dir) = require_real_project_dir("redb_persistence_bench") else {
        return;
    };

    bench_redb_cold_open_readonly(c, &source_project_dir);
    bench_redb_load_file_states(c, &source_project_dir);
    bench_redb_lock_contention_open(c, &source_project_dir);
    bench_redb_corrupt_check_graphdb(c, &source_project_dir);
    bench_redb_scan_discover_diff(c, &source_project_dir);
    bench_redb_incremental_parse_apply(c, &source_project_dir);
    bench_redb_incremental_persist_commit(c, &source_project_dir);
    bench_redb_incremental_persist_dirty_spg(c, &source_project_dir);
}

criterion_group! {
    name = redb_persistence_benches;
    config = real_project_criterion_config();
    targets = bench_redb_persistence_scenarios
}
criterion_main!(redb_persistence_benches);
