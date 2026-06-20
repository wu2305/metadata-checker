#[path = "common/bench_config.rs"]
mod bench_config;
#[path = "common/real_project.rs"]
mod real_project;
#[path = "common/runtime_exec.rs"]
mod runtime_exec;
#[path = "common/runtime_register.rs"]
mod runtime_register;
#[path = "common/runtime_request.rs"]
mod runtime_request;
#[path = "common/sandbox_create.rs"]
mod sandbox_create;

use bench_config::real_project_criterion_config;
use criterion::{Criterion, criterion_group, criterion_main};
use metadata_checker::runtime::GraphRuntime;
use metadata_checker::tool_contract::ToolCommand;
use real_project::require_real_project_dir;
use runtime_register::bench_runtime_query;
use runtime_request::runtime_query_request;
use sandbox_create::create_indexed_workspace;

const CONTRACT_PAGE: &str = "page:app/销售.app/销售/合同协议.spg";
const MEMBER_REGISTERED_PAGE: &str = "page:app/售后.app/绑定车辆/会员已注册.spg";
const QUERY_CROSS_TWO_PAGES: &str =
    "page:app/销售.app/销售/合同协议.spg,page:app/售后.app/绑定车辆/会员已注册.spg";
const INPUT3: &str = "comp:app/销售.app/销售/合同协议.spg|input3";
const MISSING_INPUT: &str = "comp:app/销售.app/销售/合同协议.spg|__bench_missing_component__";
const TEXT41: &str = "comp:app/售后.app/绑定车辆/会员已注册.spg|text41";
const MODEL11: &str = "model:app/售后.app/绑定车辆/会员已注册.spg|model11";
const BINDING_CAR_DATAFLOW_MODEL: &str = "model:绑车";
const FACT_QW_SIDEBAR_MODEL: &str = "model:fact_qwSidebar";
const WIDE_FIND_KEYWORD: &str = "销售";

/// 注册真实项目查询矩阵 benchmark。
fn bench_query_matrix(c: &mut Criterion) {
    let Some(source_project_dir) = require_real_project_dir("query_matrix_bench") else {
        return;
    };
    let workspace =
        create_indexed_workspace("query-matrix", &source_project_dir).expect("create workspace");
    let mut runtime =
        GraphRuntime::load_with_project_dir(&workspace.db_path, Some(&workspace.project_dir))
            .expect("runtime load should succeed");

    let scenarios = [
        (
            "query_page_contract_compact",
            runtime_query_request(
                ToolCommand::QueryPage,
                CONTRACT_PAGE,
                "compact",
                None,
                None,
                false,
            ),
        ),
        (
            "query_page_contract_normal",
            runtime_query_request(
                ToolCommand::QueryPage,
                CONTRACT_PAGE,
                "normal",
                None,
                None,
                false,
            ),
        ),
        (
            "query_page_member_registered_compact",
            runtime_query_request(
                ToolCommand::QueryPage,
                MEMBER_REGISTERED_PAGE,
                "compact",
                None,
                None,
                false,
            ),
        ),
        (
            "query_cross_contract_member_compact",
            runtime_query_request(
                ToolCommand::QueryCross,
                QUERY_CROSS_TWO_PAGES,
                "compact",
                None,
                None,
                false,
            ),
        ),
        (
            "query_dataflow_binding_car_compact",
            runtime_query_request(
                ToolCommand::QueryDataflow,
                BINDING_CAR_DATAFLOW_MODEL,
                "compact",
                None,
                None,
                false,
            ),
        ),
        (
            "query_model_fact_qw_sidebar_compact",
            runtime_query_request(
                ToolCommand::QueryModel,
                FACT_QW_SIDEBAR_MODEL,
                "compact",
                None,
                None,
                false,
            ),
        ),
        (
            "query_model_fact_qw_sidebar_normal",
            runtime_query_request(
                ToolCommand::QueryModel,
                FACT_QW_SIDEBAR_MODEL,
                "normal",
                None,
                None,
                false,
            ),
        ),
        (
            "explain_text41_compact",
            runtime_query_request(ToolCommand::Explain, TEXT41, "compact", None, None, false),
        ),
        (
            "explain_condition_input3_writer_compact",
            runtime_query_request(
                ToolCommand::ExplainCondition,
                INPUT3,
                "compact",
                Some("writer"),
                None,
                false,
            ),
        ),
        (
            "explain_condition_input3_writer_normal",
            runtime_query_request(
                ToolCommand::ExplainCondition,
                INPUT3,
                "normal",
                Some("writer"),
                None,
                false,
            ),
        ),
        (
            "explain_condition_missing_target_compact",
            runtime_query_request(
                ToolCommand::ExplainCondition,
                MISSING_INPUT,
                "compact",
                Some("display"),
                None,
                false,
            ),
        ),
        (
            "explain_condition_text41_display_compact",
            runtime_query_request(
                ToolCommand::ExplainCondition,
                TEXT41,
                "compact",
                Some("display"),
                None,
                false,
            ),
        ),
        (
            "explain_condition_text41_value_source_compact",
            runtime_query_request(
                ToolCommand::ExplainCondition,
                TEXT41,
                "compact",
                Some("value-source"),
                None,
                false,
            ),
        ),
        (
            "explain_condition_model11_availability_compact",
            runtime_query_request(
                ToolCommand::ExplainCondition,
                MODEL11,
                "compact",
                Some("availability"),
                None,
                false,
            ),
        ),
        (
            "advise_query_input3_writer_compact",
            runtime_query_request(
                ToolCommand::AdviseQuery,
                INPUT3,
                "compact",
                Some("writer"),
                None,
                false,
            ),
        ),
        (
            "context_input3_depth2_normal",
            runtime_query_request(ToolCommand::Context, INPUT3, "normal", None, Some(2), false),
        ),
        (
            "context_input3_depth3_full",
            runtime_query_request(ToolCommand::Context, INPUT3, "full", None, Some(3), false),
        ),
        (
            "query_page_logic_contract_full",
            runtime_query_request(
                ToolCommand::QueryPageLogic,
                CONTRACT_PAGE,
                "full",
                None,
                None,
                false,
            ),
        ),
        (
            "query_page_logic_contract_normal",
            runtime_query_request(
                ToolCommand::QueryPageLogic,
                CONTRACT_PAGE,
                "normal",
                None,
                None,
                false,
            ),
        ),
        (
            "query_page_logic_member_registered_compact",
            runtime_query_request(
                ToolCommand::QueryPageLogic,
                MEMBER_REGISTERED_PAGE,
                "compact",
                None,
                None,
                false,
            ),
        ),
        (
            "explain_condition_text41_full",
            runtime_query_request(
                ToolCommand::ExplainCondition,
                TEXT41,
                "full",
                Some("display"),
                None,
                false,
            ),
        ),
        (
            "find_page_member_registered_compact",
            runtime_query_request(
                ToolCommand::FindPage,
                "会员已注册",
                "compact",
                None,
                None,
                false,
            ),
        ),
        (
            "find_page_wide_keyword_compact",
            runtime_query_request(
                ToolCommand::FindPage,
                WIDE_FIND_KEYWORD,
                "compact",
                None,
                None,
                false,
            ),
        ),
        (
            "find_model_auto_customer_rel_compact",
            runtime_query_request(
                ToolCommand::FindModel,
                "fact_autoCustomerAutoRel",
                "compact",
                None,
                None,
                false,
            ),
        ),
        (
            "find_component_text41_compact",
            runtime_query_request(
                ToolCommand::FindComponent,
                "text41",
                "compact",
                None,
                None,
                false,
            ),
        ),
        (
            "query_page_contract_compact_check_reload",
            runtime_query_request(
                ToolCommand::QueryPage,
                CONTRACT_PAGE,
                "compact",
                None,
                None,
                true,
            ),
        ),
        (
            "explain_condition_input3_writer_compact_check_reload",
            runtime_query_request(
                ToolCommand::ExplainCondition,
                INPUT3,
                "compact",
                Some("writer"),
                None,
                true,
            ),
        ),
    ];

    for (name, request) in scenarios {
        bench_runtime_query(c, &mut runtime, name, request);
    }
}

criterion_group! {
    name = query_matrix_benches;
    config = real_project_criterion_config();
    targets = bench_query_matrix
}
criterion_main!(query_matrix_benches);
