//! Browser WASM runtime API
//!
//! M40.2：提供 browser-only WASM API，不改变现有 CLI/stdio 输出 schema。
//! 所有函数返回统一的 BrowserAnalysisEnvelope。

use crate::browser_orchestrator::{
    AnalysisArtifactKey, AnalysisArtifactScope, AnalysisPriority, BackgroundProgress,
    BackgroundScanTask, BrowserAnalysisOrchestrator, OrchestratorTaskDescriptor, QueueStatus,
};
use crate::dependency::DependencyGraph;
use crate::graph::{EdgeType, Node, NodeType};
use crate::graph_store::{GraphReadStore, GraphWriteStore};
use crate::memory_graph_store::MemoryGraphStore;
use crate::superpage;
use crate::visualization::graph_model::{
    SourceSummary, VisualEdge, VisualGraph, VisualNode, classify_edge_priority,
    edge_evidence_status, summarize_edge,
};
use crate::visualization::options::{EdgeDirection, EdgeKind, NodeKind};
use crate::visualization::sanitizer::{
    sanitize_identity_id, sanitize_metadata_entry, sanitize_text,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
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
    #[serde(default)]
    pub include_priority: bool,
    #[serde(default)]
    pub include_conditions: bool,
    #[serde(default)]
    pub include_dataflow: bool,
    /// Local Graph depth，当前固定为 2
    #[serde(default)]
    pub depth: Option<usize>,
    /// Local Graph 可见 hop，当前固定为 1
    #[serde(default)]
    pub visible_hop: Option<usize>,
    /// Local Graph 最大节点数
    #[serde(default)]
    pub max_nodes: Option<usize>,
    /// Local Graph 最大边数
    #[serde(default)]
    pub max_edges: Option<usize>,
}

const LOCAL_GRAPH_DEPTH: usize = 2;
const LOCAL_GRAPH_VISIBLE_HOP: usize = 1;
const LOCAL_GRAPH_MAX_NODES: usize = 200;
const LOCAL_GRAPH_MAX_EDGES: usize = 500;

/// Orchestrator 前台 selection 入队请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorForegroundSelection {
    pub source_path: String,
    #[serde(default)]
    pub file_id: Option<String>,
    #[serde(default)]
    pub selected_component_ids: Vec<String>,
    #[serde(default)]
    pub active_component_id: Option<String>,
}

/// Orchestrator 入队参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorForegroundRequestOptions {
    pub project_name: Option<String>,
    pub revision: Option<String>,
    #[serde(default)]
    pub generation: Option<u64>,
    #[serde(default)]
    pub processing_ticks: Option<u64>,
}

impl Default for OrchestratorForegroundRequestOptions {
    fn default() -> Self {
        Self {
            project_name: None,
            revision: None,
            generation: Some(1),
            processing_ticks: Some(1),
        }
    }
}

/// Orchestrator 后台任务入队请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorBackgroundTaskRequest {
    pub source_path: String,
    pub file_id: Option<String>,
    pub project_name: Option<String>,
    pub revision: Option<String>,
    #[serde(default)]
    pub generation: Option<u64>,
    #[serde(default)]
    pub processing_ticks: Option<u64>,
}

/// Browser runtime 内部状态
#[derive(Debug)]
pub struct BrowserRuntime {
    pub initialized: bool,
    pub options: RuntimeOptions,
    pub documents: HashMap<String, superpage::SuperPageMetadata>,
    pub graphs: HashMap<String, MemoryGraphStore>,
    pub orchestrator: BrowserAnalysisOrchestrator,
}

impl BrowserRuntime {
    fn new(options: RuntimeOptions) -> Self {
        Self {
            initialized: true,
            options,
            documents: HashMap::new(),
            graphs: HashMap::new(),
            orchestrator: BrowserAnalysisOrchestrator::new(1, 0),
        }
    }
}

impl Default for BrowserRuntime {
    fn default() -> Self {
        Self::new(RuntimeOptions::default())
    }
}

/// 初始化 WASM runtime
pub fn init_runtime(options: RuntimeOptions) -> BrowserAnalysisEnvelope {
    let runtime = BrowserRuntime::new(options.clone());
    match RUNTIME.set(Mutex::new(runtime)) {
        Ok(_) => {}
        Err(_) => {
            // 已初始化，替换内部状态
            if let Some(rt) = RUNTIME.get() {
                let mut rt = rt.lock().unwrap();
                *rt = BrowserRuntime::new(options);
            }
        }
    }

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

fn orchestration_error_diagnostic(
    code: &str,
    message: impl Into<String>,
) -> Vec<AnalysisDiagnostic> {
    vec![AnalysisDiagnostic {
        severity: "error".to_string(),
        code: code.to_string(),
        message: message.into(),
    }]
}

fn queue_status_to_str(status: QueueStatus) -> &'static str {
    match status {
        QueueStatus::Idle => "idle",
        QueueStatus::Running => "running",
        QueueStatus::Paused => "paused",
        QueueStatus::Completed => "completed",
    }
}

fn progress_to_item(progress: &BackgroundProgress, label: impl Into<String>) -> AnalysisItem {
    AnalysisItem {
        kind: "orchestrator_progress".to_string(),
        label: label.into(),
        detail: serde_json::json!({
            "status": queue_status_to_str(progress.status),
            "processed": progress.processed,
            "total": progress.total,
            "active": progress.active,
            "queued": progress.queued,
            "max_concurrency": progress.max_concurrency,
            "limit": progress.limit,
            "min_interval_ticks": progress.min_interval_ticks,
        }),
    }
}

fn orchestrator_progress_value(progress: &BackgroundProgress) -> serde_json::Value {
    serde_json::json!({
        "status": queue_status_to_str(progress.status),
        "processed": progress.processed,
        "total": progress.total,
        "active": progress.active,
        "queued": progress.queued,
        "max_concurrency": progress.max_concurrency,
        "limit": progress.limit,
        "min_interval_ticks": progress.min_interval_ticks,
    })
}

fn artifact_scope_to_string(scope: AnalysisArtifactScope) -> &'static str {
    match scope {
        AnalysisArtifactScope::Foreground => "foreground",
        AnalysisArtifactScope::Background => "background",
    }
}

fn orchestrator_task_descriptor_value(task: &OrchestratorTaskDescriptor) -> serde_json::Value {
    serde_json::json!({
        "task_id": task.task_id,
        "source_path": task.source_path,
        "project_name": task.project_name,
        "file_id": task.file_id,
        "revision": task.revision,
        "scope": artifact_scope_to_string(task.scope),
        "generation": task.generation,
        "active_component_id": task.active_component_id,
        "selected_component_ids": task.selected_component_ids,
    })
}

fn contains_restricted_selection_fields(value: &serde_json::Value) -> bool {
    matches!(value, serde_json::Value::Object(obj)
        if obj.contains_key("raw_text")
            || obj.contains_key("rawText")
            || obj.contains_key("raw_metadata"))
}

fn normalize_orchestrator_generation(generation: Option<u64>) -> u64 {
    generation.unwrap_or(1).max(1)
}

fn normalize_orchestrator_ticks(ticks: Option<u64>) -> u64 {
    ticks.unwrap_or(1).max(1)
}

const ORCHESTRATOR_PROGRESS_LIMIT: usize = 16;

/// 查询 orchestrator 状态摘要
pub fn orchestrator_status(limit: usize) -> BrowserAnalysisEnvelope {
    let rt = match RUNTIME.get() {
        Some(r) => r,
        None => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: None,
                items: vec![],
                diagnostics: orchestration_error_diagnostic(
                    "RUNTIME_NOT_INITIALIZED",
                    "Runtime has not been initialized. Call init_runtime first.",
                ),
            };
        }
    };

    let rt = rt.lock().unwrap();
    let progress = rt.orchestrator.get_progress(limit);
    BrowserAnalysisEnvelope {
        status: AnalysisStatus::Ready,
        target: None,
        items: vec![progress_to_item(&progress, "Orchestrator Progress")],
        diagnostics: vec![],
    }
}

/// 队列入队前台 orchestrator 请求
pub fn enqueue_orchestrator_foreground_selection(
    selection_json: &str,
    request_options_json: &str,
) -> BrowserAnalysisEnvelope {
    let rt = match RUNTIME.get() {
        Some(r) => r,
        None => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: None,
                items: vec![],
                diagnostics: orchestration_error_diagnostic(
                    "RUNTIME_NOT_INITIALIZED",
                    "Runtime has not been initialized. Call init_runtime first.",
                ),
            };
        }
    };

    let selection_value: serde_json::Value = match serde_json::from_str(selection_json) {
        Ok(v) => v,
        Err(e) => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: None,
                items: vec![],
                diagnostics: orchestration_error_diagnostic(
                    "INVALID_ORCHESTRATOR_SELECTION",
                    format!("Failed to parse selection JSON: {e}"),
                ),
            };
        }
    };

    if contains_restricted_selection_fields(&selection_value) {
        return BrowserAnalysisEnvelope {
            status: AnalysisStatus::Error,
            target: None,
            items: vec![],
            diagnostics: orchestration_error_diagnostic(
                "INVALID_ORCHESTRATOR_SELECTION",
                "Selection payload contains forbidden fields raw_text/rawText/raw_metadata",
            ),
        };
    }

    let options: OrchestratorForegroundRequestOptions =
        match serde_json::from_str(request_options_json) {
            Ok(v) => v,
            Err(e) => {
                return BrowserAnalysisEnvelope {
                    status: AnalysisStatus::Error,
                    target: None,
                    items: vec![],
                    diagnostics: orchestration_error_diagnostic(
                        "INVALID_ORCHESTRATOR_REQUEST_OPTIONS",
                        format!("Failed to parse orchestrator request options JSON: {e}"),
                    ),
                };
            }
        };

    let selection: OrchestratorForegroundSelection = match serde_json::from_value(selection_value) {
        Ok(v) => v,
        Err(e) => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: None,
                items: vec![],
                diagnostics: orchestration_error_diagnostic(
                    "INVALID_ORCHESTRATOR_SELECTION",
                    format!("Failed to parse orchestrator selection payload: {e}"),
                ),
            };
        }
    };

    let mut rt = rt.lock().unwrap();
    let generation = normalize_orchestrator_generation(options.generation);
    let processing_ticks = normalize_orchestrator_ticks(options.processing_ticks);

    let key = AnalysisArtifactKey::foreground(
        options.project_name.clone(),
        selection.source_path.clone(),
        selection.file_id.clone(),
        options.revision,
        selection.active_component_id.clone(),
        selection.selected_component_ids.clone(),
    );

    let result =
        rt.orchestrator
            .enqueue_foreground_request(key.clone(), generation, processing_ticks);
    let progress = rt.orchestrator.get_progress(ORCHESTRATOR_PROGRESS_LIMIT);

    let mut items = Vec::new();
    items.push(AnalysisItem {
        kind: "foreground_request".to_string(),
        label: "Foreground Request".to_string(),
        detail: serde_json::json!({
            "request_id": result.request_id,
            "task_id": result.task_id,
            "merged_with": result.merged_with,
            "cache_hit": result.cache_hit,
            "generation": generation,
            "processing_ticks": processing_ticks,
            "source_path": key.source_path,
            "project_name": key.project_name,
            "file_id": key.file_id,
            "active_component_id": key.active_component_id,
            "selected_component_ids": key.selected_component_ids,
            "scope": "foreground",
        }),
    });
    items.push(progress_to_item(&progress, "Orchestrator Progress"));
    BrowserAnalysisEnvelope {
        status: AnalysisStatus::Ready,
        target: Some(selection.source_path),
        items,
        diagnostics: vec![],
    }
}

/// 队列入队后台 orchestrator 任务
pub fn enqueue_orchestrator_background_tasks(tasks_json: &str) -> BrowserAnalysisEnvelope {
    let rt = match RUNTIME.get() {
        Some(r) => r,
        None => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: None,
                items: vec![],
                diagnostics: orchestration_error_diagnostic(
                    "RUNTIME_NOT_INITIALIZED",
                    "Runtime has not been initialized. Call init_runtime first.",
                ),
            };
        }
    };

    let tasks: Vec<OrchestratorBackgroundTaskRequest> = match serde_json::from_str(tasks_json) {
        Ok(v) => v,
        Err(e) => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: None,
                items: vec![],
                diagnostics: orchestration_error_diagnostic(
                    "INVALID_ORCHESTRATOR_TASKS",
                    format!("Failed to parse background tasks JSON: {e}"),
                ),
            };
        }
    };

    let mut rt = rt.lock().unwrap();
    let mut task_ids = Vec::new();
    let mut request_sources = Vec::new();
    let mut queued_task_descriptors = Vec::new();
    for task in tasks.iter() {
        let key = AnalysisArtifactKey::background(
            task.project_name.clone(),
            task.source_path.clone(),
            task.file_id.clone(),
            task.revision.clone(),
        );
        let generation = normalize_orchestrator_generation(task.generation);
        let processing_ticks = normalize_orchestrator_ticks(task.processing_ticks);
        let task_id = rt.orchestrator.enqueue_background_task(BackgroundScanTask {
            artifact_key: key,
            priority: AnalysisPriority::Background,
            generation,
            processing_ticks,
        });
        task_ids.push(task_id);
        request_sources.push(task.source_path.clone());
        queued_task_descriptors.push(orchestrator_task_descriptor_value(
            &OrchestratorTaskDescriptor {
                task_id,
                source_path: task.source_path.clone(),
                project_name: task.project_name.clone(),
                file_id: task.file_id.clone(),
                revision: task.revision.clone(),
                scope: AnalysisArtifactScope::Background,
                generation,
                active_component_id: None,
                selected_component_ids: Vec::new(),
            },
        ));
    }
    let progress = rt.orchestrator.get_progress(ORCHESTRATOR_PROGRESS_LIMIT);

    BrowserAnalysisEnvelope {
        status: AnalysisStatus::Ready,
        target: None,
        items: vec![
            AnalysisItem {
                kind: "orchestrator_progress".to_string(),
                label: "Background Tasks Enqueued".to_string(),
                detail: serde_json::json!({
                    "enqueued_task_count": task_ids.len(),
                    "enqueued_task_ids": task_ids,
                    "sources": request_sources,
                    "queued_task_descriptors": queued_task_descriptors,
                    "progress": orchestrator_progress_value(&progress),
                }),
            },
            progress_to_item(&progress, "Orchestrator Progress"),
        ],
        diagnostics: vec![],
    }
}

/// 推进 orchestrator 调度一轮
pub fn tick_orchestrator(logical_tick: u64, limit: usize) -> BrowserAnalysisEnvelope {
    let rt = match RUNTIME.get() {
        Some(r) => r,
        None => {
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: None,
                items: vec![],
                diagnostics: orchestration_error_diagnostic(
                    "RUNTIME_NOT_INITIALIZED",
                    "Runtime has not been initialized. Call init_runtime first.",
                ),
            };
        }
    };

    let mut rt = rt.lock().unwrap();
    let tick_result = rt.orchestrator.tick(logical_tick, limit);
    let progress = rt.orchestrator.get_progress(limit);
    let mut items = vec![AnalysisItem {
        kind: "orchestrator_tick".to_string(),
        label: "Orchestrator Tick".to_string(),
        detail: serde_json::json!({
            "logical_tick": logical_tick,
            "limit": limit,
            "started_task_ids": tick_result.started_task_ids,
            "started_task_descriptors": tick_result
                .started_task_descriptors
                .iter()
                .map(orchestrator_task_descriptor_value)
                .collect::<Vec<_>>(),
            "completed_task_ids": tick_result.completed_task_ids,
            "completed_task_descriptors": tick_result
                .completed_task_descriptors
                .iter()
                .map(orchestrator_task_descriptor_value)
                .collect::<Vec<_>>(),
            "completed_request_ids": tick_result.completed_request_ids,
            "progress": orchestrator_progress_value(&progress),
        }),
    }];

    for task in &tick_result.started_task_descriptors {
        items.push(AnalysisItem {
            kind: "orchestrator_started_task".to_string(),
            label: "Orchestrator Task Started".to_string(),
            detail: orchestrator_task_descriptor_value(task),
        });
    }

    for task in &tick_result.completed_task_descriptors {
        items.push(AnalysisItem {
            kind: "orchestrator_completed_task".to_string(),
            label: "Orchestrator Task Completed".to_string(),
            detail: orchestrator_task_descriptor_value(task),
        });
    }

    if !tick_result.completed_request_ids.is_empty() {
        for request_id in tick_result.completed_request_ids {
            if let Some(request) = rt.orchestrator.get_request(request_id) {
                items.push(AnalysisItem {
                    kind: "foreground_request".to_string(),
                    label: "Foreground Request Completed".to_string(),
                    detail: serde_json::json!({
                        "request_id": request_id,
                        "state": "completed",
                        "task_id": request.linked_task_id,
                        "scope": "foreground",
                        "source_path": request.artifact_key.source_path,
                        "project_name": request.artifact_key.project_name,
                        "file_id": request.artifact_key.file_id,
                        "revision": request.artifact_key.revision,
                        "active_component_id": request.artifact_key.active_component_id,
                        "selected_component_ids": request.artifact_key.selected_component_ids,
                        "generation": request.generation,
                        "artifact_ready_tick": request.artifact.as_ref().map(|artifact| artifact.ready_tick),
                    }),
                });
            }
        }
    }
    items.push(progress_to_item(&progress, "Orchestrator Progress"));

    BrowserAnalysisEnvelope {
        status: AnalysisStatus::Ready,
        target: None,
        items,
        diagnostics: vec![],
    }
}

/// 加载 SuperPage 文档（原始 JSON 文本）
pub fn load_superpage_document(source_path: &str, raw_text: &str) -> BrowserAnalysisEnvelope {
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
            };
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
            };
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
            };
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
            };
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
            };
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
    let mut normalized_depth = LOCAL_GRAPH_DEPTH;
    let mut normalized_visible_hop = LOCAL_GRAPH_VISIBLE_HOP;
    let max_nodes = options.max_nodes.unwrap_or(LOCAL_GRAPH_MAX_NODES).max(1);
    let max_edges = options.max_edges.unwrap_or(LOCAL_GRAPH_MAX_EDGES);

    let mut diagnostics = Vec::new();
    if options.include_priority {
        diagnostics.push(AnalysisDiagnostic {
            severity: "warning".to_string(),
            code: "UNSUPPORTED_OPTION".to_string(),
            message: "include_priority is not supported in this runtime yet.".to_string(),
        });
    }

    if options.include_dataflow {
        diagnostics.push(AnalysisDiagnostic {
            severity: "warning".to_string(),
            code: "UNSUPPORTED_OPTION".to_string(),
            message: "include_dataflow is not supported in this runtime yet.".to_string(),
        });
    }

    if options.depth != None && options.depth != Some(LOCAL_GRAPH_DEPTH) {
        normalized_depth = LOCAL_GRAPH_DEPTH;
        diagnostics.push(AnalysisDiagnostic {
            severity: "warning".to_string(),
            code: "UNSUPPORTED_OPTION".to_string(),
            message: format!("depth is fixed to {} in this runtime.", LOCAL_GRAPH_DEPTH),
        });
    }

    if options.visible_hop != None && options.visible_hop != Some(LOCAL_GRAPH_VISIBLE_HOP) {
        normalized_visible_hop = LOCAL_GRAPH_VISIBLE_HOP;
        diagnostics.push(AnalysisDiagnostic {
            severity: "warning".to_string(),
            code: "UNSUPPORTED_OPTION".to_string(),
            message: format!(
                "visible_hop is fixed to {} in this runtime.",
                LOCAL_GRAPH_VISIBLE_HOP
            ),
        });
    }

    let rt = match RUNTIME.get() {
        Some(r) => r,
        None => {
            let mut error_diagnostics = diagnostics;
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: Some(selection.source_path.clone()),
                items: vec![],
                diagnostics: {
                    error_diagnostics.push(AnalysisDiagnostic {
                        severity: "error".to_string(),
                        code: "RUNTIME_NOT_INITIALIZED".to_string(),
                        message: "Runtime has not been initialized. Call init_runtime first."
                            .to_string(),
                    });
                    error_diagnostics
                },
            };
        }
    };

    let mut rt = rt.lock().unwrap();
    let meta = match rt.documents.get(&selection.source_path).cloned() {
        Some(m) => m,
        None => {
            let mut error_diagnostics = diagnostics;
            return BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: Some(selection.source_path.clone()),
                items: vec![],
                diagnostics: {
                    error_diagnostics.push(AnalysisDiagnostic {
                        severity: "error".to_string(),
                        code: "DOCUMENT_NOT_FOUND".to_string(),
                        message: format!(
                            "Document not found for source_path: {}",
                            selection.source_path
                        ),
                    });
                    error_diagnostics
                },
            };
        }
    };

    let graph = DependencyGraph::new(&meta);
    let target_id = selection
        .active_component_id
        .as_deref()
        .or_else(|| selection.selected_component_ids.first().map(String::as_str))
        .unwrap_or("");

    if target_id.is_empty() {
        let mut error_diagnostics = diagnostics;
        error_diagnostics.push(AnalysisDiagnostic {
            severity: "info".to_string(),
            code: "EMPTY_SELECTION".to_string(),
            message: "No component selected.".to_string(),
        });
        let mut visual_graph = VisualGraph::empty();
        visual_graph.target = String::new();
        visual_graph.status = "idle".to_string();
        visual_graph.depth = normalized_depth;
        visual_graph.visible_hop = normalized_visible_hop;
        return BrowserAnalysisEnvelope {
            status: AnalysisStatus::Partial,
            target: Some(selection.source_path.clone()),
            items: vec![AnalysisItem {
                kind: "visual_graph".to_string(),
                label: "Visual Graph".to_string(),
                detail: serde_json::to_value(&visual_graph)
                    .unwrap_or_else(|_| serde_json::json!({})),
            }],
            diagnostics: error_diagnostics,
        };
    }

    let mut items = Vec::new();

    // 组件基本信息
    let comp_opt = meta.components.iter().find(|c| c.id == target_id);
    if let Some(comp) = comp_opt {
        items.push(AnalysisItem {
            kind: "component".to_string(),
            label: format!("Component: {}", comp.id),
            detail: serde_json::json!({
                "id": comp.id,
                "component_type": comp.component_type,
            }),
        });
    } else {
        let mut error_diagnostics = diagnostics;
        error_diagnostics.push(AnalysisDiagnostic {
            severity: "error".to_string(),
            code: "COMPONENT_NOT_FOUND".to_string(),
            message: format!(
                "Component '{}' not found in document '{}'",
                target_id, selection.source_path
            ),
        });
        return BrowserAnalysisEnvelope {
            status: AnalysisStatus::Error,
            target: Some(target_id.to_string()),
            items,
            diagnostics: error_diagnostics,
        };
    }

    // 每次分析都基于当前元数据重建内存图，避免 offscreen/后台缓存旧 blanket reads。
    let mut selection_store = MemoryGraphStore::new();
    build_spg_graph(&meta, &selection.source_path, &mut selection_store);
    rt.graphs
        .insert(selection.source_path.clone(), selection_store);

    let visual_graph = build_selection_visual_graph(
        &meta,
        rt.graphs.get(&selection.source_path),
        &graph,
        &selection.source_path,
        target_id,
        max_nodes,
        max_edges,
        normalized_depth,
        normalized_visible_hop,
    );
    items.push(AnalysisItem {
        kind: "visual_graph".to_string(),
        label: "Visual Graph".to_string(),
        detail: serde_json::to_value(visual_graph).unwrap_or_else(|_| serde_json::json!({})),
    });

    // 依赖关系
    if let Some(deps) = graph.dependencies.get(target_id) {
        let dep_ids: Vec<String> = deps
            .iter()
            .filter_map(|r| match r {
                superpage::RefType::ComponentValue(id, _)
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
    let graph_node_id = format!("comp:{}|{}", selection.source_path, target_id);
    if let Some(store) = rt.graphs.get(&selection.source_path) {
        if let Ok(Some(neighbors)) = store.get_node_edges(&graph_node_id) {
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
        diagnostics,
    }
}

fn insert_visual_node(
    graph: &mut VisualGraph,
    seen: &mut HashSet<String>,
    raw_id: &str,
    label: &str,
    kind: NodeKind,
    source_path: &str,
    depth: usize,
) -> String {
    let id = sanitize_identity_id(raw_id);
    if seen.insert(id.clone()) {
        graph.nodes.push(VisualNode {
            id: id.clone(),
            label: sanitize_text(label),
            kind,
            source_path: sanitize_text(source_path),
            depth: Some(depth),
            collapsed: depth > 3,
            importance: if depth == 0 {
                Some("focus".to_string())
            } else {
                None
            },
            expand_token: if depth > 2 {
                Some(sanitize_text(raw_id))
            } else {
                None
            },
            metadata: {
                let mut metadata = HashMap::new();
                metadata.insert(
                    "target".to_string(),
                    sanitize_metadata_entry("target", &serde_json::json!(raw_id)),
                );
                metadata
            },
        });
    }
    id
}

fn push_visual_edge(
    graph: &mut VisualGraph,
    from: &str,
    to: &str,
    kind: EdgeKind,
    label: &str,
    direction: EdgeDirection,
    evidence: Option<String>,
) {
    let sanitized_evidence = evidence.map(|value| sanitize_text(&value));
    let priority = classify_edge_priority(&kind, Some(label));
    let summary = summarize_edge(&kind, Some(label));
    let evidence_status = edge_evidence_status(&sanitized_evidence);
    graph.edges.push(VisualEdge {
        from: from.to_string(),
        to: to.to_string(),
        edge_type: Some(kind.to_string()),
        label: Some(sanitize_text(label)),
        direction,
        priority,
        summary,
        evidence_status,
        evidence: sanitized_evidence,
        kind,
    });
}

fn build_selection_visual_graph(
    meta: &superpage::SuperPageMetadata,
    store: Option<&MemoryGraphStore>,
    dependency_graph: &DependencyGraph,
    source_path: &str,
    target_id: &str,
    max_nodes: usize,
    max_edges: usize,
    depth: usize,
    visible_hop: usize,
) -> VisualGraph {
    fn component_raw_id(source_path: &str, component_id: &str) -> String {
        format!("comp:{}|{}", source_path, component_id)
    }

    fn extract_component_id<'a, 'b>(source_path: &'a str, node_id: &'b str) -> Option<&'b str> {
        let prefix = format!("comp:{}|", source_path);
        node_id.strip_prefix(prefix.as_str())
    }

    fn edge_key(from: &str, to: &str, kind: &EdgeKind, label: &str) -> String {
        format!("{}|{}|{}|{}", from, to, kind, label)
    }

    fn node_kind_by_graph_node(node: &Node) -> NodeKind {
        match node.node_type {
            NodeType::Model => NodeKind::Model,
            NodeType::Field => NodeKind::Field,
            NodeType::Action => NodeKind::Action,
            NodeType::Component => NodeKind::Component,
            NodeType::Page => NodeKind::Page,
            NodeType::Condition => NodeKind::Condition,
        }
    }

    fn edge_evidence_from_store(edge: &crate::graph::Edge) -> Option<String> {
        edge.meta
            .as_ref()
            .and_then(|value| {
                if value.is_string() {
                    value.as_str().map(|text| sanitize_text(text))
                } else {
                    Some(sanitize_text(&value.to_string()))
                }
            })
            .or_else(|| edge.field_path.as_ref().map(|text| sanitize_text(text)))
    }

    fn map_store_edge_kind(edge_type: &EdgeType) -> (EdgeKind, &str) {
        match edge_type {
            EdgeType::Reads => (EdgeKind::Reads, "reads"),
            EdgeType::Writes | EdgeType::ActionWrites => (EdgeKind::Writes, "writes"),
            EdgeType::Triggers => (EdgeKind::Triggers, "triggers"),
            EdgeType::Contains => (EdgeKind::Contains, "contains"),
            EdgeType::DataflowInput => (
                EdgeKind::Other("DataflowInput".to_string()),
                "dataflow_input",
            ),
            EdgeType::ActionReads => (EdgeKind::Other("ActionReads".to_string()), "action_reads"),
            EdgeType::DataflowOutput => (
                EdgeKind::Other("DataflowOutput".to_string()),
                "dataflow_output",
            ),
            EdgeType::OutputsTo => (EdgeKind::Other("OutputsTo".to_string()), "outputs_to"),
            EdgeType::DataflowInternal => (
                EdgeKind::Other("DataflowInternal".to_string()),
                "dataflow_internal",
            ),
            EdgeType::FieldAlias => (EdgeKind::Other("FieldAlias".to_string()), "field_alias"),
            EdgeType::FieldWrite => (EdgeKind::Other("FieldWrite".to_string()), "field_write"),
            EdgeType::EmbedsPage => (EdgeKind::Other("EmbedsPage".to_string()), "embeds_page"),
            EdgeType::OpensPage => (EdgeKind::Other("OpensPage".to_string()), "opens_page"),
            EdgeType::PassesParam => (EdgeKind::Other("PassesParam".to_string()), "passes_param"),
            EdgeType::SetsParam => (EdgeKind::Other("SetsParam".to_string()), "sets_param"),
            EdgeType::ActionNavigates => (
                EdgeKind::Other("ActionNavigates".to_string()),
                "action_navigates",
            ),
            EdgeType::ActionSetsParam => (
                EdgeKind::Other("ActionSetsParam".to_string()),
                "action_sets_param",
            ),
            EdgeType::ActionControlsComponent => (
                EdgeKind::Other("ActionControlsComponent".to_string()),
                "action_controls_component",
            ),
            EdgeType::ActionValidates => (
                EdgeKind::Other("ActionValidates".to_string()),
                "action_validates",
            ),
            EdgeType::ActionLoadsData => (
                EdgeKind::Other("ActionLoadsData".to_string()),
                "action_loads_data",
            ),
            EdgeType::DependsOn => (EdgeKind::DependsOn, "depends_on"),
        }
    }

    fn enqueue_depth(
        traversal_queue: &mut VecDeque<(String, usize, String)>,
        raw_id: &str,
        depth: usize,
        visual_id: &str,
        source_depth: usize,
    ) {
        if source_depth < 2
            && !traversal_queue
                .iter()
                .any(|(id, d, _)| id == raw_id && *d == depth)
        {
            traversal_queue.push_back((raw_id.to_string(), depth, visual_id.to_string()));
        }
    }

    let mut visual_graph = VisualGraph::empty();
    visual_graph.target = target_id.to_string();
    visual_graph.status = "ready".to_string();
    visual_graph.depth = depth;
    visual_graph.visible_hop = visible_hop;

    let mut seen_nodes = HashSet::new();
    let mut seen_edges = HashSet::new();
    let mut evidence_missing = false;
    let mut traversal_queue: VecDeque<(String, usize, String)> = VecDeque::new();

    let raw_focus_id = component_raw_id(source_path, target_id);
    let focus_id = insert_visual_node(
        &mut visual_graph,
        &mut seen_nodes,
        &raw_focus_id,
        target_id,
        NodeKind::Component,
        source_path,
        0,
    );
    visual_graph.focus_node = Some(focus_id.clone());
    traversal_queue.push_back((raw_focus_id.clone(), 0, focus_id.clone()));

    while let Some((raw_node, node_depth, current_visual_id)) = traversal_queue.pop_front() {
        let next_depth = node_depth.saturating_add(1);
        if node_depth >= depth {
            continue;
        }
        if next_depth > depth {
            continue;
        }

        if let Some(component_id) = extract_component_id(source_path, &raw_node) {
            if let Some(deps) = dependency_graph.dependencies.get(component_id) {
                for dep in deps {
                    let dep_id = match dep {
                        superpage::RefType::ComponentValue(id, _)
                        | superpage::RefType::ComponentProperty(id, _) => id,
                        _ => continue,
                    };
                    let raw_dep_id = component_raw_id(source_path, dep_id);
                    if raw_dep_id == raw_focus_id || raw_dep_id == raw_node {
                        continue;
                    }
                    let dep_node_id = insert_visual_node(
                        &mut visual_graph,
                        &mut seen_nodes,
                        &raw_dep_id,
                        dep_id,
                        NodeKind::Component,
                        source_path,
                        next_depth,
                    );
                    let key = edge_key(
                        &dep_node_id,
                        &current_visual_id,
                        &EdgeKind::DependsOn,
                        "depends_on",
                    );
                    if seen_edges.insert(key) {
                        push_visual_edge(
                            &mut visual_graph,
                            &dep_node_id,
                            &current_visual_id,
                            EdgeKind::DependsOn,
                            "depends_on",
                            EdgeDirection::Forward,
                            Some("dependency".to_string()),
                        );
                    }
                    if next_depth <= depth {
                        enqueue_depth(
                            &mut traversal_queue,
                            &raw_dep_id,
                            next_depth,
                            &dep_node_id,
                            node_depth,
                        );
                    }
                }
            }

            if let Some(reverse) = dependency_graph.reverse_deps.get(component_id) {
                for reverse_id in reverse {
                    let raw_reverse_id = component_raw_id(source_path, reverse_id);
                    if raw_reverse_id == raw_node {
                        continue;
                    }
                    let reverse_node_id = insert_visual_node(
                        &mut visual_graph,
                        &mut seen_nodes,
                        &raw_reverse_id,
                        reverse_id,
                        NodeKind::Component,
                        source_path,
                        next_depth,
                    );
                    let key = edge_key(
                        &current_visual_id,
                        &reverse_node_id,
                        &EdgeKind::DependsOn,
                        "used_by",
                    );
                    if seen_edges.insert(key) {
                        push_visual_edge(
                            &mut visual_graph,
                            &current_visual_id,
                            &reverse_node_id,
                            EdgeKind::DependsOn,
                            "used_by",
                            EdgeDirection::Forward,
                            Some("reverse_dependency".to_string()),
                        );
                    }
                    if next_depth <= depth {
                        enqueue_depth(
                            &mut traversal_queue,
                            &raw_reverse_id,
                            next_depth,
                            &reverse_node_id,
                            node_depth,
                        );
                    }
                }
            }

            for expr in meta
                .expressions
                .iter()
                .filter(|expr| expr.component_id == component_id)
            {
                let raw_expr_id = format!("expr:{}|{}|{}", source_path, component_id, expr.field);
                if raw_expr_id == raw_node {
                    continue;
                }
                let expr_node_id = insert_visual_node(
                    &mut visual_graph,
                    &mut seen_nodes,
                    &raw_expr_id,
                    &expr.field,
                    NodeKind::Condition,
                    source_path,
                    next_depth,
                );
                let key = edge_key(
                    &expr_node_id,
                    &current_visual_id,
                    &EdgeKind::DependsOn,
                    "condition_or_expression",
                );
                if seen_edges.insert(key) {
                    push_visual_edge(
                        &mut visual_graph,
                        &expr_node_id,
                        &current_visual_id,
                        EdgeKind::DependsOn,
                        "condition_or_expression",
                        EdgeDirection::Forward,
                        Some(expr.raw_expr.clone()),
                    );
                }
            }
        }

        if let Some(store) = store {
            if let Ok(Some(neighbors)) = store.get_node_edges(&raw_node) {
                for edge_view in &neighbors.outgoing {
                    if matches!(edge_view.edge.edge_type, EdgeType::Contains) {
                        continue;
                    }
                    let (edge_kind, label) = map_store_edge_kind(&edge_view.edge.edge_type);
                    let node_id = insert_visual_node(
                        &mut visual_graph,
                        &mut seen_nodes,
                        &edge_view.node.id,
                        &edge_view.node.name,
                        node_kind_by_graph_node(&edge_view.node),
                        &edge_view.node.path,
                        next_depth,
                    );
                    let key = edge_key(&current_visual_id, &node_id, &edge_kind, label);
                    if seen_edges.insert(key) {
                        let evidence = edge_evidence_from_store(&edge_view.edge);
                        if evidence.is_none() {
                            evidence_missing = true;
                        }
                        push_visual_edge(
                            &mut visual_graph,
                            &current_visual_id,
                            &node_id,
                            edge_kind,
                            label,
                            EdgeDirection::Forward,
                            evidence,
                        );
                    }
                    if next_depth <= depth {
                        enqueue_depth(
                            &mut traversal_queue,
                            &edge_view.node.id,
                            next_depth,
                            &node_id,
                            node_depth,
                        );
                    }
                }
                for edge_view in &neighbors.incoming {
                    if matches!(edge_view.edge.edge_type, EdgeType::Contains) {
                        continue;
                    }
                    let (edge_kind, label) = map_store_edge_kind(&edge_view.edge.edge_type);
                    let node_id = insert_visual_node(
                        &mut visual_graph,
                        &mut seen_nodes,
                        &edge_view.node.id,
                        &edge_view.node.name,
                        node_kind_by_graph_node(&edge_view.node),
                        &edge_view.node.path,
                        next_depth,
                    );
                    let key = edge_key(&node_id, &current_visual_id, &edge_kind, label);
                    if seen_edges.insert(key) {
                        let evidence = edge_evidence_from_store(&edge_view.edge);
                        if evidence.is_none() {
                            evidence_missing = true;
                        }
                        push_visual_edge(
                            &mut visual_graph,
                            &node_id,
                            &current_visual_id,
                            edge_kind,
                            label,
                            EdgeDirection::Forward,
                            evidence,
                        );
                    }
                    if next_depth <= depth {
                        enqueue_depth(
                            &mut traversal_queue,
                            &edge_view.node.id,
                            next_depth,
                            &node_id,
                            node_depth,
                        );
                    }
                }
            }
        }
    }

    if evidence_missing {
        visual_graph.status = "warning".to_string();
        visual_graph
            .diagnostics
            .push(crate::diagnostics::envelope_diagnostic(
                "EDGE_EVIDENCE_UNAVAILABLE",
                1,
                crate::output::schema::Location::default(),
                "Some edges are missing evidence details.",
            ));
    }

    let original_nodes = visual_graph.nodes.len();
    if visual_graph.nodes.len() > max_nodes {
        visual_graph.truncated = true;
        let mut truncated_reasons = Vec::new();
        truncated_reasons.push(format!("nodes ({} > {})", original_nodes, max_nodes));
        visual_graph.nodes.truncate(max_nodes);
        let kept_ids: HashSet<String> = visual_graph
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect();
        visual_graph
            .edges
            .retain(|edge| kept_ids.contains(&edge.from) && kept_ids.contains(&edge.to));
        visual_graph.truncated_reason = Some(truncated_reasons.join(","));
        visual_graph.status = "warning".to_string();
    }

    if visual_graph.edges.len() > max_edges {
        visual_graph.truncated = true;
        let mut truncated_reasons = Vec::new();
        let current_total = visual_graph.edges.len();
        if let Some(existing) = visual_graph.truncated_reason.clone() {
            truncated_reasons.push(existing);
        }
        truncated_reasons.push(format!("edges ({} > {})", current_total, max_edges));
        visual_graph.edges.truncate(max_edges);
        visual_graph.truncated_reason = Some(truncated_reasons.join(","));
        visual_graph.status = "warning".to_string();
    }

    if visual_graph.edges.is_empty() && !visual_graph.truncated {
        visual_graph.status = "empty".to_string();
    } else if evidence_missing || visual_graph.truncated {
        visual_graph.status = "warning".to_string();
    }

    let mut node_kinds: HashMap<String, usize> = HashMap::new();
    for node in &visual_graph.nodes {
        *node_kinds.entry(node.kind.to_string()).or_insert(0) += 1;
    }
    let mut edge_kinds: HashMap<String, usize> = HashMap::new();
    for edge in &visual_graph.edges {
        *edge_kinds.entry(edge.kind.to_string()).or_insert(0) += 1;
    }

    visual_graph.source_summary = SourceSummary {
        total_nodes: visual_graph.nodes.len(),
        total_edges: visual_graph.edges.len(),
        node_kinds,
        edge_kinds,
    };
    visual_graph
}

/// 从 SuperPage 元数据构建内存图
fn build_spg_graph(
    meta: &superpage::SuperPageMetadata,
    source_path: &str,
    store: &mut MemoryGraphStore,
) {
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

    for source in &meta.sources {
        let model_id = format!("model:{}|{}", source_path, source.id);
        store
            .upsert_node(Node {
                id: model_id,
                node_type: NodeType::Model,
                path: source_path.to_string(),
                name: source.id.clone(),
                meta: None,
            })
            .ok();
    }

    let dep_graph = DependencyGraph::new(meta);
    let mut model_read_keys: HashSet<(String, String)> = HashSet::new();

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

        let Some(refs) = dep_graph.dependencies.get(&comp.id) else {
            continue;
        };
        let comp_exprs = dep_graph.expressions.get(&comp.id);
        for ref_type in refs {
            let superpage::RefType::ModelField(model_name, field) = ref_type else {
                continue;
            };
            let dedupe_key = (comp_id.clone(), format!("{model_name}.{field}"));
            if !model_read_keys.insert(dedupe_key) {
                continue;
            }

            let model_node_id = format!("model:{}|{}", source_path, model_name);
            store
                .upsert_node(Node {
                    id: model_node_id.clone(),
                    node_type: NodeType::Model,
                    path: source_path.to_string(),
                    name: model_name.clone(),
                    meta: None,
                })
                .ok();

            let field_path = if field.is_empty() {
                model_name.clone()
            } else {
                format!("{model_name}.{field}")
            };
            let evidence = comp_exprs.and_then(|exprs| {
                exprs.iter().find_map(|expr| {
                    let references_model = expr.refs.iter().any(|candidate| {
                        matches!(
                            candidate,
                            superpage::RefType::ModelField(candidate_model, candidate_field)
                                if candidate_model == model_name && candidate_field == field
                        )
                    });
                    if references_model {
                        Some(serde_json::Value::String(expr.raw_expr.clone()))
                    } else {
                        None
                    }
                })
            });

            store
                .add_edge(crate::graph::Edge {
                    from: comp_id.clone(),
                    to: model_node_id,
                    edge_type: EdgeType::Reads,
                    field_path: Some(field_path),
                    meta: evidence,
                })
                .ok();
        }
    }
}
