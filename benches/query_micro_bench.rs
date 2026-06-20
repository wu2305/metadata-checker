#[path = "common/synthetic.rs"]
mod synthetic;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use metadata_checker::context;
use metadata_checker::explain;
use metadata_checker::query;
use synthetic::{
    build_deep_tree_page_graph, build_dense_model_context_graph, build_fanout_page_graph,
};

/// 衡量深层组件树上的 page logic 查询成本。
fn bench_query_page_logic_deep_tree(c: &mut Criterion, leaf_count: usize, max_depth: usize) {
    let store = build_deep_tree_page_graph(leaf_count, max_depth);
    let bench_name = format!("query_page_logic_deep_tree_fanout_{leaf_count}_depth_{max_depth}");
    c.bench_function(&bench_name, |bench| {
        bench.iter(|| {
            let output = query::build_query_page_logic_output(
                black_box(&store),
                black_box("page:app/home.spg"),
                None,
                black_box("compact"),
            )
            .expect("deep tree page logic benchmark should succeed");
            black_box(output);
        });
    });
}

/// 衡量高密度模型扇出图上的 context 查询成本。
fn bench_context_dense_model(c: &mut Criterion, fanout: usize, depth: usize) {
    let (store, target) = build_dense_model_context_graph(fanout);
    let bench_name = format!("context_dense_model_depth{depth}_fanout_{fanout}");
    c.bench_function(&bench_name, |bench| {
        bench.iter(|| {
            let output = context::build_context_output(
                black_box(&store),
                black_box(target.as_str()),
                depth,
                black_box("normal"),
            )
            .expect("dense model context benchmark should succeed");
            black_box(output);
        });
    });
}

/// 衡量 page logic 在不同 fanout 下的内部查询成本。
fn bench_query_page_logic_fanout(c: &mut Criterion, fanout: usize) {
    let store = build_fanout_page_graph(fanout);
    let bench_name = format!("query_page_logic_fanout_{fanout}");
    c.bench_function(&bench_name, |bench| {
        bench.iter(|| {
            let output = query::build_query_page_logic_output(
                black_box(&store),
                black_box("page:app/home.spg"),
                None,
                black_box("compact"),
            )
            .expect("query_page_logic benchmark should succeed");
            black_box(output);
        });
    });
}

/// 衡量 page 关系查询在合成图上的成本。
fn bench_query_page_fanout(c: &mut Criterion, fanout: usize) {
    let store = build_fanout_page_graph(fanout);
    let bench_name = format!("query_page_fanout_{fanout}");
    c.bench_function(&bench_name, |bench| {
        bench.iter(|| {
            let output =
                query::build_query_page_output(black_box(&store), black_box("page:app/home.spg"))
                    .expect("query_page benchmark should succeed");
            black_box(output);
        });
    });
}

/// 衡量 explain 在合成组件图上的成本。
fn bench_explain_component_fanout(c: &mut Criterion, fanout: usize) {
    let store = build_fanout_page_graph(fanout);
    let target = format!("comp:app/home.spg|input{}", fanout / 2);
    let bench_name = format!("explain_component_fanout_{fanout}");
    c.bench_function(&bench_name, |bench| {
        bench.iter(|| {
            let output =
                explain::build_explain_output(black_box(&store), black_box(target.as_str()))
                    .expect("explain benchmark should succeed");
            black_box(output);
        });
    });
}

/// 注册合成图 query 微基准。
fn bench_query_micro_scenarios(c: &mut Criterion) {
    for fanout in [64_usize, 128, 256] {
        bench_query_page_logic_fanout(c, fanout);
        bench_query_page_fanout(c, fanout);
    }
    bench_explain_component_fanout(c, 256);
    bench_query_page_logic_deep_tree(c, 1024, 8);
    bench_context_dense_model(c, 512, 3);
    bench_dataflow_pathology_scenarios(c);
}

#[path = "common/synthetic_dataflow.rs"]
mod synthetic_dataflow;

use synthetic_dataflow::build_pathological_dataflow_graph;

/// 衡量病理 DataFlow 在 full budget 下的输出膨胀成本。
fn bench_dataflow_pathology_full_output(c: &mut Criterion, output_fields: usize) {
    let (store, model_id) = build_pathological_dataflow_graph(output_fields, 4, 6);
    let bench_name = format!("query_dataflow_pathology_outputs_{output_fields}_full");
    c.bench_function(&bench_name, |bench| {
        bench.iter(|| {
            let output = metadata_checker::query::build_query_dataflow_output(
                black_box(&store),
                black_box(model_id.as_str()),
            )
            .expect("pathology dataflow benchmark should succeed");
            let serialized = serde_json::to_string(&output).expect("serialize dataflow output");
            assert!(
                serialized.len() > output_fields * 80,
                "pathology dataflow full output should scale with field count"
            );
            black_box(serialized);
        });
    });
}

/// 衡量多 Join / Union / Filter 叠加的 DataFlow 展开成本。
fn bench_dataflow_pathology_topology(c: &mut Criterion, join_layers: usize, filter_count: usize) {
    let (store, model_id) = build_pathological_dataflow_graph(24, join_layers, filter_count);
    let bench_name =
        format!("query_dataflow_pathology_joins_{join_layers}_filters_{filter_count}_compact");
    c.bench_function(&bench_name, |bench| {
        bench.iter(|| {
            let output = metadata_checker::query::build_query_dataflow_output(
                black_box(&store),
                black_box(model_id.as_str()),
            )
            .expect("pathology topology dataflow benchmark should succeed");
            black_box(output);
        });
    });
}

fn bench_dataflow_pathology_scenarios(c: &mut Criterion) {
    bench_dataflow_pathology_full_output(c, 64);
    bench_dataflow_pathology_topology(c, 6, 8);
}

criterion_group!(query_micro_benches, bench_query_micro_scenarios);
criterion_main!(query_micro_benches);
