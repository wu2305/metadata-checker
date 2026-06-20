use super::runtime_exec::run_runtime_query;
use criterion::{Criterion, black_box};
use metadata_checker::runtime::{GraphRuntime, RuntimeQueryRequest};

/// 注册单个 warm runtime 查询 benchmark。
pub fn bench_runtime_query(
    c: &mut Criterion,
    runtime: &mut GraphRuntime,
    name: &str,
    query: RuntimeQueryRequest,
) {
    c.bench_function(name, |bench| {
        bench.iter(|| {
            let response = run_runtime_query(runtime, black_box(query.clone()))
                .expect("runtime query benchmark should succeed");
            black_box(response);
        });
    });
}
