#[path = "common/bench_config.rs"]
mod bench_config;
#[path = "common/crud.rs"]
mod crud;
#[path = "common/first_existing_target.rs"]
mod first_existing_target;
#[path = "common/real_project.rs"]
mod real_project;
#[path = "common/restore_file.rs"]
mod restore_file;
#[path = "common/runtime_load.rs"]
mod runtime_load;
#[path = "common/runtime_request.rs"]
mod runtime_request;
#[path = "common/sandbox_create.rs"]
mod sandbox_create;

use anyhow::{Context, Result};
use bench_config::real_project_criterion_config;
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use crud::{
    append_update_data_action, explain_condition_contains_expr, find_page_result_contains,
    graph_has_node, graph_missing_node, json_value_contains_str, run_post_mutation_pipeline,
    update_component_visibility_condition,
};
use first_existing_target::first_existing_target;
use metadata_checker::scanner::indexer::ProjectIndexer;
use metadata_checker::tool_contract::ToolCommand;
use real_project::require_real_project_dir;
use restore_file::restore_file;
use runtime_load::load_warm_runtime;
use runtime_request::runtime_query_request;
use sandbox_create::{create_indexed_workspace, create_workspace};
use std::cell::Cell;
use std::path::Path;

const CONTRACT_PAGE_REL: &str = "app/销售.app/销售/合同协议.spg";
const CONTRACT_PAGE_FILE: &str = "app/销售.app/销售/合同协议.spg";
const MEMBER_REGISTERED_PAGE_REL: &str = "app/售后.app/绑定车辆/会员已注册.spg";
const MEMBER_REGISTERED_PAGE_FILE: &str = "app/售后.app/绑定车辆/会员已注册.spg";
const AUTO_CUSTOMER_REL_TBL_FILE: &str = "data/tables/主数据/fact_autoCustomerAutoRel.tbl";

const CONTRACT_TEXT45: &str = "comp:app/销售.app/销售/合同协议.spg|text45";
fn write_dirty_variant(path: &Path, original: &[u8], iteration: usize) -> Result<()> {
    let mut content = original.to_vec();
    content.push(b'\n');
    content.extend(std::iter::repeat(b' ').take((iteration % 31) + 1));
    std::fs::write(path, content).with_context(|| format!("write dirty variant {}", path.display()))
}

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
fn bench_noop_rebuild(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("noop-rebuild", source_project_dir).expect("create workspace");
    c.bench_function("rebuild_noop_existing_graphdb", |bench| {
        bench.iter(|| {
            let report = ProjectIndexer::scan(
                black_box(&workspace.project_dir),
                black_box(&workspace.db_path),
            )
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

/// 衡量单个真实 SPG 文件变化后的增量 rebuild 成本。
fn bench_dirty_spg_rebuild(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("dirty-spg", source_project_dir).expect("create workspace");
    let target = first_existing_target(
        &workspace.project_dir,
        &[CONTRACT_PAGE_FILE, MEMBER_REGISTERED_PAGE_FILE],
    )
    .expect("real project should contain a target SPG");
    let original = std::fs::read(&target).expect("read target SPG");
    let iteration = Cell::new(0_usize);

    c.bench_function("rebuild_dirty_single_spg", |bench| {
        bench.iter_batched(
            || {
                let step = iteration.get() + 1;
                iteration.set(step);
                write_dirty_variant(&target, &original, step).expect("write dirty SPG");
            },
            |_| {
                let report = ProjectIndexer::scan(
                    black_box(&workspace.project_dir),
                    black_box(&workspace.db_path),
                )
                .expect("dirty SPG rebuild should succeed");
                assert!(report.dirty >= 1, "dirty SPG rebuild should mark dirty");
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量单个真实 TBL 文件变化后的增量 rebuild 成本。
fn bench_dirty_tbl_rebuild(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("dirty-tbl", source_project_dir).expect("create workspace");
    let target = first_existing_target(&workspace.project_dir, &[AUTO_CUSTOMER_REL_TBL_FILE])
        .expect("real project should contain a target TBL");
    let original = std::fs::read(&target).expect("read target TBL");
    let iteration = Cell::new(0_usize);

    c.bench_function("rebuild_dirty_single_tbl", |bench| {
        bench.iter_batched(
            || {
                let step = iteration.get() + 1;
                iteration.set(step);
                write_dirty_variant(&target, &original, step).expect("write dirty TBL");
            },
            |_| {
                let report = ProjectIndexer::scan(
                    black_box(&workspace.project_dir),
                    black_box(&workspace.db_path),
                )
                .expect("dirty TBL rebuild should succeed");
                assert!(report.dirty >= 1, "dirty TBL rebuild should mark dirty");
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量删除一个真实元数据文件后的增量 rebuild 成本。
fn bench_deleted_file_rebuild(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("deleted-file", source_project_dir).expect("create workspace");
    let target = first_existing_target(&workspace.project_dir, &[MEMBER_REGISTERED_PAGE_FILE])
        .expect("real project should contain a target file");
    let original = std::fs::read(&target).expect("read target file");

    c.bench_function("rebuild_deleted_single_file", |bench| {
        bench.iter_batched(
            || {
                restore_file(&target, &original).expect("restore target file");
                ProjectIndexer::scan(&workspace.project_dir, &workspace.db_path)
                    .expect("restore baseline scan should succeed");
                std::fs::remove_file(&target).expect("remove target file");
            },
            |_| {
                let report = ProjectIndexer::scan(
                    black_box(&workspace.project_dir),
                    black_box(&workspace.db_path),
                )
                .expect("deleted file rebuild should succeed");
                assert!(report.deleted >= 1, "deleted rebuild should mark deletion");
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量新增一个真实元数据文件后的增量 rebuild 成本。
fn bench_added_file_rebuild(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("added-file", source_project_dir).expect("create workspace");
    let target = first_existing_target(&workspace.project_dir, &[MEMBER_REGISTERED_PAGE_FILE])
        .expect("real project should contain a target file");
    let original = std::fs::read(&target).expect("read target file");

    c.bench_function("rebuild_added_single_file", |bench| {
        bench.iter_batched(
            || {
                if target.exists() {
                    std::fs::remove_file(&target).expect("remove target file");
                }
                ProjectIndexer::scan(&workspace.project_dir, &workspace.db_path)
                    .expect("delete baseline scan should succeed");
                restore_file(&target, &original).expect("restore target as added file");
            },
            |_| {
                let report = ProjectIndexer::scan(
                    black_box(&workspace.project_dir),
                    black_box(&workspace.db_path),
                )
                .expect("added file rebuild should succeed");
                assert!(report.dirty >= 1, "added rebuild should mark dirty");
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量结构化可见性修改后的 scan -> check_reload -> explain_condition 闭环成本。
fn bench_crud_update_component_visibility_then_query(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("crud-visibility", source_project_dir).expect("create workspace");
    let page_path = first_existing_target(&workspace.project_dir, &[CONTRACT_PAGE_FILE])
        .expect("real project should contain contract page");
    let original = std::fs::read(&page_path).expect("read contract page");
    let mut runtime = load_warm_runtime(&workspace).expect("load warm runtime");
    let query = runtime_query_request(
        ToolCommand::ExplainCondition,
        CONTRACT_TEXT45,
        "compact",
        Some("display"),
        None,
        false,
    );
    let iteration = Cell::new(0_usize);

    c.bench_function("crud_update_component_visibility_then_query", |bench| {
        bench.iter_batched(
            || {
                restore_file(&page_path, &original).expect("restore contract page");
                ProjectIndexer::scan(&workspace.project_dir, &workspace.db_path)
                    .expect("restore baseline scan");
                let step = iteration.get() + 1;
                iteration.set(step);
                let visible_condition = format!("input3.value='metadata_checker_bench_{step}'");
                update_component_visibility_condition(&page_path, "text45", &visible_condition)
                    .expect("update component visibility");
            },
            |_| {
                let visible_condition =
                    format!("input3.value='metadata_checker_bench_{}'", iteration.get());
                let response = run_post_mutation_pipeline(&workspace, &mut runtime, query.clone())
                    .expect("visibility CRUD pipeline should succeed");
                let result = &response.result;
                assert!(
                    result.get("ok").and_then(|value| value.as_bool()) != Some(false),
                    "explain_condition after visibility update should succeed"
                );
                assert!(
                    explain_condition_contains_expr(result, &visible_condition),
                    "explain_condition should reflect updated visibleCondition"
                );
                assert!(
                    graph_has_node(&runtime, &format!("comp:{CONTRACT_PAGE_REL}|text45"),),
                    "target component node should remain in graph after visibility update"
                );
                black_box(response);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量追加写模型动作后的 scan -> check_reload -> query_model 闭环成本。
fn bench_crud_add_action_write_then_query_model(c: &mut Criterion, source_project_dir: &Path) {
    let workspace = create_indexed_workspace("crud-action-write", source_project_dir)
        .expect("create workspace");
    let page_path = first_existing_target(&workspace.project_dir, &[CONTRACT_PAGE_FILE])
        .expect("real project should contain contract page");
    let original = std::fs::read(&page_path).expect("read contract page");
    let mut runtime = load_warm_runtime(&workspace).expect("load warm runtime");
    let iteration = Cell::new(0_usize);

    c.bench_function("crud_add_action_write_then_query_model", |bench| {
        bench.iter_batched(
            || {
                restore_file(&page_path, &original).expect("restore contract page");
                ProjectIndexer::scan(&workspace.project_dir, &workspace.db_path)
                    .expect("restore baseline scan");
                let step = iteration.get() + 1;
                iteration.set(step);
                let action_id = format!("bench_action_write_{step}");
                append_update_data_action(
                    &page_path, "button1", &action_id, "model9", "send", "'1'",
                )
                .expect("append updateData action");
                action_id
            },
            |action_id| {
                let explain_target = format!("action:{CONTRACT_PAGE_REL}|button1|{action_id}");
                let query = runtime_query_request(
                    ToolCommand::Explain,
                    &explain_target,
                    "compact",
                    None,
                    None,
                    false,
                );
                let action_node_id = format!("action:{CONTRACT_PAGE_REL}|button1|{action_id}");
                let response = run_post_mutation_pipeline(&workspace, &mut runtime, query)
                    .expect("action write CRUD pipeline should succeed");
                assert_eq!(
                    response.result.get("kind").and_then(|value| value.as_str()),
                    Some("Explain"),
                    "post-mutation explain on new action should succeed"
                );
                assert!(
                    graph_has_node(&runtime, &action_node_id),
                    "graph should contain newly appended action node {action_node_id}"
                );
                assert!(
                    json_value_contains_str(&response.result, "updateData"),
                    "explain result should surface appended updateData action semantics"
                );
                black_box(response);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 衡量删除页面后的 scan -> check_reload -> find_page 闭环成本。
fn bench_crud_delete_page_then_find_page(c: &mut Criterion, source_project_dir: &Path) {
    let workspace =
        create_indexed_workspace("crud-delete-page", source_project_dir).expect("create workspace");
    let page_path = first_existing_target(&workspace.project_dir, &[MEMBER_REGISTERED_PAGE_FILE])
        .expect("real project should contain member registered page");
    let original = std::fs::read(&page_path).expect("read member registered page");
    let mut runtime = load_warm_runtime(&workspace).expect("load warm runtime");
    let query = runtime_query_request(
        ToolCommand::FindPage,
        "会员已注册",
        "compact",
        None,
        None,
        false,
    );

    let baseline = runtime
        .query(query.clone())
        .expect("baseline find_page should succeed");
    assert!(
        find_page_result_contains(&baseline.result, MEMBER_REGISTERED_PAGE_FILE),
        "baseline find_page should include member registered page"
    );

    let deleted_page_id = format!("page:{MEMBER_REGISTERED_PAGE_REL}");
    assert!(
        graph_has_node(&runtime, &deleted_page_id),
        "baseline graph should contain member registered page node"
    );

    c.bench_function("crud_delete_page_then_find_page", |bench| {
        bench.iter_batched(
            || {
                restore_file(&page_path, &original).expect("restore member registered page");
                ProjectIndexer::scan(&workspace.project_dir, &workspace.db_path)
                    .expect("restore baseline scan");
                std::fs::remove_file(&page_path).expect("delete member registered page");
            },
            |_| {
                let response = run_post_mutation_pipeline(&workspace, &mut runtime, query.clone())
                    .expect("delete page CRUD pipeline should succeed");
                assert!(
                    !find_page_result_contains(&response.result, MEMBER_REGISTERED_PAGE_FILE),
                    "find_page after page deletion should not return deleted page"
                );
                assert!(
                    graph_missing_node(&runtime, &deleted_page_id),
                    "deleted page node should be removed from graph"
                );
                black_box(response);
            },
            BatchSize::SmallInput,
        );
    });
}

/// 注册真实项目 rebuild benchmark。
fn bench_rebuild_scenarios(c: &mut Criterion) {
    let Some(source_project_dir) = require_real_project_dir("rebuild_bench") else {
        return;
    };

    bench_cold_build(c, &source_project_dir);
    bench_noop_rebuild(c, &source_project_dir);
    bench_dirty_spg_rebuild(c, &source_project_dir);
    bench_dirty_tbl_rebuild(c, &source_project_dir);
    bench_deleted_file_rebuild(c, &source_project_dir);
    bench_added_file_rebuild(c, &source_project_dir);
    bench_crud_update_component_visibility_then_query(c, &source_project_dir);
    bench_crud_add_action_write_then_query_model(c, &source_project_dir);
    bench_crud_delete_page_then_find_page(c, &source_project_dir);
}

criterion_group! {
    name = rebuild_benches;
    config = real_project_criterion_config();
    targets = bench_rebuild_scenarios
}
criterion_main!(rebuild_benches);
