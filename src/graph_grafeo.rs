//! Grafeo 图存储实现（M59-3 C1）
//!
//! 只走 grafeo 的直写/直读 API（`create_node_with_props`、`create_edge_with_props`、
//! `get_neighbors_outgoing/incoming`、`delete_node/delete_edge`），写入与读取路径上
//! 没有任何查询语言。依据是 M59 的 grafeo spike 实测：逐条 Cypher 导入 90,400 节点
//! 要 287.75s，同一批数据走直写 API 是 4.71s；而无属性索引的
//! `MATCH (n) WHERE n.id = $id` 建边在实验窗口内根本跑不完（全标签扫描）。
//! 所以本实现自持「稳定字符串 id → `NodeId`」映射，建边时直接用 `NodeId`，
//! 并在打开时建好 `id` 属性索引供后续查询层使用。
//!
//! 本模块同时承担 C1（GraphStore 契约）与 C2（索引状态提交 + 导入接线）。
//! 扫描编排侧见 `scanner::indexer::IndexScanStore`：file states、per-file
//! scanner 诊断、diff-refresh checkpoint 都以 `IndexState` 标签的 meta 节点
//! 形式存进同一张图（grafeo 没有独立 KV 接口，named graph 又依赖我们刻意
//! 不启用的 `lpg` feature），打开时按标签还原成内存缓存。
//! redb 仍是默认后端，退役在 D2，前置条件见
//! `docs/plans/2026-09-06-m59-grafeo-implementation-plan.md`。

use crate::diff_refresh::DiffRefreshCheckpoint;
use crate::graph::{Edge, FileState, Node};
use crate::graph_store::{
    EdgeFactKey, GraphEdgeView, GraphNeighbors, GraphReadStore, GraphStoreError, GraphStoreResult,
    GraphWriteStore, IndexCommit, IndexReport, IndexStateStore, edge_dedup_key, merge_upsert_meta,
};
use grafeo::{Config, GrafeoDB, NodeId, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// 所有项目节点共用的标签。
///
/// 没有把 `NodeType` 编成标签：grafeo 的 `Session` 不提供改标签的 API
/// （`add_label`/`remove_label` 只在 core 的 `Node` 值上，改不回 store），
/// 而 `upsert_node` 必须支持把占位节点的类型改成确认后的类型
/// （B1 契约用例 `node_update_is_visible_through_adjacency` 直接要求这点）。
/// 标签一旦写死就会与 `node_type` 属性分叉，所以类型只存属性。
const LABEL_NODE: &str = "Node";

/// 稳定字符串 id（建属性索引的那一个）
const PROP_ID: &str = "id";
/// 节点类型（`NodeType` 的 serde 变体名）
const PROP_NODE_TYPE: &str = "node_type";
/// 节点路径
const PROP_PATH: &str = "path";
/// 节点名
const PROP_NAME: &str = "name";
/// 节点/边 meta（JSON 文本；`Null` 表示无）
const PROP_META: &str = "meta";
/// 产生该实体或关系的源文件
const PROP_ORIGIN_FILE: &str = "origin_file";
/// 边的字段路径
const PROP_FIELD_PATH: &str = "field_path";

/// 索引侧状态记录共用的标签（file states / scanner 诊断 / checkpoint）。
///
/// 不用 grafeo 的 named graph 隔离：`store.graph(name)` 在 `lpg` feature 下
/// 才返回独立图，而我们刻意不开 `lpg`（它连带 gql/cypher/gremlin/sql-pgq
/// 四个解析器，正是体积大头），所以侧状态只能共存在同一张图里，靠标签与
/// 项目节点区分。`iter_nodes`/`node_count`/`rebuild_lookups` 全部按标签
/// 过滤，侧状态不会泄进项目图口径。
const LABEL_INDEX_STATE: &str = "IndexState";
/// IndexState 节点：命名空间 key（`{前缀}{logical_path}` 或单例名）
const PROP_STATE_KEY: &str = "key";
/// IndexState 节点：JSON 载荷
const PROP_STATE_VALUE: &str = "value";

/// meta key 前缀：file state（`file_state:{logical_path}`）
const PREFIX_FILE_STATE: &str = "file_state:";
/// meta key 前缀：per-file scanner 诊断 entry（`scanner_entry:{logical_path}`）
const PREFIX_SCANNER_ENTRY: &str = "scanner_entry:";
/// meta key 单例：diff-refresh checkpoint
const KEY_DIFF_REFRESH_CHECKPOINT: &str = "diff_refresh_checkpoint";

fn file_state_key(path: &str) -> String {
    format!("{PREFIX_FILE_STATE}{path}")
}

fn scanner_entry_key(path: &str) -> String {
    format!("{PREFIX_SCANNER_ENTRY}{path}")
}

/// Grafeo 图存储
///
/// 计数口径：`node_count` 取 `node_ids.len()`（排除 IndexState meta 节点与
/// 任何外来标签节点），`edge_count` 取 `db.edge_count()`——meta 节点从不上边，
/// 引擎计数又按当前 epoch 过滤 tombstone，两边都恰好是项目图口径。
///
/// 删除节点必须自己先删光它的邻边：grafeo 的 `delete_node` 只标记节点版本链
/// 并摘掉标签/属性索引，**不级联删边**，残留的边会让邻接表指向一个已删节点。
///
/// 与 redb 的提交语义差异：`edge`+`storage` 特性集下 grafeo 没有事务
/// （`begin_transaction`/`commit` 是 `lpg` feature 的 API），apply 阶段的
/// 节点/边写入立即生效，`persist_index` 只落侧状态。中途崩溃会留下
/// 「图已部分更新、file_states 仍旧」的窗口——下轮 diff 把同一批文件再判脏，
/// 按旧 state 的 `node_ids` 重放删除+重写即可收敛（节点 id 与边事实都是
/// 内容决定的，重放幂等）。已知残留风险：崩溃后文件内容又回滚时，崩溃前
/// 写入的新节点可能变成无人引用的孤儿（没有任何已提交 state 记得它们）；
/// 是否引入按 `origin_file` 的 GC 留给 D1 验收再评估。
pub struct GrafeoGraphStore {
    db: GrafeoDB,
    /// 稳定字符串 id → grafeo `NodeId`，建边与点查都靠它，避免按属性扫描
    node_ids: HashMap<String, NodeId>,
    /// 边去重键集合，与 `memory_graph_store` / `graph_redb` 的 `seen_edges` 同口径
    seen_edges: HashSet<EdgeFactKey>,
    /// IndexState meta 节点：key → `NodeId` 定位表
    meta_nodes: HashMap<String, NodeId>,
    /// 已落库的 file states 缓存（打开时随 meta 节点复原，diff 的唯一输入）
    file_states: HashMap<String, FileState>,
    /// 已落库的 per-file scanner 诊断缓存
    scanner_entries: HashMap<String, Vec<u8>>,
    /// 已落库的 diff-refresh checkpoint
    checkpoint: Option<DiffRefreshCheckpoint>,
    /// 本轮 upsert 过的节点 id（persist 后清空，与 redb `dirty_nodes` 同口径）
    dirty_nodes: HashSet<String>,
    /// 本轮删除的节点 id（persist 后清空，与 redb `removed_nodes` 同口径）
    removed_nodes: HashSet<String>,
}

impl std::fmt::Debug for GrafeoGraphStore {
    /// `GrafeoDB` 未实现 `Debug`，这里只打印可观测的规模信息。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrafeoGraphStore")
            .field("nodes", &self.node_ids.len())
            .field("edge_facts", &self.seen_edges.len())
            .finish()
    }
}

impl GrafeoGraphStore {
    /// 建一个纯内存库（测试与 browser 侧的等价后端）
    pub fn in_memory() -> GraphStoreResult<Self> {
        let db = GrafeoDB::with_config(Config::in_memory()).map_err(|err| {
            GraphStoreError::OpenFailed {
                path: "<memory>".to_string(),
                reason: err.to_string(),
            }
        })?;
        Ok(Self::attach(db))
    }

    /// 打开（或创建）一个持久化库，并从现有内容重建 id 映射与去重键。
    ///
    /// **路径须以 `.grafeo` 结尾**：grafeo 对新路径按扩展名选存储格式，
    /// 只有单文件格式支持 `open_read_only`（共享读），目录格式是 legacy WAL 布局。
    pub fn open(path: impl AsRef<Path>) -> GraphStoreResult<Self> {
        let path = path.as_ref();
        let db = GrafeoDB::with_config(Config::persistent(path)).map_err(|err| {
            GraphStoreError::OpenFailed {
                path: path.display().to_string(),
                reason: err.to_string(),
            }
        })?;
        let mut store = Self::attach(db);
        store.rebuild_lookups()?;
        Ok(store)
    }

    /// 以只读方式打开已有库（多进程共享读，不回放 WAL、不允许写）
    pub fn open_read_only(path: impl AsRef<Path>) -> GraphStoreResult<Self> {
        let path = path.as_ref();
        let db = GrafeoDB::open_read_only(path).map_err(|err| GraphStoreError::OpenFailed {
            path: path.display().to_string(),
            reason: err.to_string(),
        })?;
        let mut store = Self::attach(db);
        store.rebuild_lookups()?;
        Ok(store)
    }

    /// 关闭库：落 checkpoint 到 `.grafeo` 文件并释放文件锁。
    /// 消费 `self`，因为 grafeo 关闭后该句柄不可再用。
    pub fn close(self) -> GraphStoreResult<()> {
        self.db.close().map_err(|err| GraphStoreError::WriteFailed {
            reason: err.to_string(),
        })
    }

    /// 建索引并包装成 store。`id` 属性索引是直写路径的前提：
    /// 没有它，后续查询层按 id 定位就退化成全标签扫描。
    fn attach(db: GrafeoDB) -> Self {
        db.create_property_index(PROP_ID);
        Self {
            db,
            node_ids: HashMap::new(),
            seen_edges: HashSet::new(),
            meta_nodes: HashMap::new(),
            file_states: HashMap::new(),
            scanner_entries: HashMap::new(),
            checkpoint: None,
            dirty_nodes: HashSet::new(),
            removed_nodes: HashSet::new(),
        }
    }

    /// 从库里现有的节点和边重建 `node_ids`、`seen_edges` 与索引侧状态缓存。
    ///
    /// 前两样是进程内派生状态，重启后必须复原，否则重开的库会把已存在的边
    /// 当成新事实重复写入，也无法按字符串 id 定位节点。IndexState 记录同样
    /// 在这里还原成内存缓存——file states 是下一轮增量 diff 的唯一输入，
    /// 丢了它等于全库强制重建。
    fn rebuild_lookups(&mut self) -> GraphStoreResult<()> {
        let mut by_node_id: HashMap<NodeId, String> = HashMap::new();
        // IndexState 记录先只收裸 (key, value, NodeId)：iter_nodes 持有 db 的
        // 不可变借用，解码写不进自身字段——循环结束后统一还原进缓存。
        let mut state_records = Vec::new();
        for node in self.db.iter_nodes() {
            if node.has_label(LABEL_INDEX_STATE) {
                state_records.push((
                    read_text(|key| node.get_property(key).cloned(), PROP_STATE_KEY),
                    read_text(|key| node.get_property(key).cloned(), PROP_STATE_VALUE),
                    node.id,
                ));
                continue;
            }
            // 非本实现写入的标签不归本 store 解释（未来 projection 等形态）。
            if !node.has_label(LABEL_NODE) {
                continue;
            }
            let id =
                read_text(|key| node.get_property(key).cloned(), PROP_ID).ok_or_else(|| {
                    GraphStoreError::Corrupted {
                        reason: format!("grafeo 节点 {:?} 缺少 {PROP_ID} 属性", node.id),
                    }
                })?;
            by_node_id.insert(node.id, id.clone());
            self.node_ids.insert(id, node.id);
        }
        for (key, value, node_id) in state_records {
            self.load_state_record(key, value, node_id)?;
        }

        let mut facts = HashSet::new();
        for edge in self.db.iter_edges() {
            let (Some(from), Some(to)) = (by_node_id.get(&edge.src), by_node_id.get(&edge.dst))
            else {
                return Err(GraphStoreError::Corrupted {
                    reason: format!("grafeo 边 {:?} 的端点不存在", edge.id),
                });
            };
            let restored = edge_from_props(from.clone(), to.clone(), &edge.edge_type, |key| {
                edge.get_property(key).cloned()
            })?;
            facts.insert(edge_dedup_key(&restored));
        }
        self.seen_edges = facts;
        Ok(())
    }

    /// 把一条 IndexState 记录的裸字段还原进侧状态缓存并登记定位表。
    ///
    /// 未知 key 按 Corrupted 报错而不是跳过：本文件只由本实现写入，出现
    /// 不认识的记录形态等于持久化数据超出当前 schema——静默跳过会让下一轮
    /// 扫描读到残缺的侧状态，表现为「文件怎么改都不脏」这种更难查的症状。
    fn load_state_record(
        &mut self,
        key: Option<String>,
        value: Option<String>,
        node_id: NodeId,
    ) -> GraphStoreResult<()> {
        let key = key.ok_or_else(|| GraphStoreError::Corrupted {
            reason: format!("索引状态节点 {node_id:?} 缺少 {PROP_STATE_KEY} 属性"),
        })?;
        let value = value.unwrap_or_default();
        if let Some(path) = key.strip_prefix(PREFIX_FILE_STATE) {
            let state = serde_json::from_str::<FileState>(&value).map_err(|err| {
                GraphStoreError::DeserializeFailed {
                    reason: format!("还原 file_state {path} 失败：{err}"),
                }
            })?;
            self.file_states.insert(path.to_string(), state);
        } else if let Some(path) = key.strip_prefix(PREFIX_SCANNER_ENTRY) {
            self.scanner_entries
                .insert(path.to_string(), value.into_bytes());
        } else if key == KEY_DIFF_REFRESH_CHECKPOINT {
            self.checkpoint = Some(serde_json::from_str(&value).map_err(|err| {
                GraphStoreError::DeserializeFailed {
                    reason: format!("还原 diff-refresh checkpoint 失败：{err}"),
                }
            })?);
        } else {
            return Err(GraphStoreError::Corrupted {
                reason: format!("未知索引状态 key：{key}"),
            });
        }
        self.meta_nodes.insert(key, node_id);
        Ok(())
    }

    /// upsert 一条 IndexState 记录（按 key 定位，value 为 JSON 文本）
    fn put_state(&mut self, key: &str, value: &str) -> GraphStoreResult<()> {
        let session = self.db.session();
        if let Some(&existing) = self.meta_nodes.get(key) {
            session
                .set_node_property(existing, PROP_STATE_VALUE, Value::from(value))
                .map_err(|err| GraphStoreError::WriteFailed {
                    reason: format!("写索引状态 {key} 失败：{err}"),
                })?;
            return Ok(());
        }
        let created = session
            .create_node_with_props(
                &[LABEL_INDEX_STATE],
                [
                    (PROP_STATE_KEY, Value::from(key)),
                    (PROP_STATE_VALUE, Value::from(value)),
                ],
            )
            .map_err(|err| GraphStoreError::WriteFailed {
                reason: format!("建索引状态 {key} 失败：{err}"),
            })?;
        self.meta_nodes.insert(key.to_string(), created);
        Ok(())
    }

    /// 删除一条 IndexState 记录（不存在则视为幂等 no-op）
    fn delete_state(&mut self, key: &str) -> GraphStoreResult<()> {
        let Some(node) = self.meta_nodes.remove(key) else {
            return Ok(());
        };
        // meta 节点从不上边，不需要级联删邻边。
        self.db.session().delete_node(node);
        Ok(())
    }

    /// 全库 per-file scanner 诊断 entry（顺序不保证，调用方自行排序）。
    /// 与 `GraphDB::load_scanner_diagnostic_entries` 同出口。
    pub fn load_scanner_diagnostic_entries(&self) -> GraphStoreResult<Vec<(String, Vec<u8>)>> {
        Ok(self
            .scanner_entries
            .iter()
            .map(|(path, bytes)| (path.clone(), bytes.clone()))
            .collect())
    }

    /// 已落库的 diff-refresh checkpoint（无则 `None`），
    /// 与 `GraphDB::load_diff_refresh_checkpoint` 同出口。
    pub fn load_diff_refresh_checkpoint(&self) -> GraphStoreResult<Option<DiffRefreshCheckpoint>> {
        Ok(self.checkpoint.clone())
    }
    fn neighbors_of(
        &self,
        node_id: &str,
        node: NodeId,
        outgoing: bool,
    ) -> GraphStoreResult<Vec<GraphEdgeView>> {
        let session = self.db.session();
        let pairs = if outgoing {
            session.get_neighbors_outgoing(node)
        } else {
            session.get_neighbors_incoming(node)
        };

        let mut views = Vec::with_capacity(pairs.len());
        for (other, edge_id) in pairs {
            // 端点与边必然可读：建边时校验过端点，删节点时连带删边。
            // 读不到说明不变量已被破坏，按 Corrupted 报错而不是静默跳过——
            // 静默跳过会把「图少了一条边」伪装成正常结果。
            let edge = session
                .get_edge(edge_id)
                .ok_or_else(|| GraphStoreError::Corrupted {
                    reason: format!("邻接表引用了不存在的边 {edge_id:?}"),
                })?;
            let other_node = session
                .get_node(other)
                .ok_or_else(|| GraphStoreError::Corrupted {
                    reason: format!("边 {edge_id:?} 的对端节点 {other:?} 不存在"),
                })?;
            let node_view = node_from_props(|key| other_node.get_property(key).cloned())?;
            let (from, to) = if outgoing {
                (node_id.to_string(), node_view.id.clone())
            } else {
                (node_view.id.clone(), node_id.to_string())
            };
            let edge_view = edge_from_props(from, to, &edge.edge_type, |key| {
                edge.get_property(key).cloned()
            })?;
            views.push(GraphEdgeView {
                node: node_view,
                edge: edge_view,
            });
        }
        Ok(views)
    }
}

impl GraphReadStore for GrafeoGraphStore {
    fn get_node(&self, node_id: &str) -> GraphStoreResult<Option<Node>> {
        let Some(&node) = self.node_ids.get(node_id) else {
            return Ok(None);
        };
        let session = self.db.session();
        let Some(raw) = session.get_node(node) else {
            return Err(GraphStoreError::Corrupted {
                reason: format!("id 映射指向不存在的 grafeo 节点：{node_id}"),
            });
        };
        Ok(Some(node_from_props(|key| raw.get_property(key).cloned())?))
    }

    fn get_node_edges(&self, node_id: &str) -> GraphStoreResult<Option<GraphNeighbors>> {
        // 契约：`None` = 节点不存在，`Some(空邻居)` = 节点存在但孤立。
        let Some(&node) = self.node_ids.get(node_id) else {
            return Ok(None);
        };
        Ok(Some(GraphNeighbors {
            outgoing: self.neighbors_of(node_id, node, true)?,
            incoming: self.neighbors_of(node_id, node, false)?,
        }))
    }

    fn node_count(&self) -> GraphStoreResult<usize> {
        // `db.node_count()` 会把 IndexState meta 节点一起算进来；项目图口径
        // 以 id 映射表为准（与 `iter_nodes`/`rebuild_lookups` 的标签过滤一致）。
        Ok(self.node_ids.len())
    }

    fn edge_count(&self) -> GraphStoreResult<usize> {
        Ok(self.db.edge_count())
    }

    fn iter_nodes(&self) -> GraphStoreResult<Box<dyn Iterator<Item = Node> + '_>> {
        // 先收集再返回：属性还原会失败（坏数据），而 Iterator<Item = Node>
        // 没有报错位置，惰性地 `filter_map(.ok())` 会静默少节点。
        // 返回类型本来就是 owned Node，collect 不额外增加克隆量级。
        let mut nodes = Vec::with_capacity(self.node_ids.len());
        for raw in self.db.iter_nodes() {
            if !raw.has_label(LABEL_NODE) {
                continue;
            }
            nodes.push(node_from_props(|key| raw.get_property(key).cloned())?);
        }
        Ok(Box::new(nodes.into_iter()))
    }
}

impl GraphWriteStore for GrafeoGraphStore {
    fn upsert_node(&mut self, node: Node) -> GraphStoreResult<()> {
        let session = self.db.session();
        let node_type = enum_name(&node.node_type)?;
        let meta_text = meta_to_text(node.meta.as_ref())?;

        if let Some(&existing) = self.node_ids.get(&node.id) {
            // 共享的 meta 合并规则：无 meta 的写入不抹掉既有 meta，
            // 占位类型不降级已确认类型。与 memory/redb 调的是同一份函数。
            let previous =
                session
                    .get_node(existing)
                    .ok_or_else(|| GraphStoreError::Corrupted {
                        reason: format!("id 映射指向不存在的 grafeo 节点：{}", node.id),
                    })?;
            let merged = merge_upsert_meta(
                read_meta(|key| previous.get_property(key).cloned())?,
                node.meta.clone(),
            );
            let updates = [
                (PROP_NODE_TYPE, Value::from(node_type.as_str())),
                (PROP_PATH, Value::from(node.path.as_str())),
                (PROP_NAME, Value::from(node.name.as_str())),
                (PROP_META, meta_to_text(merged.as_ref())?),
                (PROP_ORIGIN_FILE, optional_text(node.origin_file.as_deref())),
            ];
            for (key, value) in updates {
                session
                    .set_node_property(existing, key, value)
                    .map_err(|err| GraphStoreError::WriteFailed {
                        reason: format!("写节点属性 {key} 失败：{err}"),
                    })?;
            }
            self.dirty_nodes.insert(node.id.clone());
            self.removed_nodes.remove(&node.id);
            return Ok(());
        }

        let created = session
            .create_node_with_props(
                &[LABEL_NODE],
                [
                    (PROP_ID, Value::from(node.id.as_str())),
                    (PROP_NODE_TYPE, Value::from(node_type.as_str())),
                    (PROP_PATH, Value::from(node.path.as_str())),
                    (PROP_NAME, Value::from(node.name.as_str())),
                    (PROP_META, meta_text),
                    (PROP_ORIGIN_FILE, optional_text(node.origin_file.as_deref())),
                ],
            )
            .map_err(|err| GraphStoreError::WriteFailed {
                reason: format!("建节点 {} 失败：{err}", node.id),
            })?;
        self.node_ids.insert(node.id.clone(), created);
        self.dirty_nodes.insert(node.id.clone());
        self.removed_nodes.remove(&node.id);
        Ok(())
    }

    fn add_edge(&mut self, edge: Edge) -> GraphStoreResult<()> {
        // 端点缺失静默忽略：扫描顺序会让引用先于定义出现，这不是错误
        // （与 memory/redb 一致，也不在这里顺手建占位节点）。
        let (Some(&src), Some(&dst)) = (self.node_ids.get(&edge.from), self.node_ids.get(&edge.to))
        else {
            return Ok(());
        };
        // 按完整事实去重：关系相同但 field_path / meta / 来源不同的是不同事实。
        if self.seen_edges.contains(&edge_dedup_key(&edge)) {
            return Ok(());
        }

        let edge_type = enum_name(&edge.edge_type)?;
        self.db
            .session()
            .create_edge_with_props(
                src,
                dst,
                &edge_type,
                [
                    (PROP_FIELD_PATH, optional_text(edge.field_path.as_deref())),
                    (PROP_META, meta_to_text(edge.meta.as_ref())?),
                    (PROP_ORIGIN_FILE, optional_text(edge.origin_file.as_deref())),
                ],
            )
            .map_err(|err| GraphStoreError::WriteFailed {
                reason: format!("建边 {} -> {} 失败：{err}", edge.from, edge.to),
            })?;
        // 落库成功后才登记去重键：create_edge 失败时提前登记会把重试当重复静默丢弃。
        self.seen_edges.insert(edge_dedup_key(&edge));
        Ok(())
    }

    fn remove_nodes_by_ids(&mut self, node_ids: &[String]) -> GraphStoreResult<()> {
        let session = self.db.session();
        for id in node_ids {
            // 与 redb 同口径：即使 id 不在图里也进 removed 账（调用方只关心
            // 「本轮要求删除的集合」，不区分它先前是否存在）。
            self.removed_nodes.insert(id.clone());
            self.dirty_nodes.remove(id);
            let Some(node) = self.node_ids.remove(id) else {
                continue;
            };
            // 必须显式删邻边：grafeo 的 delete_node 不级联。
            for (_, edge_id) in session.get_neighbors_outgoing(node) {
                session.delete_edge(edge_id);
            }
            for (_, edge_id) in session.get_neighbors_incoming(node) {
                session.delete_edge(edge_id);
            }
            session.delete_node(node);
        }

        // 去重键一起清：否则节点删后重建，原来那条边会被当重复丢弃，图里永久缺边。
        let removed: HashSet<&str> = node_ids.iter().map(String::as_str).collect();
        self.seen_edges.retain(|(from, to, _, _, _, _)| {
            !removed.contains(from.as_str()) && !removed.contains(to.as_str())
        });
        Ok(())
    }
}

impl IndexStateStore for GrafeoGraphStore {
    /// file states 是增量 diff 的唯一输入，打开时已随 meta 节点复原成缓存，
    /// 这里直接返回克隆——与 redb 每次新开只读事务扫表的语义等价。
    fn load_file_states(&self) -> GraphStoreResult<HashMap<String, FileState>> {
        Ok(self.file_states.clone())
    }

    /// 提交语义与 redb `persist_commit` 同口径：file states 全量对账
    /// （新增/变化 upsert、缺席删除）、scanner entries 覆盖 + 按删除清单移除、
    /// checkpoint 覆盖或清除。
    ///
    /// 与 redb 的差异：grafeo 无事务，逐条写就是提交本身——函数中途失败会
    /// 留下部分落库的侧状态。这与 redb 单事务的原子性不同，但各条记录
    /// 互相独立、幂等可重放，下轮提交自然会补齐（见类型文档的崩溃窗口说明）。
    fn persist_index(&mut self, commit: IndexCommit) -> GraphStoreResult<IndexReport> {
        let IndexCommit {
            file_states,
            dirty_nodes,
            deleted_nodes,
            checkpoint,
            // grafeo 直写不消费 M56 delta：edge-key 快照服务于 redb v2 shadow
            // 的 Stale/Current 翻转，本后端没有影子层；`wants_index_delta`
            // 返回 false 已保证扫描端不会收集它。
            delta: _,
            scanner_entries,
            scanner_deleted_paths,
        } = commit;

        let indexed = file_states.len();
        let dirty = dirty_nodes.len();
        let deleted = deleted_nodes.len();

        for (path, state) in &file_states {
            if self.file_states.get(path) != Some(state) {
                let text = serde_json::to_string(state).map_err(|err| {
                    GraphStoreError::SerializeFailed {
                        reason: format!("序列化 file_state {path} 失败：{err}"),
                    }
                })?;
                self.put_state(&file_state_key(path), &text)?;
            }
        }
        let stale_states: Vec<String> = self
            .file_states
            .keys()
            .filter(|path| !file_states.contains_key(*path))
            .cloned()
            .collect();
        for path in stale_states {
            self.delete_state(&file_state_key(&path))?;
        }
        self.file_states = file_states;

        for (path, bytes) in &scanner_entries {
            // entry 是本实现自己序列化的 JSON，非 UTF-8 意味着上游写坏了格式。
            let text = String::from_utf8(bytes.clone()).map_err(|err| {
                GraphStoreError::SerializeFailed {
                    reason: format!("scanner 诊断 entry {path} 不是有效 UTF-8：{err}"),
                }
            })?;
            self.put_state(&scanner_entry_key(path), &text)?;
            self.scanner_entries.insert(path.clone(), bytes.clone());
        }
        for path in &scanner_deleted_paths {
            self.delete_state(&scanner_entry_key(path))?;
            self.scanner_entries.remove(path);
        }

        match &checkpoint {
            Some(checkpoint) => {
                let text = serde_json::to_string(checkpoint).map_err(|err| {
                    GraphStoreError::SerializeFailed {
                        reason: format!("序列化 diff-refresh checkpoint 失败：{err}"),
                    }
                })?;
                self.put_state(KEY_DIFF_REFRESH_CHECKPOINT, &text)?;
            }
            None => self.delete_state(KEY_DIFF_REFRESH_CHECKPOINT)?,
        }
        self.checkpoint = checkpoint;

        self.dirty_nodes.clear();
        self.removed_nodes.clear();

        // 与 memory/redb 同一出口口径：store 层报节点账数，
        // 文件口径由报告出口层覆盖。
        Ok(IndexReport {
            indexed,
            unchanged: indexed.saturating_sub(dirty),
            dirty,
            deleted,
        })
    }
}

#[cfg(feature = "cli-local")]
impl crate::scanner::indexer::IndexScanStore for GrafeoGraphStore {
    fn load_scanner_diagnostic_entries(&self) -> GraphStoreResult<Vec<(String, Vec<u8>)>> {
        GrafeoGraphStore::load_scanner_diagnostic_entries(self)
    }

    fn pending_dirty_nodes(&self) -> Vec<String> {
        self.dirty_nodes.iter().cloned().collect()
    }

    fn pending_removed_nodes(&self) -> Vec<String> {
        self.removed_nodes.iter().cloned().collect()
    }

    /// grafeo 直写不消费 M56 delta（无 redb v2 shadow 层），返回 false 让扫描
    /// 端跳过两次 incident-edge 全邻接收集。
    fn wants_index_delta(&self) -> bool {
        false
    }
}

/// 把 serde 单元变体枚举转成稳定字符串。
///
/// 不手写 match：`NodeType` 6 个、`EdgeType` 22 个变体，抄一遍就多一处
/// 会和 serde 口径分叉的真相，新增边类型时还得记得改这里。
fn enum_name<T: serde::Serialize>(value: &T) -> GraphStoreResult<String> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => Ok(name),
        Ok(other) => Err(GraphStoreError::SerializeFailed {
            reason: format!("期望单元变体字符串，实际得到 {other}"),
        }),
        Err(err) => Err(GraphStoreError::SerializeFailed {
            reason: err.to_string(),
        }),
    }
}

/// 由稳定字符串还原 serde 单元变体枚举
fn enum_from_name<T: serde::de::DeserializeOwned>(name: &str) -> GraphStoreResult<T> {
    serde_json::from_value(serde_json::Value::String(name.to_string())).map_err(|err| {
        GraphStoreError::DeserializeFailed {
            reason: format!("无法还原枚举变体 {name}：{err}"),
        }
    })
}

/// `Option<&str>` → grafeo 属性值；`None` 写成 `Null` 而不是省略属性，
/// 这样 upsert 覆盖写回 `None` 时也是一次普通的属性赋值。
fn optional_text(value: Option<&str>) -> Value {
    match value {
        Some(text) => Value::from(text),
        None => Value::Null,
    }
}

/// meta JSON → 属性值（紧凑 JSON 文本）
fn meta_to_text(meta: Option<&serde_json::Value>) -> GraphStoreResult<Value> {
    match meta {
        Some(meta) => serde_json::to_string(meta)
            .map(|text| Value::from(text.as_str()))
            .map_err(|err| GraphStoreError::SerializeFailed {
                reason: format!("序列化 meta 失败：{err}"),
            }),
        None => Ok(Value::Null),
    }
}

/// 读字符串属性；`Null` 与缺失都视为 `None`
fn read_text(lookup: impl Fn(&str) -> Option<Value>, key: &str) -> Option<String> {
    lookup(key)
        .as_ref()
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// 读 meta 属性并反序列化
fn read_meta(
    lookup: impl Fn(&str) -> Option<Value>,
) -> GraphStoreResult<Option<serde_json::Value>> {
    let Some(text) = read_text(lookup, PROP_META) else {
        return Ok(None);
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|err| GraphStoreError::DeserializeFailed {
            reason: format!("反序列化 meta 失败：{err}"),
        })
}

/// 由 grafeo 节点属性还原项目 `Node`。
///
/// 入参是属性读取闭包而不是 `&grafeo_core::graph::lpg::Node`：该类型没有
/// 被 `grafeo` 顶层 re-export，写进函数签名就得为此多引一个 `grafeo-core`
/// 直接依赖并自己对齐版本。值可以照用，只是不能命名。
fn node_from_props(lookup: impl Fn(&str) -> Option<Value>) -> GraphStoreResult<Node> {
    let id = read_text(&lookup, PROP_ID).ok_or_else(|| GraphStoreError::Corrupted {
        reason: format!("grafeo 节点缺少 {PROP_ID} 属性"),
    })?;
    let node_type_name =
        read_text(&lookup, PROP_NODE_TYPE).ok_or_else(|| GraphStoreError::Corrupted {
            reason: format!("grafeo 节点 {id} 缺少 {PROP_NODE_TYPE} 属性"),
        })?;
    // upsert_node 两条路径都必写 path/name——缺失同样是坏数据，与 id/node_type 同口径报 Corrupted，
    // 不能让损坏节点以空路径/空名字混进 adjacency 结果。
    let path = read_text(&lookup, PROP_PATH).ok_or_else(|| GraphStoreError::Corrupted {
        reason: format!("grafeo 节点 {id} 缺少 {PROP_PATH} 属性"),
    })?;
    let name = read_text(&lookup, PROP_NAME).ok_or_else(|| GraphStoreError::Corrupted {
        reason: format!("grafeo 节点 {id} 缺少 {PROP_NAME} 属性"),
    })?;
    Ok(Node {
        id,
        node_type: enum_from_name(&node_type_name)?,
        path,
        name,
        meta: read_meta(&lookup)?,
        origin_file: read_text(&lookup, PROP_ORIGIN_FILE),
    })
}

/// 由 grafeo 边类型与属性还原项目 `Edge`（端点字符串 id 由调用方给出）
fn edge_from_props(
    from: String,
    to: String,
    edge_type_name: &str,
    lookup: impl Fn(&str) -> Option<Value>,
) -> GraphStoreResult<Edge> {
    Ok(Edge {
        from,
        to,
        edge_type: enum_from_name(edge_type_name)?,
        field_path: read_text(&lookup, PROP_FIELD_PATH),
        meta: read_meta(&lookup)?,
        origin_file: read_text(&lookup, PROP_ORIGIN_FILE),
    })
}
