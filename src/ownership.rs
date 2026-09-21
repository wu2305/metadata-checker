//! M59-2 来源贡献与项目绑定契约。
//!
//! 本模块只定义来源账本的稳定数据结构和输入校验，不把贡献信息伪装成
//! Node/Edge 的展示 metadata。图的派生与持久化由 scanner/indexer 和 store
//! 在后续 M59-2 包中消费。

use crate::graph::{Edge, Node};
use crate::graph_store::{GraphStore, edge_dedup_key};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// 项目绑定准备格式版本，不代表来源账本已实现。
///
/// 它与 `fact_schema_version=2` 分离：后者只描述完整事实键，不能表示
/// 节点定义、引用和来源撤销已经可用。
pub const PROJECT_BINDING_SCHEMA_VERSION: u32 = 1;
/// 已启用来源账本的独立 schema 版本。
///
/// 该 marker 只能由真实账本提交写入；准备阶段的绑定 marker 不得冒充它。
pub const OWNERSHIP_SCHEMA_VERSION: u32 = 2;
/// 当前来源账本快照的序列化版本。
pub const OWNERSHIP_LEDGER_VERSION: u32 = 1;

/// 项目绑定的稳定逻辑标识。
///
/// 该值必须由项目/store 边界提供，不能直接使用机器相关的绝对目录路径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct ProjectBinding(String);

impl TryFrom<String> for ProjectBinding {
    type Error = anyhow::Error;

    /// 反序列化沿用构造校验，不能从 JSON 注入非法绑定。
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl ProjectBinding {
    /// 校验并创建项目绑定。
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() {
            bail!("project binding must not be empty");
        }
        if value.contains('\0') {
            bail!("project binding must not contain NUL");
        }
        if value.starts_with('/') || value.starts_with('\\') {
            bail!("project binding must not be an absolute path");
        }
        if value.as_bytes().get(1) == Some(&b':') {
            bail!("project binding must not be a drive path");
        }
        Ok(Self(value))
    }

    /// 返回持久化用的绑定值。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 来源贡献的种类。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContributionKind {
    /// 当前文件对实体的完整定义。
    Definition,
    /// 当前文件对目标实体的引用，引用仍可独立保留占位节点。
    Reference,
    /// 当前文件产生的一条完整关系事实。
    Edge,
}

/// 一个来源文件贡献的实体定义或引用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityContribution {
    pub origin_file: String,
    pub node: Node,
    pub kind: ContributionKind,
}

/// 一个来源文件贡献的边事实。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeContribution {
    pub origin_file: String,
    pub edge: Edge,
}

/// 单文件可撤销贡献账本。
///
/// `entities` 和 `edges` 都按来源文件完整保存。替换文件时只能撤销该
/// 文件的记录，不能根据端点删除其它来源的关系。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileContributionLedger {
    pub origin_file: String,
    pub revision: String,
    pub entities: Vec<EntityContribution>,
    pub edges: Vec<EdgeContribution>,
}

impl FileContributionLedger {
    /// 创建一个带稳定来源路径的空账本。
    pub fn new(origin_file: impl Into<String>, revision: impl Into<String>) -> Self {
        Self {
            origin_file: origin_file.into(),
            revision: revision.into(),
            entities: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// 按实体 ID 聚合贡献，供派生图计算使用。
    pub fn entity_contributions(&self) -> BTreeMap<String, Vec<&EntityContribution>> {
        let mut grouped = BTreeMap::new();
        for contribution in &self.entities {
            grouped
                .entry(contribution.node.id.clone())
                .or_insert_with(Vec::new)
                .push(contribution);
        }
        grouped
    }
}

/// 持久化来源账本的顶层快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnershipLedgerSnapshot {
    pub schema_version: u32,
    pub project_binding: String,
    pub files: BTreeMap<String, FileContributionLedger>,
}

impl OwnershipLedgerSnapshot {
    /// 创建空的、已绑定项目的来源账本。
    pub fn new(binding: &ProjectBinding) -> Self {
        Self {
            schema_version: OWNERSHIP_LEDGER_VERSION,
            project_binding: binding.as_str().to_string(),
            files: BTreeMap::new(),
        }
    }

    /// 校验快照版本和项目归属。
    pub fn validate(&self, binding: &ProjectBinding) -> Result<()> {
        anyhow::ensure!(
            self.schema_version == OWNERSHIP_LEDGER_VERSION,
            "GRAPH_OWNERSHIP_SCHEMA_STALE: unsupported ownership ledger version {}",
            self.schema_version
        );
        anyhow::ensure!(
            self.project_binding == binding.as_str(),
            "GRAPH_PROJECT_BINDING_MISMATCH: ownership ledger belongs to another project"
        );
        for (path, ledger) in &self.files {
            anyhow::ensure!(
                path == &ledger.origin_file,
                "GRAPH_OWNERSHIP_SCHEMA_STALE: ledger key does not match origin_file"
            );
        }
        Ok(())
    }
}

/// 同一节点 id 存在多个不兼容 Definition 时的冲突记录（spec：必须报告，不得静默择一）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OwnershipConflict {
    /// 冲突的节点 id。
    pub node_id: String,
    /// 提供该 id Definition 的来源文件（去重后按字典序）。
    pub definition_origins: Vec<String>,
}

/// 从账本聚合出同 id 多 Definition 的来源冲突。
///
/// 与重建的择一规则共用“按 id 分组、看 Definition 个数”的语义，保证报告与
/// 实际择一行为不会分叉。
pub fn ledger_definition_conflicts(
    ledgers: &BTreeMap<String, FileContributionLedger>,
) -> Vec<OwnershipConflict> {
    let mut definitions: HashMap<&str, Vec<&EntityContribution>> = HashMap::new();
    for ledger in ledgers.values() {
        for contribution in &ledger.entities {
            if contribution.kind == ContributionKind::Definition {
                definitions
                    .entry(contribution.node.id.as_str())
                    .or_default()
                    .push(contribution);
            }
        }
    }
    let mut conflicts: Vec<OwnershipConflict> = definitions
        .into_iter()
        .filter(|(_, contributions)| contributions.len() > 1)
        .map(|(node_id, contributions)| {
            let mut origins: Vec<String> = contributions
                .iter()
                .map(|contribution| contribution.origin_file.clone())
                .collect();
            origins.sort();
            origins.dedup();
            OwnershipConflict {
                node_id: node_id.to_string(),
                definition_origins: origins,
            }
        })
        .collect();
    conflicts.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    conflicts
}

/// 将各来源贡献确定性地派生为当前图。
///
/// 先收集定义、引用和完整事实，再一次性 upsert/add，避免扫描顺序决定
/// 共享目标的最终 metadata；定义消失但引用仍在时，保留 PhysicalTable 占位。
/// 同一 id 出现多个 Definition 时按确定性规则择一，但必须通过返回值报告冲突。
pub fn rebuild_graph_from_ledgers(
    graph: &mut dyn GraphStore,
    ledgers: &BTreeMap<String, FileContributionLedger>,
) -> Result<Vec<OwnershipConflict>> {
    let mut entities: HashMap<String, Vec<&EntityContribution>> = HashMap::new();
    let mut edges: HashMap<crate::graph_store::EdgeFactKey, &EdgeContribution> = HashMap::new();
    for ledger in ledgers.values() {
        for contribution in &ledger.entities {
            entities
                .entry(contribution.node.id.clone())
                .or_default()
                .push(contribution);
        }
        for contribution in &ledger.edges {
            edges
                .entry(edge_dedup_key(&contribution.edge))
                .or_insert(contribution);
        }
    }

    let current_ids: Vec<String> = graph.iter_nodes()?.map(|node| node.id).collect();
    graph.remove_nodes_by_ids(&current_ids)?;

    let mut desired_nodes: Vec<Node> = entities
        .into_iter()
        .map(|(_, contributions)| {
            let mut definitions: Vec<&EntityContribution> = contributions
                .iter()
                .copied()
                .filter(|contribution| contribution.kind == ContributionKind::Definition)
                .collect();
            definitions.sort_by(|left, right| {
                left.origin_file.cmp(&right.origin_file).then(
                    serde_json::to_string(&left.node)
                        .unwrap_or_default()
                        .cmp(&serde_json::to_string(&right.node).unwrap_or_default()),
                )
            });
            let node = definitions
                .first()
                .map(|contribution| contribution.node.clone())
                .or_else(|| {
                    contributions
                        .iter()
                        .find(|contribution| contribution.kind == ContributionKind::Reference)
                        .map(|contribution| contribution.node.clone())
                })
                .expect("entity contribution must contain a node");
            let has_definition = !definitions.is_empty();
            if has_definition {
                return node;
            }
            if node.node_type == crate::graph::NodeType::Model {
                let mut placeholder = node;
                if let Some(meta) = placeholder
                    .meta
                    .as_mut()
                    .and_then(|value| value.as_object_mut())
                {
                    meta.insert("modelType".to_string(), serde_json::json!("PhysicalTable"));
                } else {
                    placeholder.meta = Some(serde_json::json!({"modelType": "PhysicalTable"}));
                }
                return placeholder;
            }
            node
        })
        .collect();
    desired_nodes.sort_by(|left, right| left.id.cmp(&right.id));
    for node in desired_nodes {
        graph.upsert_node(node)?;
    }

    let mut desired_edges: Vec<Edge> = edges
        .into_values()
        .map(|contribution| contribution.edge.clone())
        .collect();
    desired_edges.sort_by(|left, right| {
        serde_json::to_string(left)
            .unwrap_or_default()
            .cmp(&serde_json::to_string(right).unwrap_or_default())
    });
    for edge in desired_edges {
        graph.add_edge(edge)?;
    }
    Ok(ledger_definition_conflicts(ledgers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeType, NodeType};

    fn node(id: &str) -> Node {
        Node {
            id: id.to_string(),
            node_type: NodeType::Page,
            path: "app/a.spg".to_string(),
            name: "a".to_string(),
            meta: None,
            origin_file: None,
        }
    }

    #[test]
    fn project_binding_rejects_machine_paths() {
        assert!(ProjectBinding::new("/tmp/project").is_err());
        assert!(ProjectBinding::new("C:\\project").is_err());
        assert!(ProjectBinding::new("project\0id").is_err());
    }

    #[test]
    fn ledger_keeps_definition_reference_and_duplicate_edges() {
        let mut ledger = FileContributionLedger::new("app/a.spg", "rev-1");
        ledger.entities.push(EntityContribution {
            origin_file: "app/a.spg".to_string(),
            node: node("page:app/a.spg"),
            kind: ContributionKind::Definition,
        });
        ledger.entities.push(EntityContribution {
            origin_file: "app/a.spg".to_string(),
            node: node("page:app/ghost.spg"),
            kind: ContributionKind::Reference,
        });
        let edge = Edge {
            from: "page:app/a.spg".to_string(),
            to: "page:app/ghost.spg".to_string(),
            edge_type: EdgeType::EmbedsPage,
            field_path: Some("app/ghost.spg".to_string()),
            meta: None,
            origin_file: None,
        };
        ledger.edges.push(EdgeContribution {
            origin_file: "app/a.spg".to_string(),
            edge: edge.clone(),
        });
        ledger.edges.push(EdgeContribution {
            origin_file: "app/a.spg".to_string(),
            edge,
        });

        assert_eq!(ledger.entity_contributions().len(), 2);
        assert_eq!(ledger.edges.len(), 2);
        assert_eq!(ledger.origin_file, "app/a.spg");
    }

    fn definition_ledger(origin: &str, id: &str) -> FileContributionLedger {
        let mut ledger = FileContributionLedger::new(origin, "rev-1");
        let mut owned = node(id);
        owned.path = origin.to_string();
        owned.origin_file = Some(origin.to_string());
        ledger.entities.push(EntityContribution {
            origin_file: origin.to_string(),
            node: owned,
            kind: ContributionKind::Definition,
        });
        ledger
    }

    fn reference_ledger(origin: &str, id: &str) -> FileContributionLedger {
        let mut ledger = FileContributionLedger::new(origin, "rev-1");
        let mut shared = node(id);
        shared.path = "tables/shared.tbl".to_string();
        shared.origin_file = Some(origin.to_string());
        ledger.entities.push(EntityContribution {
            origin_file: origin.to_string(),
            node: shared,
            kind: ContributionKind::Reference,
        });
        ledger
    }

    /// 重建必须确定：同一输入重复重建（HashMap 迭代序随机）产出完全一致。
    #[test]
    fn rebuild_is_deterministic_across_runs() {
        let mut ledgers = BTreeMap::new();
        ledgers.insert(
            "app/a.spg".to_string(),
            definition_ledger("app/a.spg", "page:app/a.spg"),
        );
        ledgers.insert(
            "tables/shared.tbl".to_string(),
            reference_ledger("tables/shared.tbl", "model:shared"),
        );

        let mut first = crate::memory_graph_store::MemoryGraphStore::new();
        let mut second = crate::memory_graph_store::MemoryGraphStore::new();
        rebuild_graph_from_ledgers(&mut first, &ledgers).unwrap();
        rebuild_graph_from_ledgers(&mut second, &ledgers).unwrap();

        let first_nodes: Vec<String> = first
            .iter_nodes()
            .unwrap()
            .map(|n| serde_json::to_string(&n).unwrap())
            .collect();
        let second_nodes: Vec<String> = second
            .iter_nodes()
            .unwrap()
            .map(|n| serde_json::to_string(&n).unwrap())
            .collect();
        assert_eq!(first_nodes, second_nodes);
    }

    /// 同 id 多个 Definition 必须报告冲突（spec），且择一结果确定。
    #[test]
    fn rebuild_reports_conflicting_definitions() {
        let mut ledgers = BTreeMap::new();
        ledgers.insert(
            "tables/b.tbl".to_string(),
            definition_ledger("tables/b.tbl", "model:dup"),
        );
        ledgers.insert(
            "tables/a.tbl".to_string(),
            definition_ledger("tables/a.tbl", "model:dup"),
        );

        let mut store = crate::memory_graph_store::MemoryGraphStore::new();
        let conflicts = rebuild_graph_from_ledgers(&mut store, &ledgers).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].node_id, "model:dup");
        assert_eq!(
            conflicts[0].definition_origins,
            vec!["tables/a.tbl".to_string(), "tables/b.tbl".to_string()]
        );
        // 择一必须是字典序第一个来源，与插入顺序无关
        let chosen = store.iter_nodes().unwrap().find(|n| n.id == "model:dup");
        assert_eq!(
            chosen.map(|n| n.origin_file.clone()),
            Some(Some("tables/a.tbl".to_string()))
        );
    }

    /// Reference-only 共享目标不构成冲突，多个来源只保留一个节点。
    #[test]
    fn rebuild_keeps_shared_reference_without_conflict() {
        let mut ledgers = BTreeMap::new();
        ledgers.insert(
            "app/a.spg".to_string(),
            reference_ledger("app/a.spg", "model:shared"),
        );
        ledgers.insert(
            "app/b.spg".to_string(),
            reference_ledger("app/b.spg", "model:shared"),
        );

        let mut store = crate::memory_graph_store::MemoryGraphStore::new();
        let conflicts = rebuild_graph_from_ledgers(&mut store, &ledgers).unwrap();
        assert_eq!(conflicts, Vec::new());
        assert_eq!(store.iter_nodes().unwrap().count(), 1);
    }
}
