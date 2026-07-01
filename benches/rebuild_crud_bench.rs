#[path = "common/bench_config.rs"]
mod bench_config;
#[path = "common/crud.rs"]
mod crud;
#[path = "common/first_existing_target.rs"]
mod first_existing_target;
#[path = "common/mutation_setup.rs"]
mod mutation_setup;
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

use bench_config::real_project_criterion_config;
use criterion::{Criterion, criterion_group, criterion_main};
use crud::{
    append_update_data_action, explain_condition_contains_expr, find_page_result_contains,
    graph_has_node, graph_missing_node, json_value_contains_str, register_crud_mutation_bench,
    run_post_mutation_pipeline, update_component_visibility_condition,
};
use first_existing_target::first_existing_target;
use metadata_checker::tool_contract::ToolCommand;
use real_project::require_real_project_dir;
use runtime_load::load_warm_runtime;
use runtime_request::runtime_query_request;
use sandbox_create::create_indexed_workspace;
use std::sync::{Arc, Mutex};

const CONTRACT_PAGE_REL: &str = "app/销售.app/销售/合同协议.spg";
const CONTRACT_PAGE_FILE: &str = "app/销售.app/销售/合同协议.spg";
const MEMBER_REGISTERED_PAGE_REL: &str = "app/售后.app/绑定车辆/会员已注册.spg";
const MEMBER_REGISTERED_PAGE_FILE: &str = "app/售后.app/绑定车辆/会员已注册.spg";
const CONTRACT_TEXT45: &str = "comp:app/销售.app/销售/合同协议.spg|text45";

/// 衡量结构化可见性修改后的 scan -> check_reload -> explain_condition 闭环成本。
fn bench_crud_update_component_visibility_then_query(
    c: &mut Criterion,
    source_project_dir: &std::path::Path,
) {
    let workspace =
        create_indexed_workspace("crud-visibility", source_project_dir).expect("create workspace");
    let page_path = first_existing_target(&workspace.project_dir, &[CONTRACT_PAGE_FILE])
        .expect("real project should contain contract page");
    let original = std::fs::read(&page_path).expect("read contract page");
    let visibility_query = runtime_query_request(
        ToolCommand::ExplainCondition,
        CONTRACT_TEXT45,
        "compact",
        Some("display"),
        None,
        false,
    );

    register_crud_mutation_bench(
        c,
        workspace,
        page_path,
        original,
        "crud_update_component_visibility_then_query",
        |step, _workspace, page_path| {
            let visible_condition = format!("input3.value='metadata_checker_bench_{step}'");
            update_component_visibility_condition(page_path, "text45", &visible_condition)
                .expect("update component visibility");
            Ok(())
        },
        move |workspace, runtime, step| {
            let visible_condition = format!("input3.value='metadata_checker_bench_{step}'");
            let response =
                run_post_mutation_pipeline(workspace, runtime, visibility_query.clone())?;
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
                graph_has_node(runtime, &format!("comp:{CONTRACT_PAGE_REL}|text45")),
                "target component node should remain in graph after visibility update"
            );
            Ok(())
        },
    );
}

/// 衡量追加写模型动作后的 scan -> check_reload -> explain 闭环成本。
fn bench_crud_add_action_write_then_query_model(
    c: &mut Criterion,
    source_project_dir: &std::path::Path,
) {
    let workspace = create_indexed_workspace("crud-action-write", source_project_dir)
        .expect("create workspace");
    let page_path = first_existing_target(&workspace.project_dir, &[CONTRACT_PAGE_FILE])
        .expect("real project should contain contract page");
    let original = std::fs::read(&page_path).expect("read contract page");
    let action_id = Arc::new(Mutex::new(String::new()));
    let action_id_for_setup = action_id.clone();

    register_crud_mutation_bench(
        c,
        workspace,
        page_path,
        original,
        "crud_add_action_write_then_query_model",
        move |step, _workspace, page_path| {
            let next_action_id = format!("bench_action_write_{step}");
            append_update_data_action(
                page_path,
                "button1",
                &next_action_id,
                "model9",
                "send",
                "'1'",
            )
            .expect("append updateData action");
            *action_id_for_setup.lock().expect("action id lock") = next_action_id;
            Ok(())
        },
        move |workspace, runtime, _step| {
            let action_id = action_id.lock().expect("action id lock").clone();
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
            let response = run_post_mutation_pipeline(workspace, runtime, query)?;
            assert_eq!(
                response.result.get("kind").and_then(|value| value.as_str()),
                Some("Explain"),
                "post-mutation explain on new action should succeed"
            );
            assert!(
                graph_has_node(runtime, &action_node_id),
                "graph should contain newly appended action node {action_node_id}"
            );
            assert!(
                json_value_contains_str(&response.result, "updateData"),
                "explain result should surface appended updateData action semantics"
            );
            Ok(())
        },
    );
}

/// 衡量删除页面后的 scan -> check_reload -> find_page 闭环成本。
fn bench_crud_delete_page_then_find_page(c: &mut Criterion, source_project_dir: &std::path::Path) {
    let workspace =
        create_indexed_workspace("crud-delete-page", source_project_dir).expect("create workspace");
    let page_path = first_existing_target(&workspace.project_dir, &[MEMBER_REGISTERED_PAGE_FILE])
        .expect("real project should contain member registered page");
    let original = std::fs::read(&page_path).expect("read member registered page");
    let find_page_query = runtime_query_request(
        ToolCommand::FindPage,
        "会员已注册",
        "compact",
        None,
        None,
        false,
    );
    let deleted_page_id = format!("page:{MEMBER_REGISTERED_PAGE_REL}");
    {
        let mut runtime = load_warm_runtime(&workspace).expect("load warm runtime");
        let baseline = runtime
            .query(find_page_query.clone())
            .expect("baseline find_page should succeed");
        assert!(
            find_page_result_contains(&baseline.result, MEMBER_REGISTERED_PAGE_FILE),
            "baseline find_page should include member registered page"
        );
        assert!(
            graph_has_node(&runtime, &deleted_page_id),
            "baseline graph should contain member registered page node"
        );
    }

    register_crud_mutation_bench(
        c,
        workspace,
        page_path,
        original,
        "crud_delete_page_then_find_page",
        |_step, _workspace, page_path| {
            std::fs::remove_file(page_path).expect("delete member registered page");
            Ok(())
        },
        move |workspace, runtime, _step| {
            let response = run_post_mutation_pipeline(workspace, runtime, find_page_query.clone())?;
            assert!(
                !find_page_result_contains(&response.result, MEMBER_REGISTERED_PAGE_FILE),
                "find_page after page deletion should not return deleted page"
            );
            assert!(
                graph_missing_node(runtime, &deleted_page_id),
                "deleted page node should be removed from graph"
            );
            Ok(())
        },
    );
}

/// 注册真实项目 CRUD mutation benchmark。
fn bench_rebuild_crud_scenarios(c: &mut Criterion) {
    let Some(source_project_dir) = require_real_project_dir("rebuild_crud_bench") else {
        return;
    };

    bench_crud_update_component_visibility_then_query(c, &source_project_dir);
    bench_crud_add_action_write_then_query_model(c, &source_project_dir);
    bench_crud_delete_page_then_find_page(c, &source_project_dir);
}

criterion_group! {
    name = rebuild_crud_benches;
    config = real_project_criterion_config();
    targets = bench_rebuild_crud_scenarios
}
criterion_main!(rebuild_crud_benches);
