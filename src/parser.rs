use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::superpage;

/// 文件解析入口模块
///
/// 自动识别文件类型（SuperPage / 旧版 Table）并调用对应解析器：
/// - .spg 结尾 → SuperPage 解析
/// - 其他 → 尝试旧版 Table 解析
///
/// 同时提供统一的 PageMetadata 结构，兼容两种输入格式。
/// Parsed metadata summary extracted from the low-code platform JSON.
#[derive(Debug, Default)]
pub struct PageMetadata {
    pub input_path: Option<String>,
    pub page_id: Option<String>,
    pub page_name: Option<String>,
    pub version: Option<String>,
    pub components: Vec<ComponentInfo>,
    pub data_bindings: Vec<DataBinding>,
    pub settings: HashMap<String, Value>,
    pub raw: Value,
    /// SuperPage specific metadata
    pub superpage: Option<superpage::SuperPageMetadata>,
}

#[derive(Debug, Default)]
pub struct ComponentInfo {
    pub id: Option<String>,
    pub component_type: Option<String>,
    pub title: Option<String>,
    pub db_field: Option<String>,
}

#[derive(Debug, Default)]
pub struct DataBinding {
    pub source_id: Option<String>,
    pub path: Option<String>,
    pub filter: Option<String>,
}

/// 根据文件类型自动选择解析器
pub fn parse_file(path: &Path) -> Result<PageMetadata> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;
    let raw: Value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse JSON from: {}", path.display()))?;

    let mut meta = PageMetadata {
        input_path: Some(path.display().to_string()),
        ..PageMetadata::default()
    };

    // Detect if it's a SuperPage by checking for "canvas" field
    let is_superpage = raw.get("canvas").is_some();

    if is_superpage {
        meta.superpage = Some(superpage::parse_superpage_from_value(raw.clone())?);
        // Also set basic fields
        if let Some(obj) = raw.as_object() {
            meta.version = obj
                .get("version")
                .and_then(|v| v.as_str())
                .map(String::from);
        }
    } else {
        // Original parsing logic for non-superpage JSON
        if let Some(obj) = raw.as_object() {
            meta.version = obj
                .get("version")
                .and_then(|v| v.as_str())
                .map(String::from);

            // Handle "forms" wrapper (fapp-style)
            if let Some(forms_val) = obj.get("forms")
                && let Some(forms_obj) = forms_val.as_object()
            {
                meta.version = forms_obj
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or(meta.version);
                if let Some(forms_arr) = forms_obj.get("forms").and_then(|v| v.as_array()) {
                    for form in forms_arr {
                        if let Some(form_obj) = form.as_object() {
                            let page_id = form_obj
                                .get("id")
                                .and_then(|v| v.as_str())
                                .map(String::from);
                            meta.page_id = meta.page_id.or(page_id);
                            if let Some(components) =
                                form_obj.get("components").and_then(|v| v.as_array())
                            {
                                for comp in components {
                                    meta.components.push(extract_component(comp));
                                }
                            }
                        }
                    }
                    if let Some(params) = forms_obj.get("params").and_then(|v| v.as_array()) {
                        for param in params {
                            if let Some(pobj) = param.as_object() {
                                meta.data_bindings.push(DataBinding {
                                    source_id: pobj
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .map(String::from),
                                    path: pobj
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .map(String::from),
                                    filter: pobj
                                        .get("value")
                                        .and_then(|v| v.as_str())
                                        .map(String::from),
                                });
                            }
                        }
                    }
                }
            }

            // Handle "settings"
            if let Some(settings) = obj.get("settings")
                && let Some(smap) = settings.as_object()
            {
                for (k, v) in smap {
                    meta.settings.insert(k.clone(), v.clone());
                }
            }

            // Handle workflow-style "nodes"
            if let Some(nodes) = obj.get("nodes").and_then(|v| v.as_array()) {
                for node in nodes {
                    let mut comp = ComponentInfo::default();
                    if let Some(nobj) = node.as_object() {
                        comp.id = nobj.get("id").and_then(|v| v.as_str()).map(String::from);
                        comp.component_type =
                            nobj.get("type").and_then(|v| v.as_str()).map(String::from);
                        comp.title = nobj.get("desc").and_then(|v| v.as_str()).map(String::from);
                        meta.components.push(comp);
                    }
                }
            }
        }
    }

    meta.raw = raw;
    Ok(meta)
}

fn extract_component(value: &Value) -> ComponentInfo {
    let mut comp = ComponentInfo::default();
    if let Some(obj) = value.as_object() {
        comp.id = obj.get("id").and_then(|v| v.as_str()).map(String::from);
        comp.component_type = obj.get("type").and_then(|v| v.as_str()).map(String::from);
        comp.title = obj
            .get("title")
            .and_then(|v| v.get("text"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| obj.get("title").and_then(|v| v.as_str()).map(String::from));
        comp.db_field = obj
            .get("dbfield")
            .and_then(|v| v.as_str())
            .map(String::from);
    }
    comp
}
