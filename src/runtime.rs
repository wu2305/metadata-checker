use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, Instant};

use crate::graph::GraphDB;

/// Hot Graph Runtime：在同一进程内复用已加载的 GraphDB
///
/// M23 目标：把"加载图"和"执行查询"从 CLI 分支中解耦，
/// 证明同一个 runtime 连续执行多次查询时，第二次不再全量加载 graphdb。
pub struct GraphRuntime {
    /// 内存中的图数据库（全量加载）
    pub graph: GraphDB,
    /// graphdb 文件路径
    pub graph_db_path: PathBuf,
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
}

/// Runtime 查询命令枚举
///
/// M23 最小范围只要求 ExplainCondition，后续里程碑可扩展
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuntimeQueryCommand {
    ExplainCondition,
}

/// Runtime 查询请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeQueryRequest {
    pub command: RuntimeQueryCommand,
    pub target: String,
    pub budget: String,
    pub human: bool,
}

/// Runtime 查询响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeQueryResponse {
    pub result: serde_json::Value,
    pub timing: RuntimeTiming,
    pub diagnostics: Vec<String>,
}

/// 阶段耗时统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTiming {
    /// graph 加载耗时（热查询时为 0）
    pub graph_load_ms: u128,
    /// 查询计算耗时
    pub query_compute_ms: u128,
    /// 序列化耗时
    pub serialize_ms: u128,
    /// 总耗时
    pub total_ms: u128,
}

impl GraphRuntime {
    /// 加载 graphdb 并构建 Runtime
    ///
    /// 记录文件 metadata，初始化 load_count = 1
    pub fn load(graph_db_path: impl AsRef<Path>) -> Result<Self> {
        let path = graph_db_path.as_ref().to_path_buf();
        let start = Instant::now();
        let graph = GraphDB::open_or_diagnostic(&path)
            .map_err(|e| anyhow::anyhow!("GraphDB open failed: {}", serde_json::to_string(&e).unwrap_or_default()))?;
        let graph_load_ms = start.elapsed().as_millis();

        let (graph_file_mtime, graph_file_size) = std::fs::metadata(&path)
            .map(|m| (m.modified().ok(), m.len()))
            .unwrap_or((None, 0));

        let mut diagnostics = Vec::new();
        diagnostics.push(format!("Graph loaded in {} ms", graph_load_ms));

        Ok(GraphRuntime {
            graph,
            graph_db_path: path,
            loaded_at: SystemTime::now(),
            graph_file_mtime,
            graph_file_size,
            load_count: 1,
            graph_load_ms,
        })
    }

    /// 执行查询，复用内存中的 graph
    ///
    /// M23 只支持 RuntimeQueryCommand::ExplainCondition
    pub fn query(&self, request: RuntimeQueryRequest) -> Result<RuntimeQueryResponse> {
        let total_start = Instant::now();
        let mut diagnostics = Vec::new();

        let query_start = Instant::now();
        let mut result = match request.command {
            RuntimeQueryCommand::ExplainCondition => {
                crate::explain::build_explain_condition_output(
                    &self.graph,
                    &request.target,
                    &request.budget,
                )?
            }
        };
        let query_compute_ms = query_start.elapsed().as_millis();

        if request.human {
            let human_text = crate::explain::render_explain_condition_human(&result, &request.target);
            if let Some(obj) = result.as_object_mut() {
                obj.insert("human_summary".to_string(), serde_json::Value::String(human_text));
            }
        }

        let serialize_start = Instant::now();
        // 预序列化以统计耗时，但不改变返回的 result
        let _ = serde_json::to_vec(&result)?;
        let serialize_ms = serialize_start.elapsed().as_millis();

        let total_ms = total_start.elapsed().as_millis();

        diagnostics.push(format!(
            "Query compute: {} ms, serialize: {} ms, total: {} ms",
            query_compute_ms, serialize_ms, total_ms
        ));

        Ok(RuntimeQueryResponse {
            result,
            timing: RuntimeTiming {
                graph_load_ms: 0,
                query_compute_ms,
                serialize_ms,
                total_ms,
            },
            diagnostics,
        })
    }
}
