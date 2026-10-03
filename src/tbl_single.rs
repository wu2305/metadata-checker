//! 单文件 .tbl 解析模块
//!
//! M37 起只承担结构化解析，human/JSON 输出已迁到 output::tbl。
//! 提供不依赖本地文件系统的 core API：parse_tbl_from_value_with_source。
//!
//! 负责在没有项目图数据库时，直接从单个 .tbl JSON 文件提取表/DataFlow 语义。

use crate::source_id::{ProjectRef, SourceId};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// 从 .tbl JSON 提取的字段信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TblField {
    pub name: String,
    pub dbfield: Option<String>,
    pub data_type: Option<String>,
    pub length: Option<i64>,
    pub is_dimension: bool,
    pub input_field: Option<String>,
    pub exp: Option<String>,
    pub original_field: Option<String>,
    pub original_node: Option<String>,
}

/// 单文件 .tbl 元数据
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TblMetadata {
    pub input_path: Option<String>,
    pub table_id: Option<String>,
    pub table_name: Option<String>,
    pub version: Option<String>,
    pub db_table_name: Option<String>,
    pub is_dataflow: bool,
    pub fields: Vec<TblField>,
    pub dataflow_inputs: Vec<DataFlowInput>,
    pub dataflow_outputs: Vec<DataFlowOutput>,
    pub internal_nodes: Vec<DataFlowNode>,
    pub raw: serde_json::Value,
}

/// DataFlow 输入源
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataFlowInput {
    pub node_id: String,
    pub node_type: String,
    pub alias: Option<String>,
    pub module_table_path: Option<String>,
}

/// DataFlow 输出目标
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataFlowOutput {
    pub db_table_name: Option<String>,
    pub field_count: usize,
}

/// DataFlow 内部节点
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataFlowNode {
    pub node_id: String,
    pub node_type: String,
    pub alias: Option<String>,
    pub input_nodes: Vec<String>,
    pub fields: Vec<TblField>,
}

/// 解析单个 .tbl 文件（native wrapper）。
///
/// M37 降级为 wrapper：创建 SourceId 后调用 core API。
/// M37 native-only：依赖 std::path::Path，不能进入 WASM core。
/// 本地 Path 相关逻辑只存在于本层，不进入 parse_tbl_from_value_with_source。
pub fn parse_tbl(path: &Path, raw: Value) -> Result<TblMetadata> {
    let source = SourceId::from_local_path(ProjectRef::new("default"), path, None)?;
    parse_tbl_from_value_with_source(&source, raw)
}

/// 纯解析入口，不依赖本地文件系统。
///
/// source 提供身份与展示信息；raw 为已反序列化的 JSON。
/// M37 core API，供 WASM/远程/测试直接调用。
pub fn parse_tbl_from_value_with_source(source: &SourceId, raw: Value) -> Result<TblMetadata> {
    let mut meta = TblMetadata {
        input_path: source
            .display_path
            .clone()
            .or_else(|| Some(source.source_path.clone())),
        raw: raw.clone(),
        ..Default::default()
    };

    let obj = raw.as_object();

    meta.version = obj
        .and_then(|o| o.get("version"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let props = obj
        .and_then(|o| o.get("properties"))
        .and_then(|v| v.as_object());
    // 空串是未落表数据流（没有物理输出表），与项目扫描口径一致，按「没有」处理。
    meta.db_table_name = props
        .and_then(|p| p.get("dbTableName"))
        .and_then(|v| v.as_str())
        .filter(|name| !name.is_empty())
        .map(String::from);

    meta.is_dataflow = raw.get("dataFlow").is_some();

    // Table name from properties.name or source_path basename
    let basename = std::path::Path::new(&source.source_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string());
    meta.table_name = props
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| basename.clone());

    // Table id: use dbTableName if available, else basename
    meta.table_id = meta.db_table_name.clone().or_else(|| basename);

    // Parse dimensions (fields)
    if let Some(dims) = raw.get("dimensions").and_then(|d| d.as_array()) {
        for (idx, dim) in dims.iter().enumerate() {
            if let Some(dim_obj) = dim.as_object() {
                let field = parse_dim_field(dim_obj, idx);
                meta.fields.push(field);
            }
        }
    }

    // Parse DataFlow nodes if present
    if meta.is_dataflow {
        if let Some(nodes) = raw
            .get("dataFlow")
            .and_then(|d| d.get("nodes"))
            .and_then(|n| n.as_object())
        {
            for (node_id, node_val) in nodes {
                if let Some(node_obj) = node_val.as_object() {
                    let node_type = node_obj
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    let alias = node_obj
                        .get("alias")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let module_table_path = node_obj
                        .get("moduleTablePath")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    let input_nodes: Vec<String> = node_obj
                        .get("inputNodes")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();

                    let mut node_fields = Vec::new();
                    if let Some(fields) = node_obj.get("fields").and_then(|v| v.as_array()) {
                        for (idx, f) in fields.iter().enumerate() {
                            if let Some(fobj) = f.as_object() {
                                node_fields.push(parse_dim_field(fobj, idx));
                            }
                        }
                    }

                    meta.internal_nodes.push(DataFlowNode {
                        node_id: node_id.clone(),
                        node_type: node_type.clone(),
                        alias: alias.clone(),
                        input_nodes,
                        fields: node_fields,
                    });

                    // Input nodes: ModelTable or FileInput
                    if node_type == "ModelTable"
                        || node_type == "modelTable"
                        || node_type == "FileInput"
                    {
                        meta.dataflow_inputs.push(DataFlowInput {
                            node_id: node_id.clone(),
                            node_type,
                            alias,
                            module_table_path,
                        });
                    }
                }
            }
        }

        // Output: physical table (dbTableName)
        if let Some(db_name) = &meta.db_table_name {
            meta.dataflow_outputs.push(DataFlowOutput {
                db_table_name: Some(db_name.clone()),
                field_count: meta.fields.len(),
            });
        }
    }

    Ok(meta)
}

fn parse_dim_field(dim_obj: &serde_json::Map<String, serde_json::Value>, _idx: usize) -> TblField {
    TblField {
        name: dim_obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        dbfield: dim_obj
            .get("dbfield")
            .and_then(|v| v.as_str())
            .map(String::from),
        data_type: dim_obj
            .get("dataType")
            .and_then(|v| v.as_str())
            .map(String::from),
        length: dim_obj.get("length").and_then(|v| v.as_i64()),
        is_dimension: dim_obj
            .get("isDimension")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        input_field: dim_obj
            .get("inputField")
            .and_then(|v| v.as_str())
            .map(String::from),
        exp: dim_obj
            .get("exp")
            .and_then(|v| v.as_str())
            .map(String::from),
        original_field: dim_obj
            .get("originalField")
            .and_then(|v| v.as_str())
            .map(String::from),
        original_node: dim_obj
            .get("originalNode")
            .and_then(|v| v.as_str())
            .map(String::from),
    }
}
