use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Runtime 查询响应。
///
/// CLI、stdio 与未来 MCP adapter 都应围绕这个统一对象做协议包装，
/// 不在各自分支里重复统计 timing、序列化大小和 diagnostics。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeQueryResponse {
    pub result: serde_json::Value,
    pub timing: RuntimeTiming,
    pub diagnostics: Vec<String>,
}

/// 阶段耗时统计。
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
    /// 响应 JSON 字节数
    pub output_size_bytes: u64,
}

/// 统一响应处理器。
pub struct ResponseProcessor;

impl ResponseProcessor {
    /// 构建 Runtime 查询响应，并统一统计序列化大小。
    pub fn runtime_response(
        result: serde_json::Value,
        diagnostics: Vec<String>,
        graph_load_ms: u128,
        query_compute_ms: u128,
        total_start: Instant,
    ) -> Result<RuntimeQueryResponse> {
        let serialize_start = Instant::now();
        let output_size_bytes = serde_json::to_vec(&result)?.len() as u64;
        let serialize_ms = serialize_start.elapsed().as_millis();
        let total_ms = total_start.elapsed().as_millis();

        Ok(RuntimeQueryResponse {
            result,
            timing: RuntimeTiming {
                graph_load_ms,
                query_compute_ms,
                serialize_ms,
                total_ms,
                output_size_bytes,
            },
            diagnostics,
        })
    }

    /// 构造空 timing，用于非查询响应和错误响应。
    pub fn zero_timing() -> RuntimeTiming {
        RuntimeTiming {
            graph_load_ms: 0,
            query_compute_ms: 0,
            serialize_ms: 0,
            total_ms: 0,
            output_size_bytes: 0,
        }
    }

    /// 构造只有查询耗时的 timing，用于 adapter 直接执行的轻量命令。
    pub fn query_timing(query_compute_ms: u128) -> RuntimeTiming {
        RuntimeTiming {
            graph_load_ms: 0,
            query_compute_ms,
            serialize_ms: 0,
            total_ms: query_compute_ms,
            output_size_bytes: 0,
        }
    }

    /// 根据已经序列化出的 JSON 更新 timing。
    pub fn update_timing_after_serialization(
        timing: &mut RuntimeTiming,
        output_size_bytes: u64,
        serialize_ms: u128,
    ) -> bool {
        let total_ms = timing.graph_load_ms + timing.query_compute_ms + serialize_ms;
        let changed = timing.output_size_bytes != output_size_bytes
            || timing.serialize_ms != serialize_ms
            || timing.total_ms != total_ms;
        timing.output_size_bytes = output_size_bytes;
        timing.serialize_ms = serialize_ms;
        timing.total_ms = total_ms;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_response_records_timing_and_size() {
        let result = serde_json::json!({"ok": true});
        let response = ResponseProcessor::runtime_response(
            result,
            vec!["done".to_string()],
            7,
            3,
            Instant::now(),
        )
        .expect("response should build");

        assert_eq!(response.diagnostics, vec!["done".to_string()]);
        assert_eq!(response.timing.graph_load_ms, 7);
        assert_eq!(response.timing.query_compute_ms, 3);
        assert!(
            response.timing.output_size_bytes > 0,
            "serialized response size must be recorded"
        );
    }

    #[test]
    fn test_update_timing_after_serialization_reports_changes() {
        let mut timing = ResponseProcessor::query_timing(5);
        let changed = ResponseProcessor::update_timing_after_serialization(&mut timing, 120, 2);

        assert!(changed, "first update should change timing fields");
        assert_eq!(timing.output_size_bytes, 120);
        assert_eq!(timing.serialize_ms, 2);
        assert_eq!(timing.total_ms, 7);

        let changed_again =
            ResponseProcessor::update_timing_after_serialization(&mut timing, 120, 2);
        assert!(
            !changed_again,
            "same serialized size and timing should be stable"
        );
    }
}
