//! 写入校验：扫描器写进图存储的每个节点和边，都先对照 `graph_schema.rs` 的契约表。
//!
//! 背景（PR #4 的 S2，计划步 L0）：契约表只描述不拦截时，端点类型错误、缺必需 `meta`
//! 键或未登记的形状都能落盘，直到有人跑一致性测试才发现。本模块把同一张表装到写入口上：
//! 违规项**不转发**给底层存储，而是返回带「文件、节点或边、原因」的错误，由扫描逐层
//! 向上传播（不吞、不静默丢弃、不 `panic!`）。
//!
//! 规则不在这里重写：节点与边的判定都是 [`crate::graph_schema::node_violations`] /
//! [`crate::graph_schema::edge_violations`]，`tests/graph_schema_tests.rs` 的整图一致性
//! 检查调用的是同一对函数。
//!
//! 为什么是「写入口」而不是各个存储实现：扫描器的全部产出都经过
//! `scanner/utils.rs` 的三个写入 helper，再进入 `process_spg_file_*` /
//! `process_tbl_file_*` 两个入口；守卫装在这两个入口上，对 redb / Grafeo / 内存三种
//! 后端一视同仁。把校验放进各存储的 `add_edge` 会连带拦住大量手工搭图的测试与
//! 浏览器侧的可视化图，那些不是「扫描结果」。
//!
//! 边的端点类型取自「本次处理过程中写过的节点」缓存；缓存里没有的端点（例如增量
//! 扫描时指向别的文件早已写入的节点）再问底层存储（[`GraphWriteStore::node_type_of`]）。
//! 两处都找不到时底层存储本来就会因端点缺失而丢弃这条边（既有契约），没有东西落盘，
//! 因此只核对不依赖端点的项（边类型已登记、`meta` 键）。

use crate::graph::{Edge, Node, NodeType};
use crate::graph_schema::{self, GraphSchema};
use crate::graph_store::{GraphStoreError, GraphStoreResult, GraphWriteStore};
use std::collections::HashMap;

/// 带契约校验的写入包装。
///
/// 一个守卫对应「一个源文件的一次处理」：`source_file` 只用于错误信息，缓存也只在
/// 这次处理内有效，所以不会跨文件积累内存。
pub struct SchemaGuard<'a> {
    inner: &'a mut dyn GraphWriteStore,
    schema: &'a GraphSchema,
    source_file: &'a str,
    /// 本次处理内 upsert 过的节点类型（边端点解析的快速路径）。
    node_types: HashMap<String, NodeType>,
}

impl<'a> SchemaGuard<'a> {
    /// 用默认契约表（[`graph_schema::schema`]）包装一个写入存储。
    pub fn new(inner: &'a mut dyn GraphWriteStore, source_file: &'a str) -> Self {
        Self::with_schema(inner, graph_schema::schema(), source_file)
    }

    /// 指定契约表。生产路径用 [`Self::new`]；测试用它构造「少一行 / 多一个必需键」的
    /// 表，验证每一类违规都真的被拒收。
    pub fn with_schema(
        inner: &'a mut dyn GraphWriteStore,
        schema: &'a GraphSchema,
        source_file: &'a str,
    ) -> Self {
        Self {
            inner,
            schema,
            source_file,
            node_types: HashMap::new(),
        }
    }

    /// 节点类型：先看本次处理写过的，再问底层存储，并把答案记进缓存。
    fn resolve_type(&mut self, node_id: &str) -> GraphStoreResult<Option<NodeType>> {
        if let Some(node_type) = self.node_types.get(node_id) {
            return Ok(Some(node_type.clone()));
        }
        let found = self.inner.node_type_of(node_id)?;
        if let Some(node_type) = &found {
            self.node_types
                .insert(node_id.to_string(), node_type.clone());
        }
        Ok(found)
    }

    /// 把违规列表折成一条存储错误；空列表表示合规。
    fn reject(&self, violations: Vec<String>) -> GraphStoreResult<()> {
        if violations.is_empty() {
            return Ok(());
        }
        Err(GraphStoreError::SchemaViolation {
            message: format!("{}：{}", self.source_file, violations.join("；")),
        })
    }
}

impl GraphWriteStore for SchemaGuard<'_> {
    fn upsert_node(&mut self, node: Node) -> GraphStoreResult<()> {
        // 写入方没带 meta 且节点已存在时，存储会沿用既有 meta（`merge_upsert_meta`
        // 规则 1），此时不核必需键。只有契约要求必需键的类型才需要查「是否已存在」。
        let needs_required = node.meta.is_none()
            && self
                .schema
                .node_row(&node.node_type)
                .is_some_and(|row| row.meta_keys.iter().any(|key| key.required));
        let inherits_meta = needs_required && self.resolve_type(&node.id)?.is_some();
        self.reject(graph_schema::node_violations(
            self.schema,
            &node,
            inherits_meta,
        ))?;
        self.node_types
            .insert(node.id.clone(), node.node_type.clone());
        self.inner.upsert_node(node)
    }

    fn add_edge(&mut self, edge: Edge) -> GraphStoreResult<()> {
        let from_type = self.resolve_type(&edge.from)?;
        let to_type = self.resolve_type(&edge.to)?;
        self.reject(graph_schema::edge_violations(
            self.schema,
            &edge,
            from_type.as_ref(),
            to_type.as_ref(),
        ))?;
        self.inner.add_edge(edge)
    }

    fn remove_nodes_by_ids(&mut self, node_ids: &[String]) -> GraphStoreResult<()> {
        for id in node_ids {
            self.node_types.remove(id);
        }
        self.inner.remove_nodes_by_ids(node_ids)
    }

    fn node_type_of(&self, node_id: &str) -> GraphStoreResult<Option<NodeType>> {
        match self.node_types.get(node_id) {
            Some(node_type) => Ok(Some(node_type.clone())),
            None => self.inner.node_type_of(node_id),
        }
    }
}

/// 干跑用的写入沉：只记录「本批写过的节点类型」，丢弃一切内容，**不改动任何真实存储**。
///
/// 用法：先让一批文件经 [`SchemaGuard`] 写进沉里，把全部违规一次性查出来；没有违规才对
/// 真实存储做批删与真实写入（`scanner/indexer.rs::apply_incremental_changes`）。这样
/// 校验失败时图和文件状态都还是原样（Grafeo 直写、没有事务，事后无法回滚）。
///
/// 端点类型解析顺序与真实写入一致：本批先写的文件优先；其次是真实存储里已有、且本批
/// 不会删除的节点。本批要删除的节点（`removed`）在真实写入时已不存在，所以视为不存在，
/// 除非本批又把它们写了回来。
pub struct DryRunSink<'a> {
    real: &'a dyn GraphWriteStore,
    removed: &'a std::collections::HashSet<&'a str>,
    node_types: HashMap<String, NodeType>,
}

impl<'a> DryRunSink<'a> {
    /// `real` 只读（只会调 `node_type_of`）；`removed` 是本批将批删的节点 id。
    pub fn new(
        real: &'a dyn GraphWriteStore,
        removed: &'a std::collections::HashSet<&'a str>,
    ) -> Self {
        Self {
            real,
            removed,
            node_types: HashMap::new(),
        }
    }
}

impl GraphWriteStore for DryRunSink<'_> {
    fn upsert_node(&mut self, node: Node) -> GraphStoreResult<()> {
        self.node_types.insert(node.id, node.node_type);
        Ok(())
    }

    fn add_edge(&mut self, _edge: Edge) -> GraphStoreResult<()> {
        Ok(())
    }

    fn remove_nodes_by_ids(&mut self, node_ids: &[String]) -> GraphStoreResult<()> {
        for id in node_ids {
            self.node_types.remove(id);
        }
        Ok(())
    }

    fn node_type_of(&self, node_id: &str) -> GraphStoreResult<Option<NodeType>> {
        if let Some(node_type) = self.node_types.get(node_id) {
            return Ok(Some(node_type.clone()));
        }
        if self.removed.contains(node_id) {
            return Ok(None);
        }
        self.real.node_type_of(node_id)
    }
}
