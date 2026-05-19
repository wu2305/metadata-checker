use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use crate::graph::GraphDB;
use crate::response_processor::ResponseProcessor;
pub use crate::response_processor::{RuntimeQueryResponse, RuntimeTiming};

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
}

/// Runtime 查询命令枚举
///
/// M23 最小范围只要求 ExplainCondition，后续里程碑可扩展
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuntimeQueryCommand {
    ExplainCondition,
    AdviseQuery,
}

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
        let path = graph_db_path.as_ref().to_path_buf();
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

        let project_dir = project_dir
            .map(|p| p.as_ref().to_path_buf())
            .or_else(|| path.parent().map(|p| p.to_path_buf()));

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
        })
    }

    /// 兼容旧签名：从 graph_db_path.parent() 推导 project_dir
    pub fn load(graph_db_path: impl AsRef<Path>) -> Result<Self> {
        Self::load_with_project_dir(graph_db_path, None::<&Path>)
    }

    /// 执行查询，复用内存中的 graph
    ///
    /// M23 只支持 RuntimeQueryCommand::ExplainCondition
    pub fn query(&self, request: RuntimeQueryRequest) -> Result<RuntimeQueryResponse> {
        let total_start = Instant::now();
        let mut diagnostics = Vec::new();

        let query_start = Instant::now();
        let mut result = match request.command {
            RuntimeQueryCommand::AdviseQuery => {
                let question_kind = request.intent.as_deref().unwrap_or("auto");
                let page_scope = request.page_scope.as_deref();
                crate::answer_contract::build_advise_query_output(
                    &request.target,
                    page_scope,
                    question_kind,
                    &request.budget,
                )
            }
            RuntimeQueryCommand::ExplainCondition => {
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
        };
        let query_compute_ms = query_start.elapsed().as_millis();

        if request.human {
            let human_text =
                crate::explain::render_explain_condition_human(&result, &request.target);
            if let Some(obj) = result.as_object_mut() {
                obj.insert(
                    "human_summary".to_string(),
                    serde_json::Value::String(human_text),
                );
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
        match Self::load_with_project_dir(&self.graph_db_path, project_dir.as_deref()) {
            Ok(new_runtime) => {
                self.graph = new_runtime.graph;
                self.loaded_at = new_runtime.loaded_at;
                self.graph_file_mtime = new_runtime.graph_file_mtime;
                self.graph_file_size = new_runtime.graph_file_size;
                self.load_count = new_runtime.load_count;
                self.graph_load_ms = new_runtime.graph_load_ms;
                self.graph_fingerprint = new_runtime.graph_fingerprint;
                self.project_dir = new_runtime.project_dir;
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
