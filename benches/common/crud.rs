use crate::sandbox_create::BenchWorkspace;
use anyhow::{Context, Result, bail, ensure};
use metadata_checker::runtime::{
    GraphRuntime, ReloadResult, RuntimeQueryRequest, RuntimeQueryResponse,
};
use metadata_checker::scanner::indexer::ProjectIndexer;
use serde_json::{Value, json};
use std::path::Path;

/// 读取并解析元数据 JSON。
pub fn read_metadata_json(path: &Path) -> Result<Value> {
    let content =
        std::fs::read(path).with_context(|| format!("read metadata file {}", path.display()))?;
    serde_json::from_slice(&content)
        .with_context(|| format!("parse metadata JSON {}", path.display()))
}

/// 将元数据 JSON 写回文件。
pub fn write_metadata_json(path: &Path, value: &Value) -> Result<()> {
    let content = serde_json::to_vec(value)
        .with_context(|| format!("serialize metadata JSON {}", path.display()))?;
    std::fs::write(path, content).with_context(|| format!("write metadata file {}", path.display()))
}

/// 在 JSON 元数据树中定位组件并执行结构化修改。
fn modify_component_by_id(
    value: &mut Value,
    target_id: &str,
    mutate: &mut dyn FnMut(&mut Value) -> Result<()>,
) -> Result<bool> {
    if let Value::Object(map) = value {
        if map.get("id").and_then(Value::as_str) == Some(target_id) {
            mutate(value)?;
            return Ok(true);
        }
        for child in map.values_mut() {
            if matches!(child, Value::Object(_) | Value::Array(_))
                && modify_component_by_id(child, target_id, mutate)?
            {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    if let Value::Array(items) = value {
        for item in items {
            if modify_component_by_id(item, target_id, mutate)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// 结构化修改组件可见性条件，触发依赖图增量更新。
pub fn update_component_visibility_condition(
    path: &Path,
    component_id: &str,
    visible_condition: &str,
) -> Result<()> {
    let mut root = read_metadata_json(path)?;
    let condition = visible_condition.to_string();
    let mut mutate = |component: &mut Value| -> Result<()> {
        let object = component
            .as_object_mut()
            .context("component node should be a JSON object")?;
        object.insert(
            "visible".to_string(),
            Value::String("condition".to_string()),
        );
        object.insert(
            "visibleCondition".to_string(),
            Value::String(condition.clone()),
        );
        Ok(())
    };
    ensure!(
        modify_component_by_id(&mut root, component_id, &mut mutate)?,
        "component {component_id} not found in {}",
        path.display()
    );
    write_metadata_json(path, &root)
}

/// 向组件追加 `updateData` 写模型动作。
pub fn append_update_data_action(
    path: &Path,
    component_id: &str,
    action_id: &str,
    model_id: &str,
    field_name: &str,
    field_value_exp: &str,
) -> Result<()> {
    let mut root = read_metadata_json(path)?;
    let action = json!({
        "id": action_id,
        "actionType": "updateData",
        "triggerType": "click",
        "dataSet": model_id,
        "dataRange": "resultset",
        "fieldValues": [
            {
                "name": field_name,
                "valueType": "exp",
                "value": field_value_exp
            }
        ]
    });
    let mut mutate = |component: &mut Value| -> Result<()> {
        let object = component
            .as_object_mut()
            .context("component node should be a JSON object")?;
        let actions = object
            .entry("actions")
            .or_insert_with(|| Value::Array(Vec::new()));
        let action_list = actions
            .as_array_mut()
            .context("component actions should be a JSON array")?;
        action_list.push(action.clone());
        Ok(())
    };
    ensure!(
        modify_component_by_id(&mut root, component_id, &mut mutate)?,
        "component {component_id} not found in {}",
        path.display()
    );
    write_metadata_json(path, &root)
}

/// 变更落盘后的标准闭环：`scan -> check_reload -> query`。
pub fn run_post_mutation_pipeline(
    workspace: &BenchWorkspace,
    runtime: &mut GraphRuntime,
    query: RuntimeQueryRequest,
) -> Result<RuntimeQueryResponse> {
    let report = ProjectIndexer::scan(&workspace.project_dir, &workspace.db_path)
        .context("post-mutation scan")?;
    ensure!(
        report.dirty > 0 || report.deleted > 0,
        "post-mutation scan should observe dirty or deleted metadata"
    );
    match runtime
        .reload_if_changed()
        .context("post-mutation check_reload")?
    {
        ReloadResult::Reloaded => {}
        ReloadResult::Unchanged => {
            bail!("graphdb changed but runtime check_reload reported unchanged")
        }
        ReloadResult::ReloadFailed { error } => {
            bail!("post-mutation check_reload failed: {error}")
        }
    }
    runtime.query(query).context("post-mutation business query")
}

/// 判断 find_page 结果中是否包含指定页面节点。
pub fn find_page_result_contains(result: &Value, page_file: &str) -> bool {
    find_page_matches(result)
        .iter()
        .any(|item| page_item_matches(item, page_file))
}

fn find_page_matches(result: &Value) -> Vec<&Value> {
    match result {
        Value::Object(map) => map
            .get("details")
            .and_then(|details| details.get("matches"))
            .and_then(Value::as_array)
            .map(|items| items.iter().collect())
            .unwrap_or_default(),
        Value::Array(items) => items.iter().collect(),
        _ => Vec::new(),
    }
}

fn page_item_matches(item: &Value, page_file: &str) -> bool {
    item.as_object().is_some_and(|map| {
        map.get("source_file")
            .and_then(Value::as_str)
            .is_some_and(|path| path == page_file || path.ends_with(page_file))
            || map
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.contains(page_file))
    })
}

/// 递归检查 JSON 值中是否包含指定字符串片段。
pub fn json_value_contains_str(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.contains(needle),
        Value::Array(items) => items
            .iter()
            .any(|item| json_value_contains_str(item, needle)),
        Value::Object(map) => map
            .values()
            .any(|item| json_value_contains_str(item, needle)),
        _ => false,
    }
}

/// 判断 explain_condition 结果是否包含指定表达式片段。
pub fn explain_condition_contains_expr(result: &Value, expected_substr: &str) -> bool {
    json_value_contains_str(result, expected_substr)
}

/// 判断图中是否存在指定节点。
pub fn graph_has_node(runtime: &GraphRuntime, node_id: &str) -> bool {
    runtime.graph.get_node(node_id).is_some()
}

/// 判断图中是否不存在指定节点。
pub fn graph_missing_node(runtime: &GraphRuntime, node_id: &str) -> bool {
    !graph_has_node(runtime, node_id)
}
