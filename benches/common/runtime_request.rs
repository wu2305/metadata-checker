use metadata_checker::runtime::RuntimeQueryRequest;
use metadata_checker::tool_contract::ToolCommand;

/// 构造带目标与预算的 runtime 查询请求。
pub fn runtime_query_request(
    command: ToolCommand,
    target: &str,
    budget: &str,
    intent: Option<&str>,
    depth: Option<usize>,
    check_reload: bool,
) -> RuntimeQueryRequest {
    RuntimeQueryRequest {
        command,
        target: target.to_string(),
        budget: budget.to_string(),
        human: false,
        intent: intent.map(ToOwned::to_owned),
        page_scope: None,
        depth,
        check_reload,
    }
}
