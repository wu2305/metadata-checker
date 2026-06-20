use criterion::{Criterion, black_box, criterion_group, criterion_main};
use metadata_checker::dependency::DependencyGraph;
use metadata_checker::parser;
use metadata_checker::priority;
use metadata_checker::superpage::{self, parse_expression_refs};
use std::path::PathBuf;

/// 返回 `tests/fixtures` 下的稳定 fixture 路径。
fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
}

/// 衡量 fixture SPG 文件解析成本。
fn bench_parse_spg(c: &mut Criterion, fixture: &str, bench_name: &str) {
    let path = fixture_path(fixture);
    c.bench_function(bench_name, |bench| {
        bench.iter(|| {
            let meta = parser::parse_file(black_box(&path)).expect("parse SPG fixture");
            black_box(meta);
        });
    });
}

/// 衡量 fixture TBL 文件解析成本。
fn bench_parse_tbl(c: &mut Criterion, fixture: &str, bench_name: &str) {
    let path = fixture_path(fixture);
    c.bench_function(bench_name, |bench| {
        bench.iter(|| {
            let meta = parser::parse_file(black_box(&path)).expect("parse TBL fixture");
            black_box(meta);
        });
    });
}

/// 衡量 SuperPage 依赖图构建成本。
fn bench_dependency_graph(c: &mut Criterion, fixture: &str, bench_name: &str) {
    let path = fixture_path(fixture);
    let meta = parser::parse_file(&path)
        .expect("parse fixture for dependency graph")
        .superpage
        .expect("fixture should be SPG");
    c.bench_function(bench_name, |bench| {
        bench.iter(|| {
            let graph = DependencyGraph::new(black_box(&meta));
            black_box(graph);
        });
    });
}

/// 衡量计算优先级分析成本。
fn bench_priority_analysis(c: &mut Criterion, fixture: &str, bench_name: &str) {
    let path = fixture_path(fixture);
    let meta = parser::parse_file(&path)
        .expect("parse fixture for priority analysis")
        .superpage
        .expect("fixture should be SPG");
    c.bench_function(bench_name, |bench| {
        bench.iter(|| {
            let analyses = priority::analyze_priority(black_box(&meta));
            black_box(analyses);
        });
    });
}

/// 衡量复杂表达式引用提取成本。
fn bench_expr_ref_parse(c: &mut Criterion) {
    let path = fixture_path("complex_expressions.spg");
    let meta = superpage::parse_superpage(&path).expect("parse complex expressions fixture");
    let expressions: Vec<String> = meta
        .expressions
        .iter()
        .map(|expr| expr.raw_expr.clone())
        .collect();
    c.bench_function("parse_expr_refs_complex_expressions", |bench| {
        bench.iter(|| {
            for expr in &expressions {
                let refs = parse_expression_refs(black_box(expr));
                black_box(refs);
            }
        });
    });
}

/// 注册 fixture 解析与单文件分析微基准。
fn bench_parse_scenarios(c: &mut Criterion) {
    bench_parse_spg(c, "large_page.spg", "parse_spg_large_page");
    bench_parse_spg(c, "actions_test.spg", "parse_spg_actions_test");
    bench_parse_spg(c, "real_world_1.spg", "parse_spg_real_world_1");
    bench_parse_tbl(c, "dataflow_table.tbl", "parse_tbl_dataflow_table");
    bench_parse_tbl(
        c,
        "test_project/app/real_dataflow.tbl",
        "parse_tbl_real_dataflow",
    );
    bench_dependency_graph(c, "large_page.spg", "dependency_graph_large_page");
    bench_dependency_graph(c, "actions_test.spg", "dependency_graph_actions_test");
    bench_priority_analysis(c, "large_page.spg", "priority_analysis_large_page");
    bench_priority_analysis(
        c,
        "test_conditions.spg",
        "priority_analysis_test_conditions",
    );
    bench_expr_ref_parse(c);
}

criterion_group!(parse_benches, bench_parse_scenarios);
criterion_main!(parse_benches);
