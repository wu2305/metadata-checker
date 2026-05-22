use crate::graph_store::{GraphNeighbors, GraphReadStore};
use serde::{Deserialize, Serialize};

/// 项目级图数据类型模块
///
/// M40.1：从原 graph.rs 拆出纯数据结构和通用 helper，不依赖 redb。
/// redb 持久化实现迁移到 graph_redb.rs（cli-local only）。

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
/// 图节点类型
pub enum NodeType {
    Page,
    Component,
    Model,
    Field,
    Action,
    /// 条件/表达式节点（visibleCondition、action.conditionExp 等）
    Condition,
}

/// 图中边类型（关系语义）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EdgeType {
    Reads,
    Writes,
    Triggers,
    Contains,
    DataflowInput,
    ActionWrites,
    EmbedsPage,
    OpensPage,
    PassesParam,
    SetsParam,
    OutputsTo,
    DataflowInternal,
    DataflowOutput,
    /// 局部模型字段到物理表字段的别名映射
    FieldAlias,
    /// Action 对字段的直接写入
    FieldWrite,
    ActionReads,
    ActionNavigates,
    ActionSetsParam,
    ActionControlsComponent,
    ActionValidates,
    ActionLoadsData,
    /// 条件/表达式对上游符号的依赖（组件值、参数、用户属性、系统变量）
    DependsOn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 图节点
pub struct Node {
    pub id: String,
    pub node_type: NodeType,
    pub path: String,
    pub name: String,
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 图边
pub struct Edge {
    pub from: String,
    pub to: String,
    pub edge_type: EdgeType,
    pub field_path: Option<String>,
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 文件状态（用于增量更新）
pub struct FileState {
    pub file_path: String,
    pub file_hash: String,
    pub mtime: u64,
    pub size: u64,
    pub node_ids: Vec<String>,
}

/// 计算 Levenshtein 编辑距离
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev = vec![0usize; b_len + 1];
    let mut curr = vec![0usize; b_len + 1];

    for j in 0..=b_len {
        prev[j] = j;
    }

    for i in 1..=a_len {
        curr[0] = i;
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[b_len]
}

/// 查找读取指定模型的所有节点（独立函数，基于 GraphReadStore）
pub fn find_readers(graph: &dyn GraphReadStore, model_id: &str) -> anyhow::Result<Vec<(Node, Edge)>> {
    Ok(graph
        .get_node_edges(model_id)?
        .map(|n| {
            n.incoming
                .into_iter()
                .filter(|ev| matches!(ev.edge.edge_type, EdgeType::Reads))
                .map(|ev| (ev.node, ev.edge))
                .collect()
        })
        .unwrap_or_default())
}

/// 查找写入指定模型的所有节点（独立函数，基于 GraphReadStore）
pub fn find_writers(graph: &dyn GraphReadStore, model_id: &str) -> anyhow::Result<Vec<(Node, Edge)>> {
    Ok(graph
        .get_node_edges(model_id)?
        .map(|n| {
            n.incoming
                .into_iter()
                .filter(|ev| matches!(ev.edge.edge_type, EdgeType::Writes | EdgeType::ActionWrites))
                .map(|ev| (ev.node, ev.edge))
                .collect()
        })
        .unwrap_or_default())
}

/// 查询 DataFlow 的输入依赖（ outgoing DataflowInput 边）（独立函数，基于 GraphReadStore）
pub fn find_dataflow_inputs(
    graph: &dyn GraphReadStore,
    model_id: &str,
) -> anyhow::Result<Vec<(Node, Edge)>> {
    Ok(graph
        .get_node_edges(model_id)?
        .map(|n| {
            n.outgoing
                .into_iter()
                .filter(|ev| matches!(ev.edge.edge_type, EdgeType::DataflowInput))
                .map(|ev| (ev.node, ev.edge))
                .collect()
        })
        .unwrap_or_default())
}

/// 查询 DataFlow 的输出目标（ outgoing OutputsTo 边）（独立函数，基于 GraphReadStore）
pub fn find_dataflow_outputs(
    graph: &dyn GraphReadStore,
    model_id: &str,
) -> anyhow::Result<Vec<(Node, Edge)>> {
    Ok(graph
        .get_node_edges(model_id)?
        .map(|n| {
            n.outgoing
                .into_iter()
                .filter(|ev| matches!(ev.edge.edge_type, EdgeType::OutputsTo))
                .map(|ev| (ev.node, ev.edge))
                .collect()
        })
        .unwrap_or_default())
}

/// 查询物理表的生产者（ incoming OutputsTo 边）（独立函数，基于 GraphReadStore）
pub fn find_produced_by(graph: &dyn GraphReadStore, model_id: &str) -> anyhow::Result<Vec<(Node, Edge)>> {
    Ok(graph
        .get_node_edges(model_id)?
        .map(|n| {
            n.incoming
                .into_iter()
                .filter(|ev| matches!(ev.edge.edge_type, EdgeType::OutputsTo))
                .map(|ev| (ev.node, ev.edge))
                .collect()
        })
        .unwrap_or_default())
}

/// 查询哪些 DataFlow 消费了该输入表（ incoming DataflowInput 边）（独立函数，基于 GraphReadStore）
pub fn find_consumed_by_dataflows(
    graph: &dyn GraphReadStore,
    model_id: &str,
) -> anyhow::Result<Vec<(Node, Edge)>> {
    Ok(graph
        .get_node_edges(model_id)?
        .map(|n| {
            n.incoming
                .into_iter()
                .filter(|ev| matches!(ev.edge.edge_type, EdgeType::DataflowInput))
                .map(|ev| (ev.node, ev.edge))
                .collect()
        })
        .unwrap_or_default())
}

/// 把 GraphReadStore::get_node_edges 结果转换为旧风格的 owned tuple 列表
pub fn get_node_edges_as_tuples(
    graph: &dyn GraphReadStore,
    node_id: &str,
) -> anyhow::Result<(Vec<(Node, Edge)>, Vec<(Node, Edge)>)> {
    let neighbors = graph
        .get_node_edges(node_id)?
        .unwrap_or_else(|| GraphNeighbors {
            outgoing: Vec::new(),
            incoming: Vec::new(),
        });
    let outgoing = neighbors
        .outgoing
        .into_iter()
        .map(|v| (v.node, v.edge))
        .collect();
    let incoming = neighbors
        .incoming
        .into_iter()
        .map(|v| (v.node, v.edge))
        .collect();
    Ok((outgoing, incoming))
}

#[cfg(feature = "cli-local")]
pub use crate::graph_redb::{GraphDB, set_graph_lock_timeout_ms};
