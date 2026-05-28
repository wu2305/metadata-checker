//! Browser WASM runtime API
//!
//! M40.2：提供 browser-only WASM API，不改变现有 CLI/stdio 输出 schema。
//! 所有函数返回统一的 BrowserAnalysisEnvelope。

use crate::browser_orchestrator::{
    AnalysisArtifactKey, AnalysisArtifactScope, BackgroundProgress, BackgroundScanTask,
    AnalysisPriority, BrowserAnalysisOrchestrator, OrchestratorTaskDescriptor, QueueStatus,
};
use crate::dependency::DependencyGraph;
use crate::graph::{EdgeType, Node, NodeType};
use crate::graph_store::{GraphReadStore, GraphWriteStore};
use crate::memory_graph_store::MemoryGraphStore;
use crate::superpage;
use crate::visualization::graph_model::{SourceSummary, VisualEdge, VisualGraph, VisualNode};
use crate::visualization::options::{EdgeDirection, EdgeKind, NodeKind};
use crate::visualization::sanitizer::{
    sanitize_identity_id, sanitize_metadata_entry, sanitize_text,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
        queued_task_descriptors.push(
            orchestrator_task_descriptor_value(&OrchestratorTaskDescriptor {
                task_id,
                source_path: task.source_path.clone(),
                project_name: task.project_name.clone(),
                file_id: task.file_id.clone(),
                revision: task.revision.clone(),
                scope: AnalysisArtifactScope::Background,
                generation,
                active_component_id: None,
                selected_component_ids: Vec::new(),
            }),
        );
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

    let rt = rt.lock().unwrap();
    let meta = match rt.documents.get(&selection.source_path) {
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

    let graph = DependencyGraph::new(meta);
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
        return BrowserAnalysisEnvelope {
            status: AnalysisStatus::Partial,
            target: Some(selection.source_path.clone()),
            items: vec![],
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

    let visual_graph = build_selection_visual_graph(
        meta,
        rt.graphs.get(&selection.source_path),
        &graph,
        &selection.source_path,
        target_id,
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
    evidence: Option<String>,
) {
    graph.edges.push(VisualEdge {
        from: from.to_string(),
        to: to.to_string(),
        edge_type: Some(kind.to_string()),
        label: Some(sanitize_text(label)),
        direction: EdgeDirection::Forward,
        evidence: evidence.map(|value| sanitize_text(&value)),
        kind,
    });
}

fn build_selection_visual_graph(
    meta: &superpage::SuperPageMetadata,
    store: Option<&MemoryGraphStore>,
    dependency_graph: &DependencyGraph,
    source_path: &str,
    target_id: &str,
) -> VisualGraph {
    let mut visual_graph = VisualGraph::empty();
    let mut seen_nodes = HashSet::new();
    let raw_focus_id = format!("comp:{}|{}", source_path, target_id);
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

    if let Some(deps) = dependency_graph.dependencies.get(target_id) {
        for dep in deps {
            let dep_id = match dep {
                superpage::RefType::ComponentValue(id)
                | superpage::RefType::ComponentProperty(id, _) => id,
                _ => continue,
            };
            let raw_dep_id = format!("comp:{}|{}", source_path, dep_id);
            let dep_node_id = insert_visual_node(
                &mut visual_graph,
                &mut seen_nodes,
                &raw_dep_id,
                dep_id,
                NodeKind::Component,
                source_path,
                1,
            );
            push_visual_edge(
                &mut visual_graph,
                &dep_node_id,
                &focus_id,
                EdgeKind::DependsOn,
                "depends_on",
                None,
            );
        }
    }

    if let Some(reverse) = dependency_graph.reverse_deps.get(target_id) {
        for reverse_id in reverse {
            let raw_reverse_id = format!("comp:{}|{}", source_path, reverse_id);
            let reverse_node_id = insert_visual_node(
                &mut visual_graph,
                &mut seen_nodes,
                &raw_reverse_id,
                reverse_id,
                NodeKind::Component,
                source_path,
                1,
            );
            push_visual_edge(
                &mut visual_graph,
                &focus_id,
                &reverse_node_id,
                EdgeKind::DependsOn,
                "used_by",
                None,
            );
        }
    }

    for expr in meta
        .expressions
        .iter()
        .filter(|expr| expr.component_id == target_id)
    {
        let raw_expr_id = format!("expr:{}|{}|{}", source_path, target_id, expr.field);
        let expr_id = insert_visual_node(
            &mut visual_graph,
            &mut seen_nodes,
            &raw_expr_id,
            &expr.field,
            NodeKind::Condition,
            source_path,
            1,
        );
        push_visual_edge(
            &mut visual_graph,
            &expr_id,
            &focus_id,
            EdgeKind::DependsOn,
            "condition_or_expression",
            Some(expr.raw_expr.clone()),
        );
    }

    if let Some(store) = store {
        if let Ok(Some(neighbors)) = store.get_node_edges(&raw_focus_id) {
            for edge_view in &neighbors.outgoing {
                let (edge_kind, label) = match edge_view.edge.edge_type {
                    EdgeType::Reads => (EdgeKind::Reads, "reads"),
                    EdgeType::Writes | EdgeType::ActionWrites => (EdgeKind::Writes, "writes"),
                    EdgeType::Triggers => (EdgeKind::Triggers, "triggers"),
                    EdgeType::Contains => (EdgeKind::Contains, "contains"),
                    _ => (
                        EdgeKind::Other(format!("{:?}", edge_view.edge.edge_type)),
                        "related",
                    ),
                };
                let node_kind = match edge_view.node.node_type {
                    NodeType::Model => NodeKind::Model,
                    NodeType::Field => NodeKind::Field,
                    NodeType::Action => NodeKind::Action,
                    NodeType::Component => NodeKind::Component,
                    NodeType::Page => NodeKind::Page,
                    _ => NodeKind::Model,
                };
                let to_id = insert_visual_node(
                    &mut visual_graph,
                    &mut seen_nodes,
                    &edge_view.node.id,
                    &edge_view.node.name,
                    node_kind,
                    &edge_view.node.path,
                    1,
                );
                push_visual_edge(&mut visual_graph, &focus_id, &to_id, edge_kind, label, None);
            }
        }
    }

    let mut node_kinds = HashMap::new();
    for node in &visual_graph.nodes {
        *node_kinds.entry(node.kind.to_string()).or_insert(0) += 1;
    }
    let mut edge_kinds = HashMap::new();
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
