use anyhow::{Context, Result};
use metadata_checker::runtime::{
    GraphRuntime, ReloadResult, RuntimeQueryRequest, RuntimeQueryResponse,
};

/// 执行可选 check_reload 的 runtime 查询。
pub fn run_runtime_query(
    runtime: &mut GraphRuntime,
    request: RuntimeQueryRequest,
) -> Result<RuntimeQueryResponse> {
    if request.check_reload {
        match runtime
            .reload_if_changed()
            .context("check_reload before query")?
        {
            ReloadResult::Reloaded | ReloadResult::Unchanged => {}
            ReloadResult::ReloadFailed { error } => {
                anyhow::bail!("check_reload failed before query: {error}");
            }
        }
    }
    runtime.query(request).context("runtime query benchmark")
}
