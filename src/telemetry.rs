use anyhow::Result;

/// 观测输出模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryMode {
    /// 不启用观测输出。
    Off,
    /// 将 tracing 事件以 JSON 写入 stderr。
    Json,
    /// 通过 OTLP 导出到远程观测平台。
    Otlp,
}

/// 观测初始化配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryConfig {
    /// 输出模式。
    pub mode: TelemetryMode,
    /// OTLP traces endpoint，例如 `http://localhost:4318/v1/traces`。
    pub otlp_endpoint: Option<String>,
    /// OTLP metrics endpoint，例如 `http://localhost:4318/v1/metrics`。
    pub otlp_metrics_endpoint: Option<String>,
}

/// 观测运行期 guard。
///
/// OTLP 模式下持有 provider，进程退出时 flush。
#[derive(Debug)]
pub struct TelemetryGuard {
    #[cfg(feature = "telemetry-otlp")]
    tracer_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    #[cfg(feature = "telemetry-otlp")]
    meter_provider: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
}

impl TelemetryGuard {
    /// 构造空 guard。
    pub fn noop() -> Self {
        Self {
            #[cfg(feature = "telemetry-otlp")]
            tracer_provider: None,
            #[cfg(feature = "telemetry-otlp")]
            meter_provider: None,
        }
    }
}

#[cfg(feature = "telemetry-otlp")]
impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.tracer_provider.take() {
            let _ = provider.shutdown();
        }
        if let Some(provider) = self.meter_provider.take() {
            let _ = provider.shutdown();
        }
    }
}

/// 初始化观测输出。
pub fn init(config: TelemetryConfig) -> Result<TelemetryGuard> {
    match config.mode {
        TelemetryMode::Off => Ok(TelemetryGuard::noop()),
        TelemetryMode::Json => init_json(),
        TelemetryMode::Otlp => init_otlp(
            config.otlp_endpoint.as_deref(),
            config.otlp_metrics_endpoint.as_deref(),
        ),
    }
}

#[cfg(feature = "telemetry")]
fn init_json() -> Result<TelemetryGuard> {
    tracing_subscriber::fmt()
        .json()
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init()
        .map_err(|e| anyhow::anyhow!("failed to initialize JSON telemetry: {}", e))?;
    Ok(TelemetryGuard::noop())
}

#[cfg(not(feature = "telemetry"))]
fn init_json() -> Result<TelemetryGuard> {
    anyhow::bail!("--trace json requires building with the telemetry feature")
}

#[cfg(feature = "telemetry-otlp")]
fn init_otlp(
    traces_endpoint: Option<&str>,
    metrics_endpoint: Option<&str>,
) -> Result<TelemetryGuard> {
    use opentelemetry::global;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::{Protocol, WithExportConfig};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let mut exporter_builder = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary);
    if let Some(endpoint) = traces_endpoint {
        exporter_builder = exporter_builder.with_endpoint(endpoint.to_string());
    }

    let exporter = exporter_builder
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build OTLP exporter: {}", e))?;
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_simple_exporter(exporter)
        .build();
    let tracer = provider.tracer("metadata-checker");
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let metrics_endpoint = resolve_metrics_endpoint(traces_endpoint, metrics_endpoint);
    let mut metric_exporter_builder = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary);
    if let Some(endpoint) = metrics_endpoint {
        metric_exporter_builder = metric_exporter_builder.with_endpoint(endpoint);
    }
    let metric_exporter = metric_exporter_builder
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build OTLP metric exporter: {}", e))?;
    let meter_provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_periodic_exporter(metric_exporter)
        .build();
    global::set_meter_provider(meter_provider.clone());

    tracing_subscriber::registry()
        .with(layer)
        .try_init()
        .map_err(|e| anyhow::anyhow!("failed to initialize OTLP telemetry: {}", e))?;

    Ok(TelemetryGuard {
        tracer_provider: Some(provider),
        meter_provider: Some(meter_provider),
    })
}

#[cfg(not(feature = "telemetry-otlp"))]
fn init_otlp(
    _traces_endpoint: Option<&str>,
    _metrics_endpoint: Option<&str>,
) -> Result<TelemetryGuard> {
    anyhow::bail!("--trace otlp requires building with the telemetry-otlp feature")
}

/// 根据 traces endpoint 推导 metrics endpoint。
#[cfg(feature = "telemetry-otlp")]
fn resolve_metrics_endpoint(
    traces_endpoint: Option<&str>,
    metrics_endpoint: Option<&str>,
) -> Option<String> {
    if let Some(endpoint) = metrics_endpoint {
        return Some(endpoint.to_string());
    }
    traces_endpoint
        .and_then(|endpoint| endpoint.strip_suffix("/v1/traces"))
        .map(|base| format!("{base}/v1/metrics"))
}

/// 创建 graph 加载 span。
#[cfg(feature = "telemetry")]
pub fn graph_load_span(graph_db_path: &str) -> tracing::Span {
    tracing::info_span!(
        "metadata_checker.stage.graph_load",
        "metadata_checker.graph.path" = %graph_db_path,
        "metadata_checker.graph.nodes" = tracing::field::Empty,
        "metadata_checker.graph.edges" = tracing::field::Empty,
        "metadata_checker.timing.graph_load_ms" = tracing::field::Empty,
        "metadata_checker.status" = tracing::field::Empty,
    )
}

/// 创建 runtime 查询能力 span。
#[cfg(feature = "telemetry")]
pub fn runtime_query_span(
    command: &str,
    target: &str,
    budget: &str,
    intent: Option<&str>,
    human: bool,
    graph_nodes: usize,
    graph_edges: usize,
) -> tracing::Span {
    tracing::info_span!(
        "metadata_checker.capability.runtime_query",
        "metadata_checker.command" = %command,
        "metadata_checker.target" = %target,
        "metadata_checker.budget" = %budget,
        "metadata_checker.intent" = intent.unwrap_or(""),
        "metadata_checker.human" = human,
        "metadata_checker.graph.nodes" = graph_nodes,
        "metadata_checker.graph.edges" = graph_edges,
        "metadata_checker.status" = tracing::field::Empty,
        "metadata_checker.timing.query_compute_ms" = tracing::field::Empty,
        "metadata_checker.timing.serialize_ms" = tracing::field::Empty,
        "metadata_checker.timing.total_ms" = tracing::field::Empty,
        "metadata_checker.output.bytes" = tracing::field::Empty,
    )
}

/// 记录 graph 加载结果。
#[cfg(feature = "telemetry")]
pub fn record_graph_load(span: &tracing::Span, graph_load_ms: u128, nodes: usize, edges: usize) {
    span.record("metadata_checker.status", "ok");
    span.record(
        "metadata_checker.timing.graph_load_ms",
        graph_load_ms as u64,
    );
    span.record("metadata_checker.graph.nodes", nodes as u64);
    span.record("metadata_checker.graph.edges", edges as u64);
}

/// 记录 runtime 查询结果。
#[cfg(feature = "telemetry")]
pub fn record_runtime_response(
    span: &tracing::Span,
    command: &str,
    budget: &str,
    timing: &crate::response_processor::RuntimeTiming,
) {
    span.record("metadata_checker.status", "ok");
    span.record(
        "metadata_checker.timing.query_compute_ms",
        timing.query_compute_ms as u64,
    );
    span.record(
        "metadata_checker.timing.serialize_ms",
        timing.serialize_ms as u64,
    );
    span.record("metadata_checker.timing.total_ms", timing.total_ms as u64);
    span.record("metadata_checker.output.bytes", timing.output_size_bytes);
    record_runtime_metrics(command, budget, timing);
}

/// 记录 runtime 能力指标。
///
/// 指标只用于观测与远程趋势分析；可重复性能测量以 Criterion / runner 为准。
#[cfg(feature = "telemetry-otlp")]
fn record_runtime_metrics(
    command: &str,
    budget: &str,
    timing: &crate::response_processor::RuntimeTiming,
) {
    use opentelemetry::KeyValue;

    let instruments = runtime_metric_instruments();
    let attributes = [
        KeyValue::new("command", command.to_string()),
        KeyValue::new("budget", budget.to_string()),
    ];

    instruments
        .duration_ms
        .record(timing.total_ms as u64, &attributes);
    instruments
        .output_bytes
        .record(timing.output_size_bytes, &attributes);
    instruments.calls.add(1, &attributes);
}

/// Runtime 能力指标 instruments。
#[cfg(feature = "telemetry-otlp")]
struct RuntimeMetricInstruments {
    duration_ms: opentelemetry::metrics::Histogram<u64>,
    output_bytes: opentelemetry::metrics::Histogram<u64>,
    calls: opentelemetry::metrics::Counter<u64>,
}

/// 获取可复用的 runtime 指标 instruments。
#[cfg(feature = "telemetry-otlp")]
fn runtime_metric_instruments() -> &'static RuntimeMetricInstruments {
    use opentelemetry::global;
    use std::sync::OnceLock;

    static INSTRUMENTS: OnceLock<RuntimeMetricInstruments> = OnceLock::new();
    INSTRUMENTS.get_or_init(|| {
        let meter = global::meter("metadata-checker");
        RuntimeMetricInstruments {
            duration_ms: meter
                .u64_histogram("metadata_checker.capability.duration_ms")
                .with_unit("ms")
                .build(),
            output_bytes: meter
                .u64_histogram("metadata_checker.output.bytes")
                .with_unit("By")
                .build(),
            calls: meter
                .u64_counter("metadata_checker.capability.calls")
                .build(),
        }
    })
}

#[cfg(all(feature = "telemetry", not(feature = "telemetry-otlp")))]
fn record_runtime_metrics(
    _command: &str,
    _budget: &str,
    _timing: &crate::response_processor::RuntimeTiming,
) {
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_off_initializes_noop_guard() {
        let guard = init(TelemetryConfig {
            mode: TelemetryMode::Off,
            otlp_endpoint: None,
            otlp_metrics_endpoint: None,
        })
        .expect("off mode should not require telemetry features");
        let _ = guard;
    }

    #[test]
    fn test_telemetry_config_keeps_otlp_endpoint() {
        let config = TelemetryConfig {
            mode: TelemetryMode::Otlp,
            otlp_endpoint: Some("http://localhost:4318/v1/traces".to_string()),
            otlp_metrics_endpoint: Some("http://localhost:4318/v1/metrics".to_string()),
        };
        assert_eq!(config.mode, TelemetryMode::Otlp);
        assert_eq!(
            config.otlp_endpoint.as_deref(),
            Some("http://localhost:4318/v1/traces")
        );
        assert_eq!(
            config.otlp_metrics_endpoint.as_deref(),
            Some("http://localhost:4318/v1/metrics")
        );
    }

    #[cfg(feature = "telemetry-otlp")]
    #[test]
    fn test_metrics_endpoint_is_derived_from_traces_endpoint() {
        let endpoint = resolve_metrics_endpoint(Some("http://localhost:4318/v1/traces"), None);
        assert_eq!(
            endpoint.as_deref(),
            Some("http://localhost:4318/v1/metrics")
        );
    }
}
