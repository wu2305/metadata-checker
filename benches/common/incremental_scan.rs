use crate::dirty_file::write_dirty_variant;
use crate::sandbox_create::BenchWorkspace;
use criterion::{BatchSize, Criterion, black_box};
use metadata_checker::scanner::indexer::ProjectIndexer;
use std::cell::Cell;
use std::path::Path;

/// 注册「单文件 dirty + 全量 scan」增量 rebuild benchmark。
pub fn register_dirty_file_incremental_scan_bench(
    c: &mut Criterion,
    workspace: &BenchWorkspace,
    target: &Path,
    original: &[u8],
    bench_name: &'static str,
) {
    let iteration = Cell::new(0_usize);
    let target = target.to_path_buf();
    let original = original.to_vec();
    let project_dir = workspace.project_dir.clone();
    let db_path = workspace.db_path.clone();

    c.bench_function(bench_name, |bench| {
        bench.iter_batched(
            || {
                let step = iteration.get() + 1;
                iteration.set(step);
                write_dirty_variant(&target, &original, step).expect("write dirty metadata file");
            },
            |_| {
                let report = ProjectIndexer::scan(black_box(&project_dir), black_box(&db_path))
                    .expect("dirty metadata incremental scan should succeed");
                assert!(
                    report.dirty >= 1,
                    "dirty incremental scan should mark dirty files"
                );
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });
}
