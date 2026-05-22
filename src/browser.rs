//! Browser WASM runtime API
//!
//! M40.2：提供 browser-only WASM API，不改变现有 CLI/stdio 输出 schema。
//! 所有函数返回统一的 BrowserAnalysisEnvelope。

use crate::dependency::DependencyGraph;
use crate::graph::{EdgeType, Node, NodeType};
use crate::graph_store::{GraphReadStore, GraphWriteStore};
use crate::memory_graph_store::MemoryGraphStore;
use crate::superpage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// WASM runtime 全局单例
static RUNTIME: OnceLock<Mutex<BrowserRuntime>> = OnceLock::new();

/// Browser analysis 统一输出信封
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserAnalysisEnvelope {
    pub status: AnalysisStatus,
    pub target: Option<String>,
    pub items: Vec<AnalysisItem>,
    pub diagnostics: Vec<AnalysisDiagnostic>,
}

/// 分析状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisStatus {
    Initializing,
    Partial,
    Ready,
    Error,
}

/// 分析结果项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisItem {
    pub kind: String,
    pub label: String,
    pub detail: serde_json::Value,
}

/// 诊断信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisDiagnostic {
    pub severity: String,
    pub code: String,
    pub message: String,
}

/// Runtime 初始化选项
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeOptions {
    pub project_ref: Option<String>,
}

/// SuperPage 选择态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuperPageSelection {
    pub source_path: String,
    pub file_id: String,
    pub selected_component_ids: Vec<String>,
    pub active_component_id: Option<String>,
}

/// 分析选项
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnalysisOptions {
    pub include_priority: bool,
    pub include_conditions: bool,
    pub include_dataflow: bool,
}

/// Browser runtime 内部状态
#[derive(Debug, Default)]
pub struct BrowserRuntime {
    pub initialized: bool,
    pub options: RuntimeOptions,
    pub documents: HashMap<String, superpage::SuperPageMetadata>,
    pub graphs: HashMap<String, MemoryGraphStore>,
}

impl BrowserRuntime {
    fn new(options: RuntimeOptions) -> Self {
        Self {
            initialized: true,
            options,
            documents: HashMap::new(),
            graphs: HashMap::new(),
        }
    }
}

/// 初始化 WASM runtime
pub fn init_runtime(options: RuntimeOptions) -> BrowserAnalysisEnvelope {
    let runtime = BrowserRuntime::new(options);
    let _ = RUNTIME.set(Mutex::new(runtime));

    BrowserAnalysisEnvelope {
        status: AnalysisStatus::Ready,
        target: None,
        items: vec![],
        diagnostics: vec![],
    }
}

/// 查询 runtime 状态
pub fn runtime_status() -> BrowserAnalysisEnvelope {
    match RUNTIME.get() {
        Some(rt) => {
            let rt = rt.lock().unwrap();
            let doc_count = rt.documents.len();
            let graph_count = rt.graphs.len();
            BrowserAnalysisEnvelope {
                status: if rt.initialized {
                    AnalysisStatus::Ready
                } else {
                    AnalysisStatus::Initializing
                },
                target: None,
                items: vec![AnalysisItem {
                    kind: "runtime_status".to_string(),
                    label: "Runtime Status".to_string(),
                    detail: serde_json::json!({
                        "initialized": rt.initialized,
                        "document_count": doc_count,
                        "graph_count": graph_count,
                    }),
                }],
                diagnostics: vec![],
            }
        }
        None => BrowserAnalysisEnvelope {
            status: AnalysisStatus::Error,
            target: None,
            items: vec![],
            diagnostics: vec![AnalysisDiagnostic {
                severity: "error".to_string(),
                code: "RUNTIME_NOT_INITIALIZED".to_string(),
                message: "Runtime has not been initialized. Call init_runtime first.".to_string(),
            }],
        },
    }
}

/// 加载 SuperPage 文档（原始 JSON 文本）
pub fn load_superpage_document(
    source_path: &str,
    raw_text: &str,
) -> BrowserAnalysisEnvelope {
    let rt = match RUNTIME.get() {
        Some(r) => r,
        None => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: Some(source_path.to_string()),
                items: vec![],
                diagnostics: vec![AnalysisDiagnostic {
                    severity: "error".to_string(),
                    code: "RUNTIME_NOT_INITIALIZED".to_string(),
                    message: "Runtime has not been initialized. Call init_runtime first."
                        .to_string(),
                }],
            }
        }
    };

    let raw_value: serde_json::Value = match serde_json::from_str(raw_text) {
        Ok(v) => v,
        Err(e) => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: Some(source_path.to_string()),
                items: vec![],
                diagnostics: vec![AnalysisDiagnostic {
                    severity: "error".to_string(),
                    code: "PARSE_ERROR".to_string(),
                    message: format!("Failed to parse JSON: {}", e),
                }],
            }
        }
    };

    let meta = match superpage::parse_superpage_from_value(raw_value) {
        Ok(m) => m,
        Err(e) => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: Some(source_path.to_string()),
                items: vec![],
                diagnostics: vec![AnalysisDiagnostic {
                    severity: "error".to_string(),
                    code: "METADATA_PARSE_ERROR".to_string(),
                    message: format!("Failed to parse SuperPage metadata: {}", e),
                }],
            }
        }
    };

    let mut rt = rt.lock().unwrap();
    rt.documents.insert(source_path.to_string(), meta);

    BrowserAnalysisEnvelope {
        status: AnalysisStatus::Ready,
        target: Some(source_path.to_string()),
        items: vec![AnalysisItem {
            kind: "document_loaded".to_string(),
            label: "Document Loaded".to_string(),
            detail: serde_json::json!({
                "source_path": source_path,
                "component_count": rt.documents[source_path].components.len(),
                "expression_count": rt.documents[source_path].expressions.len(),
            }),
        }],
        diagnostics: vec![],
    }
}

/// 为已加载的 SuperPage 构建或更新内存图
pub fn build_or_update_superpage_graph(source_path: &str) -> BrowserAnalysisEnvelope {
    let rt = match RUNTIME.get() {
        Some(r) => r,
        None => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: Some(source_path.to_string()),
                items: vec![],
                diagnostics: vec![AnalysisDiagnostic {
                    severity: "error".to_string(),
                    code: "RUNTIME_NOT_INITIALIZED".to_string(),
                    message: "Runtime has not been initialized. Call init_runtime first."
                        .to_string(),
                }],
            }
        }
    };

    let mut rt = rt.lock().unwrap();
    let meta = match rt.documents.get(source_path) {
        Some(m) => m.clone(),
        None => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: Some(source_path.to_string()),
                items: vec![],
                diagnostics: vec![AnalysisDiagnostic {
                    severity: "error".to_string(),
                    code: "DOCUMENT_NOT_FOUND".to_string(),
                    message: format!("Document not found for source_path: {}", source_path),
                }],
            }
        }
    };

    let mut store = MemoryGraphStore::new();
    build_spg_graph(&meta, source_path, &mut store);
    rt.graphs.insert(source_path.to_string(), store);

    let store = &rt.graphs[source_path];
    let node_count = store.node_count().unwrap_or(0);
    let edge_count = store.edge_count().unwrap_or(0);

    BrowserAnalysisEnvelope {
        status: AnalysisStatus::Ready,
        target: Some(source_path.to_string()),
        items: vec![AnalysisItem {
            kind: "graph_built".to_string(),
            label: "Graph Built".to_string(),
            detail: serde_json::json!({
                "source_path": source_path,
                "node_count": node_count,
                "edge_count": edge_count,
            }),
        }],
        diagnostics: vec![],
    }
}

/// 分析选中组件
pub fn analyze_superpage_selection(
    selection: SuperPageSelection,
    options: AnalysisOptions,
) -> BrowserAnalysisEnvelope {
    let rt = match RUNTIME.get() {
        Some(r) => r,
        None => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: Some(selection.source_path.clone()),
                items: vec![],
                diagnostics: vec![AnalysisDiagnostic {
                    severity: "error".to_string(),
                    code: "RUNTIME_NOT_INITIALIZED".to_string(),
                    message: "Runtime has not been initialized. Call init_runtime first."
                        .to_string(),
                }],
            }
        }
    };

    let rt = rt.lock().unwrap();
    let meta = match rt.documents.get(&selection.source_path) {
        Some(m) => m,
        None => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: Some(selection.source_path.clone()),
                items: vec![],
                diagnostics: vec![AnalysisDiagnostic {
                    severity: "error".to_string(),
                    code: "DOCUMENT_NOT_FOUND".to_string(),
                    message: format!(
                        "Document not found for source_path: {}",
                        selection.source_path
                    ),
                }],
            }
        }
    };

    let graph = DependencyGraph::new(meta);
    let target_id = selection
        .active_component_id
        .as_deref()
        .or_else(|| selection.selected_component_ids.first().map(String::as_str))
        .unwrap_or("");

    if target_id.is_empty() {
        return BrowserAnalysisEnvelope {
            status: AnalysisStatus::Partial,
            target: Some(selection.source_path.clone()),
            items: vec![],
            diagnostics: vec![AnalysisDiagnostic {
                severity: "info".to_string(),
                code: "EMPTY_SELECTION".to_string(),
                message: "No component selected.".to_string(),
            }],
        };
    }

    let mut items = Vec::new();

    // 组件基本信息
    if let Some(comp) = meta.components.iter().find(|c| c.id == target_id) {
        items.push(AnalysisItem {
            kind: "component".to_string(),
            label: format!("Component: {}", comp.id),
            detail: serde_json::json!({
                "id": comp.id,
                "component_type": comp.component_type,
            }),
        });
    }

    // 依赖关系
    if let Some(deps) = graph.dependencies.get(target_id) {
        let dep_ids: Vec<String> = deps
            .iter()
            .filter_map(|r| match r {
                superpage::RefType::ComponentValue(id)
                | superpage::RefType::ComponentProperty(id, _) => Some(id.clone()),
                _ => None,
            })
            .collect();
        if !dep_ids.is_empty() {
            items.push(AnalysisItem {
                kind: "dependencies".to_string(),
                label: "Dependencies".to_string(),
                detail: serde_json::json!({ "depends_on": dep_ids }),
            });
        }
    }

    // 反向依赖
    if let Some(reverse) = graph.reverse_deps.get(target_id) {
        if !reverse.is_empty() {
            items.push(AnalysisItem {
                kind: "reverse_dependencies".to_string(),
                label: "Used By".to_string(),
                detail: serde_json::json!({ "used_by": reverse }),
            });
        }
    }

    // 表达式
    if let Some(exprs) = graph.expressions.get(target_id) {
        let expr_details: Vec<serde_json::Value> = exprs
            .iter()
            .map(|e| {
                serde_json::json!({
                    "field": e.field,
                    "expression": e.raw_expr,
                })
            })
            .collect();
        if !expr_details.is_empty() {
            items.push(AnalysisItem {
                kind: "expressions".to_string(),
                label: "Expressions".to_string(),
                detail: serde_json::json!({ "expressions": expr_details }),
            });
        }
    }

    // 图存储中的边（如果图已构建）
    if let Some(store) = rt.graphs.get(&selection.source_path) {
        if let Ok(Some(neighbors)) = store.get_node_edges(target_id) {
            let reads: Vec<String> = neighbors
                .outgoing
                .iter()
                .filter(|ev| matches!(ev.edge.edge_type, EdgeType::Reads))
                .map(|ev| ev.node.id.clone())
                .collect();
            let writes: Vec<String> = neighbors
                .outgoing
                .iter()
                .filter(|ev| matches!(ev.edge.edge_type, EdgeType::Writes | EdgeType::ActionWrites))
                .map(|ev| ev.node.id.clone())
                .collect();
            if !reads.is_empty() {
                items.push(AnalysisItem {
                    kind: "reads".to_string(),
                    label: "Reads".to_string(),
                    detail: serde_json::json!({ "targets": reads }),
                });
            }
            if !writes.is_empty() {
                items.push(AnalysisItem {
                    kind: "writes".to_string(),
                    label: "Writes".to_string(),
                    detail: serde_json::json!({ "targets": writes }),
                });
            }
        }
    }

    // 条件分析
    if options.include_conditions {
        let comp_exprs: Vec<&superpage::ComponentExpr> = meta
            .expressions
            .iter()
            .filter(|e| e.component_id == target_id)
            .collect();
        let mut conditions = Vec::new();
        for expr in &comp_exprs {
            if expr.field.contains("Condition") || expr.field.contains("condition") {
                conditions.push(expr.field.clone());
            }
        }
        if !conditions.is_empty() {
            items.push(AnalysisItem {
                kind: "conditions".to_string(),
                label: "Conditions".to_string(),
                detail: serde_json::json!({ "fields": conditions }),
            });
        }
    }

    BrowserAnalysisEnvelope {
        status: AnalysisStatus::Ready,
        target: Some(target_id.to_string()),
        items,
        diagnostics: vec![],
    }
}

/// 从 SuperPage 元数据构建内存图
fn build_spg_graph(meta: &superpage::SuperPageMetadata, source_path: &str, store: &mut MemoryGraphStore) {
    let page_id = format!("page:{}", source_path);
    store
        .upsert_node(Node {
            id: page_id.clone(),
            node_type: NodeType::Page,
            path: source_path.to_string(),
            name: source_path.to_string(),
            meta: None,
        })
        .ok();

    for comp in &meta.components {
        let comp_id = format!("comp:{}|{}", source_path, comp.id);
        store
            .upsert_node(Node {
                id: comp_id.clone(),
                node_type: NodeType::Component,
                path: source_path.to_string(),
                name: comp.id.clone(),
                meta: None,
            })
            .ok();
        store
            .add_edge(crate::graph::Edge {
                from: page_id.clone(),
                to: comp_id.clone(),
                edge_type: EdgeType::Contains,
                field_path: None,
                meta: None,
            })
            .ok();

        // 数据源读取边
        for source in &meta.sources {
            let model_id = format!("model:{}|{}", source_path, source.id);
            store
                .upsert_node(Node {
                    id: model_id.clone(),
                    node_type: NodeType::Model,
                    path: source_path.to_string(),
                    name: source.id.clone(),
                    meta: None,
                })
                .ok();
            store
                .add_edge(crate::graph::Edge {
                    from: comp_id.clone(),
                    to: model_id,
                    edge_type: EdgeType::Reads,
                    field_path: None,
                    meta: None,
                })
                .ok();
        }
    }
}
