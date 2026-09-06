#[path = "common/bench_config.rs"]
mod bench_config;
#[path = "common/dirty_file.rs"]
mod dirty_file;
#[path = "common/external_graph_lock.rs"]
mod external_graph_lock;
#[path = "common/first_existing_target.rs"]
mod first_existing_target;
#[path = "common/graphdb_fixture.rs"]
mod graphdb_fixture;
#[path = "common/real_project.rs"]
mod real_project;
#[path = "common/restore_file.rs"]
mod restore_file;
#[path = "common/sandbox_create.rs"]
mod sandbox_create;

use bench_config::real_project_criterion_config;
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use dirty_file::write_dirty_variant;
use external_graph_lock::acquire_external_graph_lock;
use first_existing_target::first_existing_target;
use graphdb_fixture::corrupt_graphdb_header;
use metadata_checker::graph::{GraphDB, set_graph_lock_timeout_ms};
use metadata_checker::graph_store::IndexCommit;
use metadata_checker::scanner::indexer::ProjectIndexer;
use metadata_checker::storage_provider::LocalStorageProvider;
use real_project::require_real_project_dir;
use restore_file::restore_file;
use sandbox_create::create_indexed_workspace;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;

const CONTRACT_PAGE_FILE: &str = "app/销售.app/销售/合同协议.spg";

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
                    checkpoint: None,
                    delta: None,
                    scanner_entries: Vec::new(),
                    scanner_deleted_paths: Vec::new(),
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

/// M54 Task 5：dirty/deleted 文件规模曲线（合成项目，scan 批删路径）。
///
/// 合成 120 个最小 SPG 的临时项目，不依赖真实项目目录；
/// 分别采集 dirty 1/5/20/100 与 deleted 1/5/20 的增量 scan 耗时。
fn bench_redb_batch_delete_curve(c: &mut Criterion) {
    const FILE_COUNT: usize = 120;
    let workspace = create_batch_curve_workspace(FILE_COUNT).expect("create batch curve workspace");

    for &dirty_count in &[1_usize, 5, 20, 100] {
        let mut iteration = 0_usize;
        let bench_id = format!("redb_batch_delete_curve_dirty_{dirty_count}");
        c.bench_function(&bench_id, |bench| {
            bench.iter_batched(
                || {
                    iteration += 1;
                    restore_batch_curve_baseline(&workspace);
                    for index in 0..dirty_count {
                        std::fs::write(
                            workspace.project_dir.join(format!("page_{index:03}.spg")),
                            synthetic_spg(index, iteration),
                        )
                        .expect("write dirty spg");
                    }
                },
                |_| {
                    let report = ProjectIndexer::scan(
                        black_box(&workspace.project_dir),
                        black_box(&workspace.db_path),
                    )
                    .expect("batch delete curve scan");
                    // persist 路径的 report.dirty 是脏节点数而非文件数，只断言非零
                    assert!(
                        report.dirty > 0,
                        "dirty curve scan should record dirty nodes"
                    );
                    black_box(report);
                },
                BatchSize::SmallInput,
            );
        });
    }

    for &deleted_count in &[1_usize, 5, 20] {
        let bench_id = format!("redb_batch_delete_curve_deleted_{deleted_count}");
        c.bench_function(&bench_id, |bench| {
            bench.iter_batched(
                || {
                    restore_batch_curve_baseline(&workspace);
                    for index in 0..deleted_count {
                        std::fs::remove_file(
                            workspace.project_dir.join(format!("page_{index:03}.spg")),
                        )
                        .expect("remove spg for delete curve");
                    }
                },
                |_| {
                    let report = ProjectIndexer::scan(
                        black_box(&workspace.project_dir),
                        black_box(&workspace.db_path),
                    )
                    .expect("batch delete curve scan");
                    // persist 路径的 report.deleted 是删除节点数而非文件数，只断言非零
                    assert!(
                        report.deleted > 0,
                        "deleted curve scan should record deleted nodes"
                    );
                    black_box(report);
                },
                BatchSize::SmallInput,
            );
        });
    }
}

/// 规模曲线用最小 SPG 内容；`salt` 用于生成内容不同的 dirty 变体。
fn synthetic_spg(index: usize, salt: usize) -> String {
    serde_json::json!({
        "version": "4.19.7",
        "theme": "default",
        "canvas": {
            "id": "canvas",
            "type": "canvas",
            "components": [
                {"id": format!("input{index}_{salt}"), "type": "input"}
            ]
        }
    })
    .to_string()
}

/// 规模曲线合成工作区：项目目录、graphdb 与基线快照。
struct BatchCurveWorkspace {
    project_dir: std::path::PathBuf,
    db_path: std::path::PathBuf,
    baseline_db: Vec<u8>,
    originals: Vec<String>,
}

/// 创建合成工作区并完成首次全量索引，保存 graphdb 基线字节。
fn create_batch_curve_workspace(file_count: usize) -> std::io::Result<BatchCurveWorkspace> {
    let root = std::env::temp_dir().join(format!(
        "metadata-checker-m54-batch-curve-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let project_dir = root.join("project");
    std::fs::create_dir_all(&project_dir)?;
    let originals: Vec<String> = (0..file_count)
        .map(|index| synthetic_spg(index, 0))
        .collect();
    for (index, original) in originals.iter().enumerate() {
        std::fs::write(project_dir.join(format!("page_{index:03}.spg")), original)?;
    }
    let db_path = root.join("graph.redb");
    ProjectIndexer::scan(&project_dir, &db_path).expect("index synthetic workspace");
    let baseline_db = std::fs::read(&db_path)?;
    Ok(BatchCurveWorkspace {
        project_dir,
        db_path,
        baseline_db,
        originals,
    })
}

/// 恢复基线：重置 graphdb 字节并把全部原始 SPG 写回（覆盖 dirty 变体、补回已删文件）。
fn restore_batch_curve_baseline(workspace: &BatchCurveWorkspace) {
    std::fs::write(&workspace.db_path, &workspace.baseline_db).expect("restore baseline graphdb");
    for (index, original) in workspace.originals.iter().enumerate() {
        std::fs::write(
            workspace.project_dir.join(format!("page_{index:03}.spg")),
            original,
        )
        .expect("restore original spg");
    }
}

/// M56 Task 13：topology-dirty 持久化基线（合成项目，scan 批删路径）。
///
/// (a) metadata-only dirty：组件 id 稳定、只改静态属性；
/// (b) topology dirty：增删组件（单 SPG 与 20 SPG）。
/// 当前实现下 scanner remove/re-add 恒置 topology_dirty，
/// 预期两组都走 v2 full layout——该事实即 M56 验收门的基线。
fn bench_redb_topology_dirty_baseline(c: &mut Criterion) {
    const FILE_COUNT: usize = 120;
    let workspace = create_batch_curve_workspace(FILE_COUNT).expect("create batch curve workspace");

    let mut iteration = 0_usize;
    c.bench_function("redb_topology_baseline_meta_only_1", |bench| {
        bench.iter_batched(
            || {
                iteration += 1;
                restore_batch_curve_baseline(&workspace);
                std::fs::write(
                    workspace.project_dir.join("page_000.spg"),
                    synthetic_spg_meta_variant(0, iteration),
                )
                .expect("write meta-only dirty spg");
            },
            |_| {
                let report = ProjectIndexer::scan(
                    black_box(&workspace.project_dir),
                    black_box(&workspace.db_path),
                )
                .expect("meta-only baseline scan");
                assert!(report.dirty > 0);
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });

    for &(label, dirty_count) in &[("topology_1", 1_usize), ("topology_20", 20_usize)] {
        let mut iteration = 0_usize;
        let bench_id = format!("redb_topology_baseline_{label}");
        c.bench_function(&bench_id, |bench| {
            bench.iter_batched(
                || {
                    iteration += 1;
                    restore_batch_curve_baseline(&workspace);
                    for index in 0..dirty_count {
                        std::fs::write(
                            workspace.project_dir.join(format!("page_{index:03}.spg")),
                            synthetic_spg(index, iteration),
                        )
                        .expect("write topology dirty spg");
                    }
                },
                |_| {
                    let report = ProjectIndexer::scan(
                        black_box(&workspace.project_dir),
                        black_box(&workspace.db_path),
                    )
                    .expect("topology baseline scan");
                    assert!(report.dirty > 0);
                    black_box(report);
                },
                BatchSize::SmallInput,
            );
        });
    }
}

/// metadata-only 变体：组件 id 稳定，只改静态属性（不改组件结构）。
fn synthetic_spg_meta_variant(index: usize, salt: usize) -> String {
    serde_json::json!({
        "version": "4.19.7",
        "theme": "default",
        "canvas": {
            "id": "canvas",
            "type": "canvas",
            "components": [
                {"id": format!("input{index}"), "type": "input", "placeholder": format!("hint {salt}")}
            ]
        }
    })
    .to_string()
}

/// M56 Task 15：增量 persist 规模曲线（链式 12000 节点图，delta 路径）。
///
/// 测量区只含 `persist_commit`；restore/open/mutate 在 setup 完成。
/// 对照组 `redb_persist_curve_full_10` 同图同 dirty 规模走全量路径。
fn bench_redb_incremental_persist_curve(c: &mut Criterion) {
    const TOTAL_NODES: usize = 12_000;
    let fixture = create_persist_curve_fixture(TOTAL_NODES).expect("create persist curve fixture");

    for &dirty_count in &[10_usize, 100, 1000, 10000] {
        let bench_id = format!("redb_persist_curve_delta_{dirty_count}");
        c.bench_function(&bench_id, |bench| {
            bench.iter_batched(
                || prepare_dirty_commit(&fixture, dirty_count),
                |(mut graph, commit)| {
                    let report = graph.persist_commit(&commit).expect("delta persist");
                    assert_eq!(report.full_rewrite, false);
                    black_box(report);
                },
                BatchSize::SmallInput,
            );
        });
    }

    c.bench_function("redb_persist_curve_full_10", |bench| {
        bench.iter_batched(
            || prepare_dirty_commit(&fixture, 10),
            |(mut graph, mut commit)| {
                commit.delta = None;
                let report = graph.persist_commit(&commit).expect("full persist");
                assert_eq!(report.full_rewrite, true);
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 曲线 fixture：链式图基线（全量 Current persist 后的 db 字节）。
struct PersistCurveFixture {
    db_path: std::path::PathBuf,
    baseline_db: Vec<u8>,
    node_ids: Vec<String>,
}

/// 构建链式图基线：node_i -Reads→ node_{i+1}，全量 persist 后保存 db 字节。
fn create_persist_curve_fixture(total_nodes: usize) -> std::io::Result<PersistCurveFixture> {
    use metadata_checker::graph::{EdgeType, GraphDB, Node, NodeType};
    use metadata_checker::graph_store::GraphWriteStore;

    let root = std::env::temp_dir().join(format!(
        "metadata-checker-m56-persist-curve-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;
    let db_path = root.join("graph.redb");

    let node_ids: Vec<String> = (0..total_nodes).map(|i| format!("node:{i}")).collect();
    {
        let mut graph = GraphDB::open(&db_path).expect("create curve graph");
        for id in &node_ids {
            GraphWriteStore::upsert_node(
                &mut graph,
                Node {
                    id: id.clone(),
                    node_type: NodeType::Component,
                    path: "bench/curve.spg".to_string(),
                    name: id.clone(),
                    meta: None,
                },
            )
            .expect("add curve node");
        }
        for pair in node_ids.windows(2) {
            graph.add_edge(&pair[0], &pair[1], EdgeType::Reads, None);
        }
        graph
            .persist(&std::collections::HashMap::new())
            .expect("baseline full persist");
    }
    let baseline_db = std::fs::read(&db_path)?;
    Ok(PersistCurveFixture {
        db_path,
        baseline_db,
        node_ids,
    })
}

/// bench setup：restore → open → remove+re-add 前 k 个节点（模拟 scanner dirty），
/// 构造 delta commit。测量区只负责 persist_commit。
fn prepare_dirty_commit(
    fixture: &PersistCurveFixture,
    dirty_count: usize,
) -> (metadata_checker::graph::GraphDB, IndexCommit) {
    use metadata_checker::graph::Edge;
    use metadata_checker::graph_redb::edge_storage_key;
    use metadata_checker::graph_store::{GraphReadStore, GraphWriteStore, IndexDelta};

    std::fs::write(&fixture.db_path, &fixture.baseline_db).expect("restore baseline db");
    let mut graph = GraphDB::open(&fixture.db_path).expect("open curve graph");
    let dirty_ids: Vec<String> = fixture.node_ids[..dirty_count].to_vec();

    let mut removed_edge_keys = std::collections::HashSet::new();
    let mut removed_nodes = Vec::new();
    let mut removed_edges: Vec<Edge> = Vec::new();
    for id in &dirty_ids {
        let node = GraphReadStore::get_node(&graph, id)
            .expect("get curve node")
            .expect("curve node exists");
        removed_nodes.push(node);
        if let Some(neighbors) = GraphReadStore::get_node_edges(&graph, id).expect("curve edges") {
            for view in neighbors.outgoing.iter().chain(neighbors.incoming.iter()) {
                removed_edge_keys.insert(edge_storage_key(&view.edge));
                removed_edges.push(view.edge.clone());
            }
        }
    }

    graph.remove_nodes_by_ids(&dirty_ids);
    for node in removed_nodes {
        GraphWriteStore::upsert_node(&mut graph, node).expect("re-add curve node");
    }
    let mut seen_edges = std::collections::HashSet::new();
    let mut dirty_edges = Vec::new();
    for edge in removed_edges {
        graph.add_edge_with_meta(
            &edge.from,
            &edge.to,
            edge.edge_type.clone(),
            edge.field_path.clone(),
            edge.meta.clone(),
        );
        if seen_edges.insert(edge_storage_key(&edge)) {
            dirty_edges.push(edge);
        }
    }

    let commit = IndexCommit {
        file_states: std::collections::HashMap::new(),
        dirty_nodes: graph.dirty_nodes_set().iter().cloned().collect(),
        deleted_nodes: graph.removed_nodes_set().iter().cloned().collect(),
        checkpoint: None,
        delta: Some(IndexDelta {
            dirty_edges,
            removed_edge_keys: removed_edge_keys.into_iter().collect(),
            changed_file_states: Vec::new(),
            removed_file_paths: Vec::new(),
        }),
        scanner_entries: Vec::new(),
        scanner_deleted_paths: Vec::new(),
    };
    (graph, commit)
}

/// 规模曲线 Criterion 配置：与真实项目 bench 相同的样本数，更短的测量窗口。
fn batch_curve_criterion_config() -> Criterion {
    Criterion::default()
        .sample_size(10)
        .warm_up_time(std::time::Duration::from_secs(1))
        .measurement_time(std::time::Duration::from_secs(3))
}

criterion_group! {
    name = redb_persistence_benches;
    config = real_project_criterion_config();
    targets = bench_redb_persistence_scenarios
}
criterion_group! {
    name = batch_delete_curve_benches;
    config = batch_curve_criterion_config();
    targets = bench_redb_batch_delete_curve, bench_redb_topology_dirty_baseline, bench_redb_incremental_persist_curve
}
criterion_main!(redb_persistence_benches, batch_delete_curve_benches);
