use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use crate::dense_graph::DenseGraphSnapshot;
use crate::graph::GraphDB;
use crate::response_processor::ResponseProcessor;
pub use crate::response_processor::{RuntimeQueryResponse, RuntimeTiming};

/// Runtime 使用模式，用于区分一次性 CLI 查询和长生命周期服务。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeMode {
    /// 一次性查询路径，避免把初始化成本强行前移。
    OneShot,
    /// 长生命周期路径，允许在初始化阶段构建可复用读模型。
    LongLived,
}

/// Runtime 初始化阶段构建的只读派生模型。
pub struct RuntimeReadModel {
    /// 稠密 ID + CSR 风格只读图快照。
    pub dense_graph: Arc<DenseGraphSnapshot>,
    /// 预热后的页面 availability 缓存。
    pub page_logic_availability: HashMap<String, crate::query::PageLogicAvailabilityCache>,
}

impl RuntimeReadModel {
    /// 构造 page availability 缓存 key。
    fn page_logic_availability_key(page_id: &str, budget: &str) -> String {
        format!("{page_id}\n{budget}")
    }

    /// 读取预热后的 page availability 缓存。
    pub fn page_logic_availability(
        &self,
        page_id: &str,
        budget: &str,
    ) -> Option<&crate::query::PageLogicAvailabilityCache> {
        self.page_logic_availability
            .get(&Self::page_logic_availability_key(page_id, budget))
    }

    /// 判断是否已经预热指定 page/budget。
    pub fn has_page_logic_availability(&self, page_id: &str, budget: &str) -> bool {
        self.page_logic_availability(page_id, budget).is_some()
    }
}

/// Hot Graph Runtime：在同一进程内复用已加载的 GraphDB
///
/// M23 目标：把"加载图"和"执行查询"从 CLI 分支中解耦，
/// 证明同一个 runtime 连续执行多次查询时，第二次不再全量加载 graphdb。
pub struct GraphRuntime {
    /// 内存中的图数据库（全量加载）
    pub graph: GraphDB,
    /// graphdb 文件路径
    pub graph_db_path: PathBuf,
    /// 项目目录路径（从 graph_db_path 推导）
    pub project_dir: Option<PathBuf>,
    /// 加载时间戳
    pub loaded_at: SystemTime,
    /// graphdb 文件 mtime（用于后续增量检测）
    pub graph_file_mtime: Option<SystemTime>,
    /// graphdb 文件大小
    pub graph_file_size: u64,
    /// 累计加载次数（热查询应保持为 1）
    pub load_count: usize,
    /// 首次 graph 加载耗时（毫秒）
    pub graph_load_ms: u128,
    /// 累计 reload 次数
    pub reload_count: usize,
    /// 上次 reload 错误信息
    pub last_reload_error: Option<String>,
    /// graphdb 文件指纹
    pub graph_fingerprint: GraphFingerprint,
    /// 运行期预构建的稠密只读快照。
    pub dense_snapshot: Option<Arc<DenseGraphSnapshot>>,
    /// 稠密快照构建耗时（毫秒）。
    pub dense_snapshot_build_ms: u128,
    /// 是否在 runtime load/reload 阶段构建稠密快照。
    pub dense_snapshot_enabled: bool,
    /// Runtime 模式。
    pub runtime_mode: RuntimeMode,
    /// 长生命周期 runtime 的派生只读模型。
    pub read_model: Option<Arc<RuntimeReadModel>>,
    /// 派生只读模型构建耗时（毫秒）。
    pub read_model_build_ms: u128,
}

/// Runtime 查询命令枚举
///
/// M38 统一为 ToolCommand，消除 CLI / stdio / MCP 分叉。
pub type RuntimeQueryCommand = crate::tool_contract::ToolCommand;

/// Runtime 查询请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeQueryRequest {
    pub command: RuntimeQueryCommand,
    pub target: String,
    pub budget: String,
    pub human: bool,
    #[serde(default)]
    pub intent: Option<String>,
    pub page_scope: Option<String>,
    #[serde(default)]
    pub depth: Option<usize>,
    #[serde(default)]
    pub check_reload: bool,
}

/// GraphDB 文件指纹，用于变更检测
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphFingerprint {
    /// graphdb 文件路径
    pub path: PathBuf,
    /// 文件修改时间
    pub mtime: Option<SystemTime>,
    /// 文件大小
    pub size: u64,
    /// 文件前 4096 字节的内容 hash
    pub content_prefix_hash: u64,
}

/// 计算文件前 prefix_len 字节的内容 hash
fn compute_prefix_hash(path: &std::path::Path, prefix_len: usize) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    if let Ok(data) = std::fs::read(path) {
        data.iter()
            .take(prefix_len)
            .for_each(|b| b.hash(&mut hasher));
    }
    hasher.finish()
}

/// reload_if_changed 结果枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReloadResult {
    /// 文件未变更，未执行 reload
    Unchanged,
    /// 文件已变更，reload 成功
    Reloaded,
    /// 文件已变更，reload 失败
    ReloadFailed { error: String },
}

/// Runtime 状态快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatus {
    /// graphdb 路径
    pub graph_db_path: PathBuf,
    /// 加载时间戳（Unix 时间戳秒数）
    pub loaded_at: u64,
    /// 首次加载次数
    pub load_count: usize,
    /// 累计 reload 次数
    pub reload_count: usize,
    /// 图中节点数量
    pub node_count: usize,
    /// 图中边数量
    pub edge_count: usize,
    /// 文件修改时间
    pub graph_file_mtime: Option<SystemTime>,
    /// 文件大小
    pub graph_file_size: u64,
    /// 上次 reload 错误
    pub last_reload_error: Option<String>,
}

impl GraphRuntime {
    /// 加载 graphdb 并构建 Runtime
    ///
    /// 记录文件 metadata，初始化 load_count = 1。
    /// 当显式提供 `project_dir` 时优先使用，否则从 `graph_db_path.parent()` 推导。
    pub fn load_with_project_dir(
        graph_db_path: impl AsRef<Path>,
        project_dir: Option<impl AsRef<Path>>,
    ) -> Result<Self> {
        Self::load_with_project_dir_and_mode(graph_db_path, project_dir, RuntimeMode::OneShot)
    }

    /// 加载 graphdb 并显式构建稠密只读快照。
    pub fn load_with_project_dir_and_dense_snapshot(
        graph_db_path: impl AsRef<Path>,
        project_dir: Option<impl AsRef<Path>>,
    ) -> Result<Self> {
        Self::load_with_project_dir_and_mode(graph_db_path, project_dir, RuntimeMode::LongLived)
    }

    /// 按指定 Runtime 模式加载 graphdb。
    pub fn load_with_project_dir_and_mode(
        graph_db_path: impl AsRef<Path>,
        project_dir: Option<impl AsRef<Path>>,
        runtime_mode: RuntimeMode,
    ) -> Result<Self> {
        Self::load_with_project_dir_internal(graph_db_path, project_dir, runtime_mode)
    }

    fn load_with_project_dir_internal(
        graph_db_path: impl AsRef<Path>,
        project_dir: Option<impl AsRef<Path>>,
        runtime_mode: RuntimeMode,
    ) -> Result<Self> {
        let path = graph_db_path.as_ref().to_path_buf();
        #[cfg(feature = "telemetry")]
        let graph_load_span = crate::telemetry::graph_load_span(&path.to_string_lossy());
        #[cfg(feature = "telemetry")]
        let _graph_load_guard = graph_load_span.enter();

        let start = Instant::now();
        let graph = GraphDB::open_or_diagnostic(&path).map_err(|e| {
            anyhow::anyhow!(
                "GraphDB open failed: {}",
                serde_json::to_string(&e).unwrap_or_default()
            )
        })?;
        let graph_load_ms = start.elapsed().as_millis();

        let (graph_file_mtime, graph_file_size) = std::fs::metadata(&path)
            .map(|m| (m.modified().ok(), m.len()))
            .unwrap_or((None, 0));

        let mut diagnostics = Vec::new();
        diagnostics.push(format!("Graph loaded in {} ms", graph_load_ms));

        let prefix_hash = compute_prefix_hash(&path, 4096);
        let fingerprint = GraphFingerprint {
            path: path.clone(),
            mtime: graph_file_mtime,
            size: graph_file_size,
            content_prefix_hash: prefix_hash,
        };
        let build_read_model = runtime_mode == RuntimeMode::LongLived;
        let read_model_started = Instant::now();
        let read_model = if build_read_model {
            DenseGraphSnapshot::from_graph(&graph)
                .ok()
                .map(|dense_graph| {
                    Arc::new(RuntimeReadModel {
                        dense_graph: Arc::new(dense_graph),
                        page_logic_availability: HashMap::new(),
                    })
                })
        } else {
            None
        };
        let read_model_build_ms = if build_read_model {
            read_model_started.elapsed().as_millis()
        } else {
            0
        };
        let dense_snapshot = read_model
            .as_ref()
            .map(|model| Arc::clone(&model.dense_graph));
        let dense_snapshot_build_ms = read_model_build_ms;
        let dense_snapshot_enabled = build_read_model;

        let project_dir = project_dir
            .map(|p| p.as_ref().to_path_buf())
            .or_else(|| path.parent().map(|p| p.to_path_buf()));

        #[cfg(feature = "telemetry")]
        crate::telemetry::record_graph_load(
            &graph_load_span,
            graph_load_ms,
            graph.graph.node_count(),
            graph.graph.edge_count(),
        );

        Ok(GraphRuntime {
            graph,
            graph_db_path: path,
            project_dir,
            loaded_at: SystemTime::now(),
            graph_file_mtime,
            graph_file_size,
            load_count: 1,
            graph_load_ms,
            reload_count: 0,
            last_reload_error: None,
            graph_fingerprint: fingerprint,
            dense_snapshot,
            dense_snapshot_build_ms,
            dense_snapshot_enabled,
            runtime_mode,
            read_model,
            read_model_build_ms,
        })
    }

    /// 兼容旧签名：从 graph_db_path.parent() 推导 project_dir
    pub fn load(graph_db_path: impl AsRef<Path>) -> Result<Self> {
        Self::load_with_project_dir(graph_db_path, None::<&Path>)
    }

    /// 在长生命周期 runtime 的初始化阶段预热 page logic availability。
    pub fn warm_page_logic_availability(&mut self, page_id: &str, budget: &str) -> Result<u128> {
        let started_at = Instant::now();
        let cache = crate::query::build_page_logic_availability_cache(
            &self.graph,
            page_id,
            self.project_dir.as_deref(),
            budget,
        )?;
        let Some(read_model) = self.read_model.as_ref() else {
            return Err(anyhow::anyhow!(
                "page logic availability warm requires long-lived runtime read model"
            ));
        };
        let mut page_logic_availability = read_model.page_logic_availability.clone();
        page_logic_availability.insert(
            RuntimeReadModel::page_logic_availability_key(page_id, budget),
            cache,
        );
        self.read_model = Some(Arc::new(RuntimeReadModel {
            dense_graph: Arc::clone(&read_model.dense_graph),
            page_logic_availability,
        }));
        Ok(started_at.elapsed().as_millis())
    }

    /// 执行查询，复用内存中的 graph
    ///
    /// M38 统一入口：支持所有已加载 graphdb 后的运行期工具命令。
    pub fn query(&mut self, request: RuntimeQueryRequest) -> Result<RuntimeQueryResponse> {
        use crate::tool_contract::ToolCommand;
        let total_start = Instant::now();
        let mut diagnostics = Vec::new();
        #[cfg(feature = "telemetry")]
        let command_label = format!("{:?}", request.command);
        #[cfg(feature = "telemetry")]
        let query_span = crate::telemetry::runtime_query_span(
            &command_label,
            &request.target,
            &request.budget,
            request.intent.as_deref(),
            request.human,
            self.graph.graph.node_count(),
            self.graph.graph.edge_count(),
        );
        #[cfg(feature = "telemetry")]
        let _query_guard = query_span.enter();

        // ReloadGraph / CheckReload 需要可变借用 self，在 match 之前单独处理
        if request.command == ToolCommand::ReloadGraph {
            let result = match self.reload() {
                Ok(()) => {
                    diagnostics.push("GRAPH_RELOADED".to_string());
                    serde_json::to_value(self.status())?
                }
                Err(e) => {
                    diagnostics.push("GRAPH_RELOAD_FAILED".to_string());
                    serde_json::json!({
                        "ok": false,
                        "error": format!("{}", e),
                        "diagnostics": vec!["GRAPH_RELOAD_FAILED"],
                    })
                }
            };
            let query_compute_ms = total_start.elapsed().as_millis();
            diagnostics.push(format!(
                "Query compute: {} ms before response processing",
                query_compute_ms
            ));
            let response = ResponseProcessor::runtime_response(
                result,
                diagnostics,
                0,
                query_compute_ms,
                total_start,
            )?;
            #[cfg(feature = "telemetry")]
            crate::telemetry::record_runtime_response(
                &query_span,
                &command_label,
                &request.budget,
                &response.timing,
            );
            return Ok(response);
        }

        if request.command == ToolCommand::CheckReload {
            let result = match self.reload_if_changed() {
                Ok(crate::runtime::ReloadResult::Reloaded) => {
                    diagnostics.push("GRAPH_RELOADED".to_string());
                    serde_json::json!({
                        "reloaded": true,
                        "status": self.status(),
                        "diagnostics": vec!["GRAPH_RELOADED"],
                    })
                }
                Ok(crate::runtime::ReloadResult::Unchanged) => {
                    diagnostics.push("GRAPH_UNCHANGED".to_string());
                    serde_json::json!({
                        "reloaded": false,
                        "status": self.status(),
                        "diagnostics": vec!["GRAPH_UNCHANGED"],
                    })
                }
                Ok(crate::runtime::ReloadResult::ReloadFailed { error }) => {
                    diagnostics.push("GRAPH_RELOAD_FAILED".to_string());
                    serde_json::json!({
                        "reloaded": false,
                        "error": error,
                        "diagnostics": vec!["GRAPH_RELOAD_FAILED"],
                    })
                }
                Err(e) => {
                    diagnostics.push("GRAPH_RELOAD_FAILED".to_string());
                    serde_json::json!({
                        "reloaded": false,
                        "error": format!("{}", e),
                        "diagnostics": vec!["GRAPH_RELOAD_FAILED"],
                    })
                }
            };
            let query_compute_ms = total_start.elapsed().as_millis();
            diagnostics.push(format!(
                "Query compute: {} ms before response processing",
                query_compute_ms
            ));
            let response = ResponseProcessor::runtime_response(
                result,
                diagnostics,
                0,
                query_compute_ms,
                total_start,
            )?;
            #[cfg(feature = "telemetry")]
            crate::telemetry::record_runtime_response(
                &query_span,
                &command_label,
                &request.budget,
                &response.timing,
            );
            return Ok(response);
        }

        let query_start = Instant::now();
        #[cfg(feature = "telemetry")]
        let query_compute_span = tracing::info_span!(
            "metadata_checker.stage.query_compute",
            "metadata_checker.command" = %command_label,
            "metadata_checker.target" = %request.target,
        );
        #[cfg(feature = "telemetry")]
        let query_compute_guard = query_compute_span.enter();
        let mut result = match request.command {
            ToolCommand::AdviseQuery => {
                let question_kind = request.intent.as_deref().unwrap_or("auto");
                let page_scope = request.page_scope.as_deref();
                crate::answer_contract::build_advise_query_output(
                    &request.target,
                    page_scope,
                    question_kind,
                    &request.budget,
                )
            }
            ToolCommand::ExplainCondition => {
                let intent = crate::explain::TraversalIntent::parse(
                    request.intent.as_deref().unwrap_or("auto"),
                )?;
                crate::explain::build_explain_condition_output_with_intent(
                    &self.graph,
                    &request.target,
                    &request.budget,
                    intent,
                )?
            }
            ToolCommand::QueryModel => crate::query::build_query_model_output(
                &self.graph,
                &request.target,
                &request.budget,
            )?,
            ToolCommand::QueryPageLogic => {
                let project_dir = self.project_dir.as_deref();
                let availability_cache = self.read_model.as_ref().and_then(|model| {
                    model.page_logic_availability(&request.target, &request.budget)
                });
                crate::query::build_query_page_logic_output_with_availability_cache(
                    &self.graph,
                    self.read_model
                        .as_ref()
                        .map(|model| model.dense_graph.as_ref())
                        .or(self.dense_snapshot.as_deref()),
                    availability_cache,
                    &request.target,
                    project_dir,
                    &request.budget,
                )?
            }
            ToolCommand::Explain => {
                crate::explain::build_explain_output(&self.graph, &request.target)?
            }
            ToolCommand::Context => {
                let depth = request.depth.unwrap_or(1);
                crate::context::build_context_output(
                    &self.graph,
                    &request.target,
                    depth,
                    &request.budget,
                )?
            }
            ToolCommand::FindPage => serde_json::to_value(crate::query::find_nodes(
                &self.graph,
                &request.target,
                Some("page"),
                20,
            )?)?,
            ToolCommand::FindModel => serde_json::to_value(crate::query::find_nodes(
                &self.graph,
                &request.target,
                Some("model"),
                20,
            )?)?,
            ToolCommand::FindComponent => serde_json::to_value(crate::query::find_nodes(
                &self.graph,
                &request.target,
                Some("component"),
                20,
            )?)?,
            ToolCommand::QueryPage => {
                crate::query::build_query_page_output(&self.graph, &request.target)?
            }
            ToolCommand::QueryCross => {
                let (page_a, page_b) =
                    crate::tool_contract::parse_query_cross_target(&request.target)?;
                crate::query::build_query_cross_output(&self.graph, &page_a, &page_b)?
            }
            ToolCommand::QueryDataflow => {
                crate::query::build_query_dataflow_output(&self.graph, &request.target)?
            }
            ToolCommand::ReloadGraph | ToolCommand::CheckReload => {
                unreachable!("ReloadGraph and CheckReload handled before match")
            }
            ToolCommand::Status => {
                let status = self.status();
                serde_json::to_value(status)?
            }
        };
        #[cfg(feature = "telemetry")]
        drop(query_compute_guard);
        let query_compute_ms = query_start.elapsed().as_millis();

        if request.human {
            match request.command {
                ToolCommand::ExplainCondition => {
                    let human_text =
                        crate::explain::render_explain_condition_human(&result, &request.target);
                    if let Some(obj) = result.as_object_mut() {
                        obj.insert(
                            "human_summary".to_string(),
                            serde_json::Value::String(human_text),
                        );
                    }
                }
                _ => {
                    diagnostics.push("HUMAN_MODE_NOT_SUPPORTED".to_string());
                }
            }
        }

        diagnostics.push(format!(
            "Query compute: {} ms before response processing",
            query_compute_ms
        ));

        let mut response = ResponseProcessor::runtime_response(
            result,
            diagnostics,
            0,
            query_compute_ms,
            total_start,
        )?;
        response.diagnostics.push(format!(
            "Response processed: serialize {} ms, total {} ms",
            response.timing.serialize_ms, response.timing.total_ms
        ));
        #[cfg(feature = "telemetry")]
        crate::telemetry::record_runtime_response(
            &query_span,
            &command_label,
            &request.budget,
            &response.timing,
        );
        Ok(response)
    }

    /// 获取当前 graphdb 文件指纹
    pub fn current_fingerprint(&self) -> Result<GraphFingerprint> {
        let path = &self.graph_db_path;
        let (mtime, size) = std::fs::metadata(path)
            .map(|m| (m.modified().ok(), m.len()))
            .unwrap_or((None, 0));
        let prefix_hash = compute_prefix_hash(path, 4096);
        Ok(GraphFingerprint {
            path: path.clone(),
            mtime,
            size,
            content_prefix_hash: prefix_hash,
        })
    }

    /// 检查 graphdb 文件是否发生变化
    pub fn is_graph_changed(&self) -> Result<bool> {
        let current = self.current_fingerprint()?;
        let changed = current.size != self.graph_fingerprint.size
            || current.mtime != self.graph_fingerprint.mtime
            || current.content_prefix_hash != self.graph_fingerprint.content_prefix_hash;
        Ok(changed)
    }

    /// 如果 graphdb 发生变化则 reload，返回明确的结果枚举
    pub fn reload_if_changed(&mut self) -> Result<ReloadResult> {
        if self.is_graph_changed()? {
            match self.reload() {
                Ok(()) => Ok(ReloadResult::Reloaded),
                Err(e) => {
                    let err = format!("{}", e);
                    self.last_reload_error = Some(err.clone());
                    Ok(ReloadResult::ReloadFailed { error: err })
                }
            }
        } else {
            Ok(ReloadResult::Unchanged)
        }
    }

    /// 安全 reload：先加载新图，成功后再替换旧图
    pub fn reload(&mut self) -> Result<()> {
        let project_dir = self.project_dir.clone();
        match Self::load_with_project_dir_internal(
            &self.graph_db_path,
            project_dir.as_deref(),
            self.runtime_mode,
        ) {
            Ok(new_runtime) => {
                self.graph = new_runtime.graph;
                self.loaded_at = new_runtime.loaded_at;
                self.graph_file_mtime = new_runtime.graph_file_mtime;
                self.graph_file_size = new_runtime.graph_file_size;
                self.load_count = new_runtime.load_count;
                self.graph_load_ms = new_runtime.graph_load_ms;
                self.graph_fingerprint = new_runtime.graph_fingerprint;
                self.project_dir = new_runtime.project_dir;
                self.dense_snapshot = new_runtime.dense_snapshot;
                self.dense_snapshot_build_ms = new_runtime.dense_snapshot_build_ms;
                self.dense_snapshot_enabled = new_runtime.dense_snapshot_enabled;
                self.runtime_mode = new_runtime.runtime_mode;
                self.read_model = new_runtime.read_model;
                self.read_model_build_ms = new_runtime.read_model_build_ms;
                self.reload_count += 1;
                self.last_reload_error = None;
                Ok(())
            }
            Err(e) => {
                let err_msg = format!("reload failed: {}", e);
                self.last_reload_error = Some(err_msg.clone());
                Err(anyhow::anyhow!("{}", err_msg))
            }
        }
    }

    /// 获取当前 runtime 状态快照
    pub fn status(&self) -> RuntimeStatus {
        let loaded_at_secs = self
            .loaded_at
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        RuntimeStatus {
            graph_db_path: self.graph_db_path.clone(),
            loaded_at: loaded_at_secs,
            load_count: self.load_count,
            reload_count: self.reload_count,
            node_count: self.graph.graph.node_count(),
            edge_count: self.graph.graph.edge_count(),
            graph_file_mtime: self.graph_file_mtime,
            graph_file_size: self.graph_file_size,
            last_reload_error: self.last_reload_error.clone(),
        }
    }
}
