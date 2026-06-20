#[path = "common/bench_config.rs"]
mod bench_config;
#[path = "common/real_project.rs"]
mod real_project;
#[path = "common/sandbox_create.rs"]
mod sandbox_create;

use bench_config::real_project_criterion_config;
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use metadata_checker::runtime::GraphRuntime;
use metadata_checker::stdio_server::{dispatch_stdio_line, serialize_stdio_response};
use real_project::require_real_project_dir;
use sandbox_create::create_indexed_workspace;

const CONTRACT_PAGE: &str = "page:app/销售.app/销售/合同协议.spg";

fn bench_stdio_invalid_json(c: &mut Criterion, runtime: &mut GraphRuntime) {
    c.bench_function("stdio_invalid_json_line", |bench| {
        bench.iter(|| {
            let mut response = dispatch_stdio_line(runtime, black_box("{not-json"));
            assert_eq!(
                response.error.as_ref().map(|error| error.code.as_str()),
                Some("INVALID_JSON")
            );
            let _ =
                serialize_stdio_response(&mut response).expect("serialize invalid json response");
            black_box(response);
        });
    });
}

fn bench_stdio_unknown_command(c: &mut Criterion, runtime: &mut GraphRuntime) {
    let line = r#"{"request_id":"bench-unknown","command":"definitely_not_a_command"}"#;
    c.bench_function("stdio_unknown_command", |bench| {
        bench.iter(|| {
            let mut response = dispatch_stdio_line(runtime, black_box(line));
            assert_eq!(
                response.error.as_ref().map(|error| error.code.as_str()),
                Some("UNKNOWN_COMMAND")
            );
            let _ = serialize_stdio_response(&mut response)
                .expect("serialize unknown command response");
            black_box(response);
        });
    });
}

fn bench_stdio_query_page_logic_full_large(c: &mut Criterion, runtime: &mut GraphRuntime) {
    let line = format!(
        r#"{{"request_id":"bench-full","command":"query_page_logic","target":"{CONTRACT_PAGE}","budget":"full"}}"#
    );
    c.bench_function("stdio_query_page_logic_full_large", |bench| {
        bench.iter(|| {
            let mut response = dispatch_stdio_line(runtime, black_box(line.as_str()));
            assert!(response.ok, "full page logic stdio query should succeed");
            let serialized = serialize_stdio_response(&mut response)
                .expect("serialize full page logic response");
            assert!(
                serialized.len() > 10_000,
                "full page logic output should be large, got {} bytes",
                serialized.len()
            );
            black_box(serialized);
        });
    });
}

fn bench_stdio_error_then_continue(c: &mut Criterion, runtime: &mut GraphRuntime) {
    let invalid = r#"{"request_id":"bench-bad","command":"nope"}"#;
    let valid = r#"{"request_id":"bench-ok","command":"status"}"#;
    c.bench_function("stdio_error_then_status_continue", |bench| {
        bench.iter_batched(
            || {},
            |_| {
                let bad = dispatch_stdio_line(runtime, invalid);
                assert!(!bad.ok, "unknown command should fail");
                let mut ok = dispatch_stdio_line(runtime, valid);
                assert!(ok.ok, "status after error should still succeed");
                let serialized =
                    serialize_stdio_response(&mut ok).expect("serialize status response");
                black_box((bad, serialized));
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_stdio_boundary_scenarios(c: &mut Criterion) {
    let Some(source_project_dir) = require_real_project_dir("stdio_boundary_bench") else {
        return;
    };
    let workspace =
        create_indexed_workspace("stdio-boundary", &source_project_dir).expect("create workspace");
    let mut runtime =
        GraphRuntime::load_with_project_dir(&workspace.db_path, Some(&workspace.project_dir))
            .expect("runtime load should succeed");

    bench_stdio_invalid_json(c, &mut runtime);
    bench_stdio_unknown_command(c, &mut runtime);
    bench_stdio_query_page_logic_full_large(c, &mut runtime);
    bench_stdio_error_then_continue(c, &mut runtime);
}

criterion_group! {
    name = stdio_boundary_benches;
    config = real_project_criterion_config();
    targets = bench_stdio_boundary_scenarios
}
criterion_main!(stdio_boundary_benches);
