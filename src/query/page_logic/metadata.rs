use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// 从页面原始 .spg 文件读取出的补充元数据
#[derive(Clone)]
pub(super) struct PageFileMetadata {
    pub(super) page_inputs: Vec<serde_json::Value>,
    pub(super) visibility_rules: Vec<serde_json::Value>,
    pub(super) from_file: bool,
    pub(super) component_json_paths: HashMap<String, String>,
    pub(super) action_meta: HashMap<String, serde_json::Value>,
}

#[derive(Clone)]
struct CachedPageFileMeta {
    mtime: SystemTime,
    size: u64,
    metadata: PageFileMetadata,
}

// ponytail: process-local mtime/size cache；上限=无跨进程/无 scanner 索引；升级=索引阶段物化进 RuntimeReadModel。
fn page_file_meta_cache() -> &'static Mutex<HashMap<PathBuf, CachedPageFileMeta>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedPageFileMeta>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 加载页面文件中的参数、组件路径、可见性规则和 action 原始属性。
///
/// 同进程内按绝对路径 + mtime/size 复用解析结果，避免每次 warm 重复读盘解析。
pub(super) fn load_page_file_metadata(
    project_dir: Option<&std::path::Path>,
    page_path: &str,
) -> PageFileMetadata {
    let empty = || PageFileMetadata {
        page_inputs: Vec::new(),
        visibility_rules: Vec::new(),
        from_file: false,
        component_json_paths: HashMap::new(),
        action_meta: HashMap::new(),
    };

    let Some(proj_dir) = project_dir else {
        return empty();
    };
    let file_path = proj_dir.join(page_path);
    if !file_path.exists() {
        return empty();
    }
    let Ok(file_meta) = std::fs::metadata(&file_path) else {
        return empty();
    };
    let Ok(mtime) = file_meta.modified() else {
        return parse_page_file(&file_path, page_path);
    };
    let size = file_meta.len();

    if let Ok(cache) = page_file_meta_cache().lock() {
        if let Some(hit) = cache.get(&file_path) {
            if hit.mtime == mtime && hit.size == size {
                return hit.metadata.clone();
            }
        }
    }

    let metadata = parse_page_file(&file_path, page_path);
    if metadata.from_file {
        if let Ok(mut cache) = page_file_meta_cache().lock() {
            cache.insert(
                file_path,
                CachedPageFileMeta {
                    mtime,
                    size,
                    metadata: metadata.clone(),
                },
            );
        }
    }
    metadata
}

fn parse_page_file(file_path: &std::path::Path, page_path: &str) -> PageFileMetadata {
    let mut metadata = PageFileMetadata {
        page_inputs: Vec::new(),
        visibility_rules: Vec::new(),
        from_file: false,
        component_json_paths: HashMap::new(),
        action_meta: HashMap::new(),
    };

    let Ok(content) = std::fs::read_to_string(file_path) else {
        return metadata;
    };
    let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&content) else {
        return metadata;
    };

    metadata.from_file = true;
    if let Some(params) = json_val.get("params").and_then(|p| p.as_array()) {
        for p in params {
            metadata.page_inputs.push(json!({
                "id": p.get("id"),
                "name": p.get("name"),
                "type": "page_param",
            }));
        }
    }

    if let Some(components) = json_val
        .get("canvas")
        .and_then(|c| c.get("components"))
        .and_then(|c| c.as_array())
    {
        collect_components(
            components,
            "canvas.components",
            page_path,
            &mut metadata.component_json_paths,
            &mut metadata.visibility_rules,
            &mut metadata.action_meta,
        );
    }

    metadata
}

fn collect_components(
    arr: &[serde_json::Value],
    path_prefix: &str,
    source_file: &str,
    component_json_paths: &mut HashMap<String, String>,
    visibility_rules: &mut Vec<serde_json::Value>,
    action_meta: &mut HashMap<String, serde_json::Value>,
) {
    for (index, comp) in arr.iter().enumerate() {
        let comp_id = comp.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let component_path = format!("{}[{}]", path_prefix, index);
        if !comp_id.is_empty() {
            component_json_paths.insert(comp_id.to_string(), component_path.clone());
        }
        for prop in ["visible", "hidden", "disabled", "readonly"] {
            if let Some(val) = comp.get(prop) {
                let mut expr_struct =
                    crate::action_semantics::build_expression_struct(val.as_str());
                let mut rule = json!({
                    "component_id": comp_id,
                    "rule": prop,
                    "expression": val,
                    "source_file": source_file,
                    "json_path": format!("{}.{}", component_path, prop),
                });
                if let Some(obj) = expr_struct.as_object_mut() {
                    for key in [
                        "raw_expr",
                        "refs",
                        "resolved_refs",
                        "unresolved_refs",
                        "ambiguous_refs",
                        "diagnostics",
                        "confidence",
                    ] {
                        rule[key] = obj.remove(key).unwrap_or(serde_json::Value::Null);
                    }
                }
                visibility_rules.push(rule);
            }
        }

        if let Some(actions) = comp.get("actions").and_then(|a| a.as_array()) {
            for (action_index, act) in actions.iter().enumerate() {
                if let Some(aid) = act.get("id").and_then(|v| v.as_str()) {
                    let trigger_type = act
                        .get("triggerType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("click");
                    let wait_prev = act.get("waitPrev").cloned();
                    let condition = act
                        .get("condition")
                        .or_else(|| act.get("conditionExp"))
                        .cloned();
                    let action_path = format!("{}.actions[{}]", component_path, action_index);
                    action_meta.insert(
                        format!("{}|{}", comp_id, aid),
                        json!({
                            "trigger_type": trigger_type,
                            "wait_prev": wait_prev,
                            "condition": condition,
                            "json_path": action_path,
                            "source_file": source_file,
                        }),
                    );
                }
            }
        }

        for nested_key in ["components", "panels", "steps", "comps"] {
            if let Some(nested) = comp.get(nested_key).and_then(|v| v.as_array()) {
                collect_components(
                    nested,
                    &format!("{}.{}", component_path, nested_key),
                    source_file,
                    component_json_paths,
                    visibility_rules,
                    action_meta,
                );
            }
        }
    }
}
