pub mod builder;
pub mod echarts;
pub mod graph_model;
pub mod mermaid;
pub mod options;
pub mod sanitizer;

use crate::output::schema::AiOutput;
use crate::visualization::builder::VisualGraphBuilder;
use crate::visualization::echarts::EChartsRenderer;
use crate::visualization::mermaid::MermaidRenderer;
use crate::visualization::options::VisualGraphOptions;

/// 从 AiOutput 渲染为 Mermaid flowchart 文本
///
/// # 示例
/// ```
/// let output = AiOutput::new(OutputKind::PageQuery, json!({"target_id": "page:home"}));
/// let mermaid = render_mermaid_from_ai_output(&output, &VisualGraphOptions::default()).unwrap();
/// assert!(mermaid.starts_with("graph TD"));
/// ```
pub fn render_mermaid_from_ai_output(
    output: &AiOutput,
    options: &VisualGraphOptions,
) -> anyhow::Result<String> {
    let graph = VisualGraphBuilder::from_ai_output(output, options);
    Ok(MermaidRenderer::render(&graph))
}

/// 从 AiOutput 渲染为 ECharts option JSON
///
/// # 示例
/// ```
/// let output = AiOutput::new(OutputKind::ModelQuery, json!({"model_id": "model:users"}));
/// let option = render_echarts_from_ai_output(&output, &VisualGraphOptions::default()).unwrap();
/// assert!(option["series"][0]["data"].is_array());
/// ```
pub fn render_echarts_from_ai_output(
    output: &AiOutput,
    options: &VisualGraphOptions,
) -> anyhow::Result<serde_json::Value> {
    let graph = VisualGraphBuilder::from_ai_output(output, options);
    Ok(EChartsRenderer::render(&graph))
}
