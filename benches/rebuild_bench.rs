#[path = "common/bench_config.rs"]
mod bench_config;
#[path = "common/dirty_file.rs"]
mod dirty_file;
#[path = "common/first_existing_target.rs"]
mod first_existing_target;
#[path = "common/incremental_scan.rs"]
mod incremental_scan;
#[path = "common/mutation_setup.rs"]
mod mutation_setup;
#[path = "common/real_project.rs"]
mod real_project;
#[path = "common/restore_file.rs"]
mod restore_file;
#[path = "common/sandbox_create.rs"]
mod sandbox_create;

use bench_config::real_project_criterion_config;
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use first_existing_target::first_existing_target;
use incremental_scan::register_dirty_file_incremental_scan_bench;
use metadata_checker::scanner::indexer::ProjectIndexer;
use mutation_setup::restore_metadata_baseline;
use real_project::require_real_project_dir;
use restore_file::restore_file;
use sandbox_create::{BenchWorkspace, create_indexed_workspace, create_workspace};
use std::path::Path;

const MEMBER_REGISTERED_PAGE_FILE: &str = "app/售后.app/绑定车辆/会员已注册.spg";
const AUTO_CUSTOMER_REL_TBL_FILE: &str = "data/tables/主数据/fact_autoCustomerAutoRel.tbl";

/// 衡量空 graphdb 上的真实项目冷构建成本。
fn bench_cold_build(c: &mut Criterion, source_project_dir: &Path) {
    c.bench_function("rebuild_cold_build_empty_graphdb", |bench| {
        bench.iter_batched(
            || create_workspace("cold-build", source_project_dir).expect("create workspace"),
            |workspace| {
                let report = ProjectIndexer::scan(
                    black_box(&workspace.project_dir),
                    black_box(&workspace.db_path),
                )
                .expect("cold build should succeed");
                assert!(report.dirty > 0, "cold build should parse dirty files");
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量无文件变化时 discover + hash + skip persistence 成本。
fn bench_noop_rebuild(c: &mut Criterion, workspace: &BenchWorkspace) {
    let project_dir = workspace.project_dir.clone();
    let db_path = workspace.db_path.clone();
    c.bench_function("rebuild_noop_existing_graphdb", |bench| {
        bench.iter(|| {
            let report = ProjectIndexer::scan(black_box(&project_dir), black_box(&db_path))
                .expect("noop rebuild should succeed");
            assert_eq!(report.dirty, 0, "noop rebuild should not mark dirty files");
            assert_eq!(
                report.deleted, 0,
                "noop rebuild should not mark deleted files"
            );
            black_box(report);
        });
    });
}

/// 衡量单个真实 TBL 文件变化后的增量 rebuild 成本。
fn bench_dirty_tbl_rebuild(c: &mut Criterion, workspace: &BenchWorkspace) {
    let target = first_existing_target(&workspace.project_dir, &[AUTO_CUSTOMER_REL_TBL_FILE])
        .expect("real project should contain a target TBL");
    let original = std::fs::read(&target).expect("read target TBL");
    register_dirty_file_incremental_scan_bench(
        c,
        workspace,
        &target,
        &original,
        "rebuild_dirty_single_tbl",
    );
}

/// 衡量删除一个真实元数据文件后的增量 rebuild 成本。
fn bench_deleted_file_rebuild(c: &mut Criterion, workspace: &BenchWorkspace, target: &Path) {
    let original = std::fs::read(target).expect("read target file");
    let project_dir = workspace.project_dir.clone();
    let db_path = workspace.db_path.clone();
    let target = target.to_path_buf();

    c.bench_function("rebuild_deleted_single_file", |bench| {
        bench.iter_batched(
            || {
                restore_metadata_baseline(workspace, &target, &original)
                    .expect("restore deleted-file baseline");
                std::fs::remove_file(&target).expect("remove target file");
            },
            |_| {
                let report = ProjectIndexer::scan(black_box(&project_dir), black_box(&db_path))
                    .expect("deleted file rebuild should succeed");
                assert!(report.deleted >= 1, "deleted rebuild should mark deletion");
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量新增一个真实元数据文件后的增量 rebuild 成本。
fn bench_added_file_rebuild(
    c: &mut Criterion,
    workspace: &BenchWorkspace,
    target: &Path,
    original: &[u8],
) {
    let original = original.to_vec();
    let project_dir = workspace.project_dir.clone();
    let db_path = workspace.db_path.clone();
    let target = target.to_path_buf();

    c.bench_function("rebuild_added_single_file", |bench| {
        bench.iter_batched(
            || {
                if target.exists() {
                    std::fs::remove_file(&target).expect("remove target file");
                }
                ProjectIndexer::scan(&project_dir, &db_path)
                    .expect("delete baseline scan should succeed");
                restore_file(&target, original.as_slice()).expect("restore target as added file");
            },
            |_| {
                let report = ProjectIndexer::scan(black_box(&project_dir), black_box(&db_path))
                    .expect("added file rebuild should succeed");
                assert!(report.dirty >= 1, "added rebuild should mark dirty");
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 注册真实项目 rebuild 核心 benchmark（不含 CRUD 与 dirty SPG；后者由 redb bench 覆盖）。
fn bench_rebuild_core_scenarios(c: &mut Criterion) {
    let Some(source_project_dir) = require_real_project_dir("rebuild_bench") else {
        return;
    };

    bench_cold_build(c, &source_project_dir);

    let noop_dirty_tbl_workspace = create_indexed_workspace("noop-dirty-tbl", &source_project_dir)
        .expect("create shared noop/dirty-tbl workspace");
    bench_noop_rebuild(c, &noop_dirty_tbl_workspace);
    bench_dirty_tbl_rebuild(c, &noop_dirty_tbl_workspace);

    let delete_add_workspace = create_indexed_workspace("delete-add", &source_project_dir)
        .expect("create shared delete/add workspace");
    let member_page = first_existing_target(
        &delete_add_workspace.project_dir,
        &[MEMBER_REGISTERED_PAGE_FILE],
    )
    .expect("real project should contain member registered page");
    let member_page_original =
        std::fs::read(&member_page).expect("read member page before delete/add benches");
    bench_deleted_file_rebuild(c, &delete_add_workspace, &member_page);
    bench_added_file_rebuild(
        c,
        &delete_add_workspace,
        &member_page,
        &member_page_original,
    );
}

criterion_group! {
    name = rebuild_benches;
    config = real_project_criterion_config();
    targets = bench_rebuild_core_scenarios
}
criterion_main!(rebuild_benches);
