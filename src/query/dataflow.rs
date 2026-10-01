use crate::graph_store::GraphReadStore;
use crate::output::schema::format_next_query;
use crate::query::find_candidates;
use anyhow::Result;
use regex::Regex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

// ============================================================
// DataFlow 字段级来源追溯
// ============================================================

/// DataFlow 节点字段记录（支持 serde 反序列化）
#[derive(Debug, Clone, Deserialize)]
pub struct FieldRecord {
    name: String,
    dbfield: String,
    #[serde(rename = "originalField")]
    original_field: Option<String>,
    #[serde(rename = "originalNode")]
    original_node: Option<String>,
    exp: Option<String>,
    #[serde(rename = "inputField")]
    input_field: Option<String>,
    #[serde(rename = "inputNode")]
    input_node: Option<String>,
}

/// DataFlow 过滤器子句（支持 serde 反序列化）
#[derive(Debug, Clone, Deserialize)]
pub struct DataflowFilterClause {
    #[serde(rename = "leftExp")]
    pub left_exp: Option<String>,
    pub operator: Option<String>,
    #[serde(rename = "rightValue")]
    pub right_value: Option<serde_json::Value>,
    #[serde(rename = "rightExp")]
    pub right_exp: Option<String>,
    pub exp: Option<String>,
}

/// DataFlow Join 条件（支持 serde 反序列化）
#[derive(Debug, Clone, Deserialize)]
pub struct DataflowJoinCondition {
    #[serde(rename = "joinType")]
    pub join_type: String,
    #[serde(rename = "leftTable")]
    pub left_table: String,
    #[serde(rename = "rightTable")]
    pub right_table: String,
    pub clauses: Vec<DataflowFilterClause>,
}

/// DataFlow Union 字段映射条目（支持 serde 反序列化）
#[derive(Debug, Clone, Deserialize)]
pub struct DataflowUnionMapEntry {
    pub visible: bool,
    pub values: Vec<Option<String>>,
}

/// DataFlow 预解析元数据（一次性反序列化 + 预建索引）
#[derive(Debug, Clone, Default)]
pub struct DataFlowMeta {
    /// alias -> node_id
    alias_map: HashMap<String, String>,
    /// node_id -> alias
    id_to_alias: HashMap<String, String>,
    /// node_id -> node_type
    node_types: HashMap<String, String>,
    /// node_id -> [dep_node_id]
    internal_deps: HashMap<String, Vec<String>>,
    /// 预建索引：node_id -> field_name -> FieldRecord
    field_index: HashMap<String, HashMap<String, FieldRecord>>,
    /// node_id -> 过滤器子句
    node_filters: HashMap<String, Vec<DataflowFilterClause>>,
    /// node_id -> moduleTablePath
    module_table_paths: HashMap<String, String>,
    /// node_id -> Join 条件
    pub node_join_conditions: HashMap<String, Vec<DataflowJoinCondition>>,
    /// node_id -> Union 字段映射
    pub node_union_maps: HashMap<String, Vec<DataflowUnionMapEntry>>,
}

impl DataFlowMeta {
    /// 从 node.meta 的 serde_json::Value 一次性构建
    pub fn from_meta(meta: &serde_json::Value) -> Self {
        let alias_map: HashMap<String, String> = meta
            .get("aliasMap")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let node_fields_raw: HashMap<String, Vec<serde_json::Value>> = meta
            .get("nodeFields")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let internal_deps: HashMap<String, Vec<String>> = meta
            .get("internalDeps")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let node_types: HashMap<String, String> = meta
            .get("nodeTypes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let node_filters: HashMap<String, Vec<DataflowFilterClause>> = meta
            .get("nodeFilters")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let module_table_paths: HashMap<String, String> = meta
            .get("nodeTablePaths")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let node_join_conditions: HashMap<String, Vec<DataflowJoinCondition>> = meta
            .get("nodeJoinConditions")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let node_union_maps: HashMap<String, Vec<DataflowUnionMapEntry>> = meta
            .get("nodeUnionMaps")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let add_field_record = |idx: &mut HashMap<String, FieldRecord>, rec: FieldRecord| {
            idx.insert(rec.name.clone(), rec.clone());
            if rec.dbfield != rec.name {
                idx.insert(rec.dbfield.clone(), rec);
            }
        };

        // 备用解析：当 nodeFields 不足时，使用 dimensions 做兼容性字段来源
        let dimensions: Vec<FieldRecord> = meta
            .get("dimensions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|dim| {
                        let name = dim.get("name")?.as_str()?.to_string();
                        let dbfield = dim.get("dbfield")?.as_str()?.to_string();
                        let exp = dim
                            .get("exp")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let input_field = dim
                            .get("inputField")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        Some(FieldRecord {
                            name,
                            dbfield,
                            original_field: None,
                            original_node: None,
                            exp,
                            input_field,
                            input_node: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        // 预解析 node_fields：serde_json::Value -> Vec<FieldRecord>，并建立 field_name 索引
        let mut field_index: HashMap<String, HashMap<String, FieldRecord>> = HashMap::new();

        for (node_id, raw_fields) in &node_fields_raw {
            let records: Vec<FieldRecord> = raw_fields
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect();
            let mut idx = HashMap::new();
            for rec in &records {
                add_field_record(&mut idx, rec.clone());
            }
            field_index.insert(node_id.clone(), idx);
        }

        // 兼容回退：若 nodeFields 为空但 dimensions 存在，则将 dimensions 作为默认输出索引
        if field_index.is_empty() && !dimensions.is_empty() {
            let mut idx = HashMap::new();
            for dim in &dimensions {
                add_field_record(&mut idx, dim.clone());
            }
            field_index.insert("default".to_string(), idx);
        }

        // 兼容回退：Output 节点若缺少 nodeFields，使用 dimensions 补齐
        // 同时把 dimensions 的 inputField/exp 合并回已存在的 Output 字段
        let output_nodes: Vec<String> = node_types
            .iter()
            .filter(|(_, t)| *t == "Output")
            .map(|(id, _)| id.clone())
            .collect();
        for out_id in output_nodes {
            if !field_index.contains_key(&out_id) && !dimensions.is_empty() {
                let mut idx = HashMap::new();
                for dim in &dimensions {
                    idx.insert(dim.name.clone(), dim.clone());
                }
                field_index.insert(out_id.clone(), idx);
            } else if let Some(existing_idx) = field_index.get(&out_id) {
                // 将 dimensions 的 inputField/exp 合并到已有 Output 字段定义
                let dim_map: HashMap<String, &FieldRecord> =
                    dimensions.iter().map(|d| (d.name.clone(), d)).collect();
                let mut merged_idx = existing_idx.clone();
                for (field_name, rec) in existing_idx {
                    if let Some(dim) = dim_map.get(field_name) {
                        let mut merged = rec.clone();
                        if merged.exp.is_none() && dim.exp.is_some() {
                            merged.exp = dim.exp.clone();
                        }
                        if merged.input_field.is_none() && dim.input_field.is_some() {
                            merged.input_field = dim.input_field.clone();
                        }
                        merged_idx.insert(field_name.clone(), merged);
                    }
                }
                field_index.insert(out_id.clone(), merged_idx);
            }
        }

        let id_to_alias: HashMap<String, String> = alias_map
            .iter()
            .map(|(a, id)| (id.clone(), a.clone()))
            .collect();

        DataFlowMeta {
            alias_map,
            id_to_alias,
            node_types,
            internal_deps,
            field_index,
            node_filters,
            module_table_paths,
            node_join_conditions,
            node_union_maps,
        }
    }

    /// 根据 node_id 获取字段索引（预建，O(1)）
    pub fn get_fields(&self, node_id: &str) -> Option<&HashMap<String, FieldRecord>> {
        self.field_index.get(node_id)
    }

    /// 根据 node_id 获取过滤器子句
    pub fn get_node_filters(&self, node_id: &str) -> Option<&Vec<DataflowFilterClause>> {
        self.node_filters.get(node_id)
    }

    /// 根据 node_id 获取节点类型
    pub fn get_node_type(&self, node_id: &str) -> &str {
        self.node_types
            .get(node_id)
            .map(|s| s.as_str())
            .unwrap_or("Unknown")
    }

    /// 根据 alias 获取 node_id
    pub fn get_node_id(&self, alias: &str) -> Option<&String> {
        self.alias_map.get(alias)
    }

    /// 根据 node_id 获取 alias
    pub fn get_alias(&self, node_id: &str) -> Option<&String> {
        self.id_to_alias.get(node_id)
    }

    /// 根据 node_id 获取 moduleTablePath
    pub fn get_node_module_table_path(&self, node_id: &str) -> Option<&str> {
        self.module_table_paths
            .get(node_id)
            .map(std::string::String::as_str)
    }

    /// 获取 DataFlow 中所有 ModelTable 节点的 moduleTablePath
    pub fn get_model_table_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self
            .node_types
            .iter()
            .filter_map(|(node_id, node_type)| {
                if node_type == "ModelTable" {
                    self.module_table_paths
                        .get(node_id)
                        .filter(|p| !p.is_empty())
                        .cloned()
                } else {
                    None
                }
            })
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }

    /// 获取 Output 类型节点的字段索引；如果没有 Output 节点，fallback 到 "default"
    pub fn get_output_fields(&self) -> Vec<(&str, &HashMap<String, FieldRecord>)> {
        // node_types 是 HashMap：输出节点枚举序随随机种子逐次不同，
        // field_traces 的输出顺序必须确定，先排序再投影。
        let mut output_nodes: Vec<&str> = self
            .node_types
            .iter()
            .filter(|(_, t)| *t == "Output")
            .map(|(id, _)| id.as_str())
            .collect();
        output_nodes.sort_unstable();

        if !output_nodes.is_empty() {
            output_nodes
                .into_iter()
                .filter_map(|id| self.field_index.get(id).map(|fields| (id, fields)))
                .collect()
        } else {
            self.field_index
                .get("default")
                .map(|fields| vec![("default", fields)])
                .unwrap_or_default()
        }
    }

    /// 输出全部可解析 filter 投影
    pub fn project_filters(&self) -> Vec<DataflowFilterProjection> {
        // M34: 识别输出路径上的最终过滤节点（Output 节点的直接上游）
        let output_node_ids: std::collections::HashSet<&str> = self
            .node_types
            .iter()
            .filter(|(_, t)| *t == "Output")
            .map(|(id, _)| id.as_str())
            .collect();
        let mut output_upstream: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (node_id, deps) in &self.internal_deps {
            if output_node_ids.contains(node_id.as_str()) {
                for dep in deps {
                    output_upstream.insert(dep.as_str());
                }
            }
        }

        let mut entries: Vec<(String, String)> = self
            .node_filters
            .keys()
            .map(|node_id| {
                let alias = self
                    .get_alias(node_id)
                    .cloned()
                    .unwrap_or_else(|| node_id.clone());
                (alias, node_id.clone())
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let mut projections = Vec::new();
        for (node_alias, node_id) in entries {
            let node_type = self.get_node_type(&node_id).to_string();
            let is_output = output_node_ids.contains(node_id.as_str());
            let is_output_upstream = output_upstream.contains(node_id.as_str());
            let role = match node_type.as_str() {
                "ModelTable" => "source_filter",
                "Output" => "output_filter",
                "Union" => "branch_filter",
                _ if is_output || is_output_upstream => "output_filter",
                _ => "node_filter",
            }
            .to_string();

            let Some(clauses) = self.get_node_filters(&node_id) else {
                continue;
            };
            for clause in clauses {
                projections.push(project_filter_clause(
                    &node_alias,
                    &node_type,
                    &role,
                    clause,
                ));
            }
        }
        projections
    }
}

#[derive(Debug, Clone)]
/// 字段追溯链中的单步信息
pub struct TraceStep {
    node_alias: String,
    node_type: String,
    module_table_path: Option<String>,
    field_name: String,
    dbfield: String,
    input_node: Option<String>,
    exp: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DataflowFieldOriginProjection {
    pub dataflow_table: String,
    pub dataflow_output_field: String,
    pub physical_source_fields: Vec<String>,
    pub original_node: Option<String>,
    pub original_field: Option<String>,
    pub via: String,
    pub confidence: String,
    pub missing_evidence: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DataflowFilterProjection {
    pub node_alias: String,
    pub node_type: String,
    pub role: String,
    pub expr: Option<String>,
    pub left: Option<String>,
    pub operator: Option<String>,
    pub right: Option<String>,
    pub referenced_fields: Vec<String>,
    pub referenced_vars: Vec<String>,
}

fn format_filter_right_value(right_value: &serde_json::Value) -> Option<String> {
    if right_value.is_null() {
        return None;
    }
    right_value.as_str().map(str::to_string).or_else(|| {
        right_value
            .as_f64()
            .map(|value| value.to_string())
            .or_else(|| {
                if right_value.is_boolean() {
                    right_value.as_bool().map(|value| value.to_string())
                } else {
                    Some(right_value.to_string())
                }
            })
    })
}

fn collect_ordered_unique(target: &mut Vec<String>, seen: &mut HashSet<String>, item: String) {
    if seen.insert(item.clone()) {
        target.push(item);
    }
}

fn extract_filter_refs(expr: &str, fields: &mut Vec<String>, vars: &mut Vec<String>) {
    let mut seen_fields: HashSet<String> = HashSet::new();
    let mut seen_vars: HashSet<String> = HashSet::new();

    let node_field_regex =
        Regex::new(r"\[([^\[\].\n]+)\]\.\[([^\[\].\n]+)\]").expect("valid regex");
    let field_regex = Regex::new(r"\[([^\[\]\n]+)\]").expect("valid regex");
    let var_regex = Regex::new(r"\$(?:user|param)\.[A-Za-z0-9_]+").expect("valid regex");
    let param_regex = Regex::new(r"\bparam\d+\b").expect("valid regex");

    let node_field_ranges: Vec<(usize, usize)> = node_field_regex
        .captures_iter(expr)
        .map(|cap| {
            let field_name = format!("{}.{}", &cap[1], &cap[2]);
            collect_ordered_unique(fields, &mut seen_fields, field_name);
            let range = cap
                .get(0)
                .expect("regex capture should have full match")
                .range();
            (range.start, range.end)
        })
        .collect();

    for cap in field_regex.captures_iter(expr) {
        let Some(full_match) = cap.get(0) else {
            continue;
        };
        let in_node_field = node_field_ranges
            .iter()
            .any(|(start, end)| full_match.start() >= *start && full_match.end() <= *end);
        if in_node_field {
            continue;
        }
        collect_ordered_unique(fields, &mut seen_fields, cap[1].to_string());
    }

    for cap in var_regex.captures_iter(expr) {
        collect_ordered_unique(vars, &mut seen_vars, cap[0].to_string());
    }
    for cap in param_regex.captures_iter(expr) {
        collect_ordered_unique(vars, &mut seen_vars, cap[0].to_string());
    }
}

fn project_filter_clause(
    node_alias: &str,
    node_type: &str,
    role: &str,
    clause: &DataflowFilterClause,
) -> DataflowFilterProjection {
    let mut referenced_fields = Vec::new();
    let mut referenced_vars = Vec::new();

    if let Some(left_exp) = clause.left_exp.as_deref() {
        extract_filter_refs(left_exp, &mut referenced_fields, &mut referenced_vars);
    }
    if let Some(right_exp) = clause.right_exp.as_deref() {
        extract_filter_refs(right_exp, &mut referenced_fields, &mut referenced_vars);
    }
    if let Some(exp) = clause.exp.as_deref() {
        extract_filter_refs(exp, &mut referenced_fields, &mut referenced_vars);
    }

    let right = clause.right_exp.clone().or_else(|| {
        clause
            .right_value
            .as_ref()
            .and_then(format_filter_right_value)
    });

    DataflowFilterProjection {
        node_alias: node_alias.to_string(),
        node_type: node_type.to_string(),
        role: role.to_string(),
        expr: clause.exp.clone(),
        left: clause.left_exp.clone(),
        operator: clause.operator.clone(),
        right,
        referenced_fields,
        referenced_vars,
    }
}

impl DataflowFilterProjection {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "node_alias": self.node_alias,
            "node_type": self.node_type,
            "role": self.role,
            "expr": self.expr,
            "left": self.left,
            "operator": self.operator,
            "right": self.right,
            "referenced_fields": self.referenced_fields,
            "referenced_vars": self.referenced_vars,
        })
    }
}

pub fn build_via_value(original_node: Option<&str>, original_field: Option<&str>) -> String {
    match (original_node, original_field) {
        (Some(_), Some(_)) => "originalNode/originalField".to_string(),
        (Some(_), None) => "originalNode".to_string(),
        (None, Some(_)) => "originalField".to_string(),
        (None, None) => "unknown".to_string(),
    }
}

fn lookup_output_field<'a>(
    fields: &'a HashMap<String, FieldRecord>,
    target_field: &str,
) -> Option<&'a FieldRecord> {
    fields.get(target_field).or_else(|| {
        fields.values().find(|f| {
            f.dbfield == target_field
                || f.original_field
                    .as_ref()
                    .map_or(false, |value| value == target_field)
                || f.input_field
                    .as_ref()
                    .map_or(false, |value| value == target_field)
                || f.name == target_field
        })
    })
}

pub fn project_output_field_origin(
    meta: &DataFlowMeta,
    target_field: &str,
) -> DataflowFieldOriginProjection {
    let mut missing_evidence = Vec::new();
    let mut dataflow_output_field = target_field.to_string();

    let output_entry = meta
        .get_output_fields()
        .into_iter()
        .find_map(|(node_id, fields)| {
            lookup_output_field(fields, target_field).map(|field_record| (node_id, field_record))
        });

    let (output_node_id, output_field) = match output_entry {
        Some(entry) => entry,
        None => {
            return DataflowFieldOriginProjection {
                dataflow_table: String::new(),
                dataflow_output_field,
                physical_source_fields: vec![],
                original_node: None,
                original_field: None,
                via: String::new(),
                confidence: "candidate".to_string(),
                missing_evidence: vec!["output field not found in DataFlow outputs".to_string()],
            };
        }
    };

    dataflow_output_field = output_field.dbfield.clone();

    let original_node = output_field.original_node.clone();
    let original_field = output_field.original_field.clone();
    let dataflow_table = meta
        .get_node_module_table_path(output_node_id)
        .map(str::to_string)
        .or_else(|| meta.get_alias(output_node_id).cloned())
        .unwrap_or_else(|| output_node_id.to_string());

    let mut projection = DataflowFieldOriginProjection {
        dataflow_table,
        dataflow_output_field,
        physical_source_fields: Vec::new(),
        original_node: original_node.clone(),
        original_field: original_field.clone(),
        via: build_via_value(original_node.as_deref(), original_field.as_deref()),
        confidence: "candidate".to_string(),
        missing_evidence: Vec::new(),
    };

    let Some(output_node) = original_node
        .as_deref()
        .and_then(|alias| meta.get_node_id(alias))
    else {
        missing_evidence.push("missing originalNode alias in aliasMap".to_string());
        projection.missing_evidence = missing_evidence;
        return projection;
    };

    let Some(module_table_path) = meta.get_node_module_table_path(output_node) else {
        missing_evidence.push(format!(
            "original node {} has no moduleTablePath",
            original_node.as_deref().unwrap_or_default()
        ));
        projection.missing_evidence = missing_evidence;
        return projection;
    };

    if module_table_path.is_empty() {
        missing_evidence.push("moduleTablePath is empty".to_string());
        projection.missing_evidence = missing_evidence;
        return projection;
    }

    projection.confidence = "proven".to_string();
    if let Some(ref original_field_name) = original_field {
        projection
            .physical_source_fields
            .push(format!("{}.{}", module_table_path, original_field_name));
    }
    projection.missing_evidence = Vec::new();
    projection
}

impl DataflowFieldOriginProjection {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "dataflow_table": self.dataflow_table,
            "dataflow_output_field": self.dataflow_output_field,
            "physical_source_fields": self.physical_source_fields,
            "original_node": self.original_node,
            "original_field": self.original_field,
            "via": self.via,
            "confidence": self.confidence,
            "missing_evidence": self.missing_evidence,
        })
    }
}

/// 递归追溯字段的数据来源链
fn trace_field_source(
    output_field_name: &str,
    output_field: &FieldRecord,
    meta: &DataFlowMeta,
    visited: &mut Vec<(String, String)>,
) -> Vec<TraceStep> {
    let mut steps = Vec::new();

    let mut current_field_name = output_field_name.to_string();
    let mut current_node_alias: Option<String> = output_field.original_node.clone();
    let mut current_field_dbfield = output_field.dbfield.clone();
    let mut current_exp = output_field.exp.clone();

    steps.push(TraceStep {
        node_alias: "模型输出".to_string(),
        node_type: "Output".to_string(),
        module_table_path: None,
        field_name: current_field_name.clone(),
        dbfield: current_field_dbfield.clone(),
        input_node: output_field.input_node.clone(),
        exp: current_exp.clone(),
    });

    while let Some(node_alias) = &current_node_alias {
        let node_alias = node_alias.clone();

        let node_id = match meta.get_node_id(&node_alias) {
            Some(id) => id,
            None => break,
        };

        let visit_key = (node_id.to_string(), current_field_name.clone());
        if visited.contains(&visit_key) {
            break;
        }
        visited.push(visit_key);

        let node_type = meta.get_node_type(node_id).to_string();
        let field_records = match meta.get_fields(node_id) {
            Some(recs) => recs,
            None => {
                steps.push(TraceStep {
                    node_alias: node_alias.clone(),
                    node_type: node_type.clone(),
                    module_table_path: meta.get_node_module_table_path(node_id).map(str::to_string),
                    field_name: format!("{} (字段未在当前节点声明)", current_field_name),
                    dbfield: "-".to_string(),
                    input_node: None,
                    exp: None,
                });
                break;
            }
        };

        let field_rec = field_records.get(&current_field_name).or_else(|| {
            field_records.values().find(|f| {
                f.dbfield == current_field_name
                    || f.original_field
                        .as_ref()
                        .map_or(false, |value| value == &current_field_name)
                    || f.input_field
                        .as_ref()
                        .map_or(false, |value| value == &current_field_name)
                    || f.name == current_field_name
            })
        });

        if let Some(rec) = field_rec {
            current_field_name = rec.name.clone();
            current_field_dbfield = rec.dbfield.clone();
            current_exp = rec.exp.clone();
            current_node_alias = rec.original_node.clone();

            steps.push(TraceStep {
                node_alias: node_alias.clone(),
                node_type: node_type.clone(),
                module_table_path: meta.get_node_module_table_path(node_id).map(str::to_string),
                field_name: current_field_name.clone(),
                dbfield: current_field_dbfield.clone(),
                input_node: rec.input_node.clone(),
                exp: current_exp.clone(),
            });
        } else {
            steps.push(TraceStep {
                node_alias: node_alias.clone(),
                node_type: node_type.clone(),
                module_table_path: meta.get_node_module_table_path(node_id).map(str::to_string),
                field_name: format!("{} (字段未在当前节点声明)", current_field_name),
                dbfield: "-".to_string(),
                input_node: None,
                exp: None,
            });
            break;
        }
    }

    steps
}
/// 展开 DataFlow 子图，追溯字段来源
/// 构建 query_dataflow 输出（返回 Value，不打印）
pub fn build_query_dataflow_output(
    graph: &dyn GraphReadStore,
    dataflow_id: &str,
) -> Result<serde_json::Value> {
    // M59-2 A1 接线：`QueryDataflow` 是 `--relations model:X` 展开出的补充调用
    // （route.rs:165，merge_key=dataflow_subgraph），与主调用 `QueryModel` 拿**同一个**
    // target。此前这里直接精确 `get_node`，不做旧 target 解析——于是裸 `model:orders`
    // 同时匹配页面局部与物理模型时，主调用如实报 `AMBIGUOUS_TARGET`，补充调用却
    // **静默命中物理模型**并返回它的子图。同一个 target、同一条命令，两个子调用给出
    // 互相矛盾的定位结论，而矛盾的那一半正是「静默挑一个」。
    let resolved_id = match crate::query::resolve_legacy_model_target(graph, dataflow_id)? {
        crate::query::LegacyModelTarget::Exact(id)
        | crate::query::LegacyModelTarget::Resolved(id) => id,
        crate::query::LegacyModelTarget::Ambiguous(nodes) => {
            return crate::query::build_ambiguous_target_output(
                dataflow_id,
                &nodes,
                crate::output::OutputKind::ModelQuery,
                "--query-dataflow",
            );
        }
        crate::query::LegacyModelTarget::Missing => {
            let candidates = find_candidates(graph, dataflow_id, 5)?;
            let out = crate::output::schema::build_target_not_found_output(
                crate::output::schema::OutputKind::ModelQuery,
                dataflow_id,
                &candidates,
            );
            return Ok(serde_json::to_value(out)?);
        }
    };
    let dataflow_id = resolved_id.as_str();
    let node = match graph.get_node(dataflow_id)? {
        Some(n) => n,
        None => {
            let candidates = find_candidates(graph, dataflow_id, 5)?;
            let out = crate::output::schema::build_target_not_found_output(
                crate::output::schema::OutputKind::ModelQuery,
                dataflow_id,
                &candidates,
            );
            return Ok(serde_json::to_value(out)?);
        }
    };

    let raw_meta = node.meta.as_ref();
    let dfm = raw_meta.map(DataFlowMeta::from_meta).unwrap_or_default();
    let dataflow_filters = dfm.project_filters();

    let outgoing = graph
        .get_node_edges(dataflow_id)?
        .map(|n| n.outgoing)
        .unwrap_or_default();

    let inputs: Vec<_> = outgoing
        .iter()
        .filter(|ev| matches!(ev.edge.edge_type, crate::graph::EdgeType::DataflowInput))
        .map(|ev| (ev.node.clone(), ev.edge.clone()))
        .collect();

    let outputs: Vec<_> = outgoing
        .iter()
        .filter(|ev| matches!(ev.edge.edge_type, crate::graph::EdgeType::OutputsTo))
        .map(|ev| (ev.node.clone(), ev.edge.clone()))
        .collect();

    let mut field_traces: Vec<serde_json::Value> = Vec::new();

    let output_nodes = dfm.get_output_fields();
    let is_dimensions_fallback = output_nodes.iter().any(|(id, _)| *id == "default");

    if output_nodes.is_empty() {
        field_traces.push(serde_json::json!({
            "trace_source": "missing",
            "diagnostics": "No output node fields or dimensions found",
        }));
    } else {
        for (node_id, fields) in output_nodes {
            let trace_source = if is_dimensions_fallback {
                "dimensions_fallback"
            } else {
                "output_node"
            };
            let alias = dfm
                .get_alias(node_id)
                .map_or(node_id.to_string(), |v| v.clone());

            // fields 是 HashMap：枚举序随机，按字段名排序保证输出确定。
            let mut field_names: Vec<&String> = fields.keys().collect();
            field_names.sort_unstable();
            for field_name in field_names {
                let field_rec = &fields[field_name];
                let field_name: &String = field_name;
                let mut visited: Vec<(String, String)> = Vec::new();
                let trace = trace_field_source(field_name, field_rec, &dfm, &mut visited);
                let origin_projection = project_output_field_origin(&dfm, field_name);

                field_traces.push(serde_json::json!({
                    "field": field_name,
                    "dbfield": field_rec.dbfield,
                    "output_node_id": node_id,
                    "output_node_alias": alias,
                    "trace_source": trace_source,
                    "origin_projection": origin_projection.to_json(),
                    "trace": trace.iter().map(|s| serde_json::json!({
                        "node_alias": s.node_alias,
                        "node_type": s.node_type,
                        "module_table_path": s.module_table_path,
                        "field_name": s.field_name,
                        "dbfield": s.dbfield,
                        "input_node": s.input_node,
                        "exp": s.exp,
                    })).collect::<Vec<_>>(),
                }));
            }
        }
    }

    let summary = serde_json::json!({
        "dataflow_id": dataflow_id,
        "name": node.name,
        "model_type": raw_meta.and_then(|m| m.get("modelType")).and_then(|v| v.as_str()).unwrap_or("DataFlow"),
        "input_count": inputs.len(),
        "output_count": outputs.len(),
    });

    let details = serde_json::json!({
        "inputs": inputs.iter().map(|(n, e)| serde_json::json!({
            "id": n.id,
            "name": n.name,
            "path": n.path,
            "field_path": e.field_path,
        })).collect::<Vec<_>>(),
        "outputs": outputs.iter().map(|(n, e)| serde_json::json!({
            "id": n.id,
            "name": n.name,
            "path": n.path,
            "field_path": e.field_path,
        })).collect::<Vec<_>>(),
        "field_traces": field_traces,
        "internal_deps": dfm.internal_deps.iter().map(|(id, deps)| serde_json::json!({
            "node_id": id,
            "depends_on": deps,
        })).collect::<Vec<_>>(),
        "dataflow_filters": dataflow_filters.iter().map(|f| f.to_json()).collect::<Vec<_>>(),
    });

    let mut output =
        crate::output::AiOutput::new(crate::output::OutputKind::DataFlowQuery, summary);
    output.query_target = Some(dataflow_id.to_string());
    output.details = Some(details);
    output.evidence.push(
        crate::output::Evidence::new(
            format!("DataFlow {} has {} inputs", dataflow_id, inputs.len()),
            "Parsed from DataFlow metadata: input nodes",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(dataflow_id),
    );
    output.evidence.push(
        crate::output::Evidence::new(
            format!("DataFlow {} has {} outputs", dataflow_id, outputs.len()),
            "Parsed from DataFlow metadata: output nodes",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(dataflow_id),
    );
    output.next_queries = vec![
        format_next_query("--explain {} for semantic summary", dataflow_id),
        format_next_query("--context {} --depth 2", dataflow_id),
    ];

    let output = output.validate();
    Ok(serde_json::to_value(output)?)
}

/// 查询 DataFlow 模型（保留旧入口，直接打印 stdout）
pub fn query_dataflow(graph: &dyn GraphReadStore, dataflow_id: &str, human: bool) -> Result<()> {
    let node = match graph.get_node(dataflow_id)? {
        Some(n) => n,
        None => {
            let candidates = find_candidates(graph, dataflow_id, 5)?;
            let out = crate::output::schema::build_target_not_found_output(
                crate::output::schema::OutputKind::ModelQuery,
                dataflow_id,
                &candidates,
            );
            println!("{}", serde_json::to_string_pretty(&out)?);
            return Ok(());
        }
    };

    let raw_meta = node.meta.as_ref();
    let dfm = raw_meta.map(DataFlowMeta::from_meta).unwrap_or_default();
    let _dataflow_filters = dfm.project_filters();

    let outgoing = graph
        .get_node_edges(dataflow_id)?
        .map(|n| n.outgoing)
        .unwrap_or_default();

    let inputs: Vec<_> = outgoing
        .iter()
        .filter(|ev| matches!(ev.edge.edge_type, crate::graph::EdgeType::DataflowInput))
        .map(|ev| (ev.node.clone(), ev.edge.clone()))
        .collect();

    let outputs: Vec<_> = outgoing
        .iter()
        .filter(|ev| matches!(ev.edge.edge_type, crate::graph::EdgeType::OutputsTo))
        .map(|ev| (ev.node.clone(), ev.edge.clone()))
        .collect();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== DataFlow: {} ===", dataflow_id)?;
        writeln!(out, "Name: {} | Path: {}", node.name, node.path)?;
        writeln!(
            out,
            "Model Type: {}",
            raw_meta
                .and_then(|m| m.get("modelType"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
        )?;

        writeln!(out, "\n--- 输入数据源 ({}个): ---", inputs.len())?;
        for (n, e) in &inputs {
            writeln!(
                out,
                "  [ModelTable] {} -> {}",
                n.name,
                e.field_path.as_deref().unwrap_or("-")
            )?;
        }

        writeln!(out, "\n--- 输出物理表 ({}个): ---", outputs.len())?;
        for (n, e) in &outputs {
            writeln!(
                out,
                "  [物理表] {}.tbl (dbTableName: {})",
                n.name,
                e.field_path.as_deref().unwrap_or("-")
            )?;
        }

        writeln!(out, "\n=== 字段级来源追溯 ===")?;

        let output_nodes = dfm.get_output_fields();
        let is_dimensions_fallback = output_nodes.iter().any(|(id, _)| *id == "default");

        if output_nodes.is_empty() {
            writeln!(out, "\n(未找到输出节点字段映射信息)")?;
        } else {
            for (node_id, fields) in output_nodes {
                let alias = dfm
                    .get_alias(node_id)
                    .map_or(node_id.to_string(), |v| v.clone());
                if is_dimensions_fallback {
                    writeln!(out, "\n--- 输出字段 (来自 dimensions) ---")?;
                } else {
                    writeln!(out, "\n--- 输出节点: {} ({}) ---", alias, node_id)?;
                }
                // fields 是 HashMap：枚举序随机，按字段名排序保证输出确定。
                let mut field_names: Vec<&String> = fields.keys().collect();
                field_names.sort_unstable();
                for field_name in field_names {
                    let field_rec = &fields[field_name];
                    let field_name: &String = field_name;
                    let mut visited: Vec<(String, String)> = Vec::new();
                    let trace = trace_field_source(field_name, field_rec, &dfm, &mut visited);
                    writeln!(out, "\n[字段] {}", field_name)?;

                    if trace.len() <= 1 {
                        if let Some(ref input) = field_rec.input_field {
                            writeln!(out, "  来源: inputField 映射 -> {}", input)?;
                        } else if let Some(ref exp) = field_rec.exp {
                            writeln!(out, "  来源: 表达式 -> {}", exp)?;
                        } else {
                            writeln!(out, "  来源: (未记录来源关系)")?;
                        }
                        continue;
                    }

                    for (i, step) in trace.iter().enumerate() {
                        let is_output = i == 0;
                        let is_root = i == trace.len() - 1;
                        let prefix = if is_output {
                            "  输出"
                        } else if is_root {
                            "  根来源"
                        } else {
                            "  中间"
                        };
                        let exp_info = step
                            .exp
                            .as_ref()
                            .map(|e| format!(" [exp: {}]", e))
                            .unwrap_or_default();
                        let module_info = step
                            .module_table_path
                            .as_ref()
                            .map(|path| format!(" [table: {}]", path))
                            .unwrap_or_default();
                        let input_info = step
                            .input_node
                            .as_ref()
                            .map(|input| format!(" [inputNode: {}]", input))
                            .unwrap_or_default();
                        let type_label = match step.node_type.as_str() {
                            "ModelTable" => "[数据表]",
                            "Select" => "[列加工]",
                            "Join" => "[关联]",
                            "Union" => "[联合]",
                            "Distinct" => "[去重]",
                            "Output" => "[输出]",
                            _ => "[节点]",
                        };
                        writeln!(
                            out,
                            "{} {} {}.{} (db={}){}{}{}",
                            prefix,
                            type_label,
                            step.node_alias,
                            step.field_name,
                            step.dbfield,
                            module_info,
                            input_info,
                            exp_info
                        )?;
                    }
                }
            }
        }

        writeln!(out, "\n=== 内部节点拓扑 ===")?;
        if dfm.internal_deps.is_empty() {
            writeln!(out, "(未记录内部节点依赖)")?;
        } else {
            for (node_id, deps) in &dfm.internal_deps {
                let alias = dfm
                    .get_alias(node_id)
                    .map(|a| a.as_str())
                    .unwrap_or(node_id);
                let dep_names: Vec<String> = deps
                    .iter()
                    .map(|d| {
                        dfm.get_alias(d)
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| d.clone())
                    })
                    .collect();
                writeln!(out, "  [{}] -> {}", alias, dep_names.join(", "))?;
            }
        }
    } else {
        let val = build_query_dataflow_output(graph, dataflow_id)?;
        println!("{}", serde_json::to_string_pretty(&val)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dataflow_meta_indexes_output_field_by_name_and_dbfield() {
        let raw_meta = serde_json::json!({
            "aliasMap": {
                "FACT_AUTOCUSTOMERAUTOREL": "node_source",
                "模型输出": "output1"
            },
            "nodeTablePaths": {
                "node_source": "$DATA:/主数据/fact_autoCustomerAutoRel.tbl",
                "output1": "$DATA:/加工表/小程序/绑车.tbl"
            },
            "nodeTypes": {
                "node_source": "ModelTable",
                "output1": "Output"
            },
            "nodeFields": {
                "output1": [
                    {
                        "name": "车辆VIN",
                        "dbfield": "CUSTOMAUTOMYAUTOLIST",
                        "originalNode": "FACT_AUTOCUSTOMERAUTOREL",
                        "originalField": "车辆VIN",
                        "inputNode": "node_source"
                    }
                ]
            }
        });

        let meta = DataFlowMeta::from_meta(&raw_meta);

        let module_path = meta
            .get_node_module_table_path("node_source")
            .expect("moduleTablePath 应该被索引");
        assert_eq!(module_path, "$DATA:/主数据/fact_autoCustomerAutoRel.tbl");

        let output_fields = meta.get_fields("output1").expect("output1 字段索引应存在");

        let by_dbfield = output_fields
            .get("CUSTOMAUTOMYAUTOLIST")
            .expect("按 dbfield 匹配 CUSTOMAUTOMYAUTOLIST 应该命中字段");
        assert_eq!(
            by_dbfield.original_node.as_deref(),
            Some("FACT_AUTOCUSTOMERAUTOREL")
        );
        assert_eq!(by_dbfield.original_field.as_deref(), Some("车辆VIN"));

        let by_name = output_fields
            .get("车辆VIN")
            .expect("按 name 匹配 车辆VIN 应该命中同一条字段");
        assert_eq!(by_name.input_node.as_deref(), Some("node_source"));
    }

    #[test]
    fn test_dataflow_trace_field_source_keeps_original_node_and_field() {
        let raw_meta = serde_json::json!({
            "aliasMap": {
                "FACT_AUTOCUSTOMERAUTOREL": "node_source",
                "模型输出": "output1"
            },
            "nodeTablePaths": {
                "node_source": "$DATA:/主数据/fact_autoCustomerAutoRel.tbl",
                "output1": "$DATA:/加工表/小程序/绑车.tbl"
            },
            "nodeTypes": {
                "node_source": "ModelTable",
                "output1": "Output"
            },
            "nodeFields": {
                "output1": [
                    {
                        "name": "CUSTOMAUTOMYAUTOLIST",
                        "dbfield": "CUSTOMAUTOMYAUTOLIST",
                        "originalNode": "FACT_AUTOCUSTOMERAUTOREL",
                        "originalField": "车辆VIN",
                        "inputNode": "node_source"
                    }
                ]
            }
        });
        let meta = DataFlowMeta::from_meta(&raw_meta);
        let output_field = meta
            .get_fields("output1")
            .and_then(|fields| fields.get("CUSTOMAUTOMYAUTOLIST"))
            .expect("字段应存在");
        let mut visited = Vec::new();
        let trace = trace_field_source("CUSTOMAUTOMYAUTOLIST", output_field, &meta, &mut visited);

        assert!(
            trace.iter().any(|step| step.node_alias == "模型输出"),
            "第一步应仍为输出节点别名"
        );
        assert!(
            trace
                .iter()
                .any(|step| step.node_alias == "FACT_AUTOCUSTOMERAUTOREL"),
            "字段追溯应沿 originalNode 继续展开"
        );
        assert!(
            trace
                .iter()
                .any(|step| step.field_name == "CUSTOMAUTOMYAUTOLIST")
        );
        assert!(
            trace
                .iter()
                .any(|step| step.dbfield == "CUSTOMAUTOMYAUTOLIST")
        );
    }

    #[test]
    fn test_project_output_field_origin_proven_model_table() {
        let raw_meta = serde_json::json!({
            "aliasMap": {
                "FACT_AUTOCUSTOMERAUTOREL": "node_source",
                "模型输出": "output1"
            },
            "nodeTablePaths": {
                "node_source": "$DATA:/主数据/fact_autoCustomerAutoRel.tbl",
                "output1": "$DATA:/加工表/小程序/绑车.tbl"
            },
            "nodeTypes": {
                "node_source": "ModelTable",
                "output1": "Output"
            },
            "nodeFields": {
                "output1": [
                    {
                        "name": "CUSTOMAUTOMYAUTOLIST",
                        "dbfield": "CUSTOMAUTOMYAUTOLIST",
                        "originalNode": "FACT_AUTOCUSTOMERAUTOREL",
                        "originalField": "车辆VIN",
                        "inputNode": "node_source"
                    }
                ]
            }
        });
        let meta = DataFlowMeta::from_meta(&raw_meta);
        let projection = project_output_field_origin(&meta, "CUSTOMAUTOMYAUTOLIST");

        assert_eq!(projection.dataflow_table, "$DATA:/加工表/小程序/绑车.tbl");
        assert_eq!(
            projection.dataflow_output_field,
            "CUSTOMAUTOMYAUTOLIST".to_string()
        );
        assert_eq!(
            projection.physical_source_fields,
            vec!["$DATA:/主数据/fact_autoCustomerAutoRel.tbl.车辆VIN".to_string()]
        );
        assert_eq!(
            projection.original_node.as_deref(),
            Some("FACT_AUTOCUSTOMERAUTOREL")
        );
        assert_eq!(projection.original_field.as_deref(), Some("车辆VIN"));
        assert_eq!(projection.via, "originalNode/originalField");
        assert_eq!(projection.confidence, "proven");
        assert!(projection.missing_evidence.is_empty());
    }

    #[test]
    fn test_project_output_field_origin_candidate_when_module_table_path_missing() {
        let raw_meta = serde_json::json!({
            "aliasMap": {
                "FACT_MISS_TABLE": "node_source",
                "模型输出": "output1"
            },
            "nodeTypes": {
                "node_source": "ModelTable",
                "output1": "Output"
            },
            "nodeFields": {
                "output1": [
                    {
                        "name": "CUSTOMAUTOMYAUTOLIST",
                        "dbfield": "CUSTOMAUTOMYAUTOLIST",
                        "originalNode": "FACT_MISS_TABLE",
                        "originalField": "车辆VIN"
                    }
                ]
            }
        });
        let meta = DataFlowMeta::from_meta(&raw_meta);
        let projection = project_output_field_origin(&meta, "CUSTOMAUTOMYAUTOLIST");

        assert_eq!(projection.dataflow_table, "模型输出");
        assert_eq!(projection.confidence, "candidate");
        assert_eq!(projection.original_node.as_deref(), Some("FACT_MISS_TABLE"));
        assert_eq!(projection.original_field.as_deref(), Some("车辆VIN"));
        assert_eq!(projection.via, "originalNode/originalField");
        assert!(
            projection
                .missing_evidence
                .iter()
                .any(|item| item.contains("moduleTablePath"))
        );
        assert!(projection.physical_source_fields.is_empty());
    }

    fn load_filter_fixture_meta(path: &str) -> DataFlowMeta {
        let raw_content = match path {
            "source" => {
                include_str!("../../tests/fixtures/test_project/app/dataflow_filter_source.tbl")
            }
            "output" => {
                include_str!("../../tests/fixtures/test_project/app/dataflow_filter_output.tbl")
            }
            _ => panic!("unknown fixture key: {path}"),
        };
        let raw_json: serde_json::Value =
            serde_json::from_str(raw_content).expect("fixture JSON should parse");

        let mut alias_map = HashMap::new();
        let mut node_fields = HashMap::new();
        let mut node_types = HashMap::new();
        let mut node_filters = HashMap::new();
        let mut node_table_paths = HashMap::new();

        let nodes = raw_json
            .get("dataFlow")
            .and_then(|v| v.get("nodes"))
            .and_then(|v| v.as_object())
            .expect("fixture should contain dataFlow.nodes");

        for (node_id, node) in nodes {
            if let Some(alias) = node.get("alias").and_then(|v| v.as_str()) {
                alias_map.insert(alias.to_string(), node_id.clone());
            }
            if let Some(node_type) = node.get("type").and_then(|v| v.as_str()) {
                node_types.insert(node_id.clone(), node_type.to_string());
            }
            if let Some(module_table_path) = node.get("moduleTablePath").and_then(|v| v.as_str()) {
                node_table_paths.insert(node_id.clone(), module_table_path.to_string());
            }
            if let Some(fields) = node.get("fields").and_then(|v| v.as_array()) {
                node_fields.insert(node_id.clone(), fields.clone());
            }
            if let Some(clauses) = node
                .get("filter")
                .and_then(|v| v.get("clauses"))
                .and_then(|v| v.as_array())
            {
                node_filters.insert(node_id.clone(), clauses.to_vec());
            }
        }

        let meta = serde_json::json!({
            "aliasMap": alias_map,
            "nodeFields": node_fields,
            "nodeTypes": node_types,
            "nodeFilters": node_filters,
            "nodeTablePaths": node_table_paths,
        });
        DataFlowMeta::from_meta(&meta)
    }

    #[test]
    fn test_dataflow_filter_projection_source_filter_extracts_node_fields() {
        let meta = load_filter_fixture_meta("source");
        let projections = meta.project_filters();
        let source_filter = projections
            .iter()
            .find(|item| item.role == "source_filter")
            .expect("should expose source filter projection");

        assert_eq!(source_filter.node_alias, "FACT_AUTOCUSTOMERAUTOREL");
        assert_eq!(source_filter.left.as_deref(), Some("[是否展示]"));
        assert_eq!(source_filter.right.as_deref(), Some("1"));
        assert_eq!(source_filter.operator.as_deref(), Some("=="));
        assert_eq!(
            source_filter.referenced_fields.len(),
            1,
            "source filter 应只抽到 [是否展示] 一个字段"
        );
        assert!(
            source_filter
                .referenced_fields
                .contains(&"是否展示".to_string())
        );
    }

    #[test]
    fn test_dataflow_filter_projection_output_filter_extracts_vars_and_node_fields() {
        let meta = load_filter_fixture_meta("output");
        let projections = meta.project_filters();
        let output_filter = projections
            .iter()
            .find(|item| item.role == "output_filter")
            .expect("should expose output filter projection");

        assert_eq!(output_filter.node_alias, "模型输出");
        assert_eq!(
            output_filter.expr.as_deref(),
            Some("[FACT_AUTOCUSTOMERAUTOREL].[粉丝ID]=$user.WECHAT_UNIONID and [粉丝ID]=param1")
        );

        assert!(
            output_filter
                .referenced_fields
                .contains(&"FACT_AUTOCUSTOMERAUTOREL.粉丝ID".to_string())
        );
        assert!(
            output_filter
                .referenced_fields
                .contains(&"粉丝ID".to_string())
        );
        assert!(
            output_filter
                .referenced_vars
                .contains(&"$user.WECHAT_UNIONID".to_string())
        );
        assert!(
            output_filter
                .referenced_vars
                .contains(&"param1".to_string())
        );
    }
}
