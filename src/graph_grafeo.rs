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
//! 本模块只负责 GraphStore 契约（C1）。索引状态提交（`IndexStateStore`）、
//! 全量/增量导入接线属于 C2，不在此文件内实现。
//! redb 仍是默认后端，退役在 D2，前置条件见
//! `docs/plans/2026-09-06-m59-grafeo-implementation-plan.md`。

use crate::graph::{Edge, Node};
use crate::graph_store::{
    EdgeFactKey, GraphEdgeView, GraphNeighbors, GraphReadStore, GraphStoreError, GraphStoreResult,
    GraphWriteStore, edge_dedup_key, merge_upsert_meta,
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

/// Grafeo 图存储
///
/// 计数直接取 `db.node_count()` / `db.edge_count()`——两者都按当前 epoch
/// 过滤掉已删除记录，不是把 tombstone 算进去的物理条数，所以可以当权威口径，
/// 不另维护一份计数器（自带计数器只会掩盖引擎层的不一致）。
///
/// 删除节点必须自己先删光它的邻边：grafeo 的 `delete_node` 只标记节点版本链
/// 并摘掉标签/属性索引，**不级联删边**，残留的边会让邻接表指向一个已删节点。
pub struct GrafeoGraphStore {
    db: GrafeoDB,
    /// 稳定字符串 id → grafeo `NodeId`，建边与点查都靠它，避免按属性扫描
    node_ids: HashMap<String, NodeId>,
    /// 边去重键集合，与 `memory_graph_store` / `graph_redb` 的 `seen_edges` 同口径
    seen_edges: HashSet<EdgeFactKey>,
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
        }
    }

    /// 从库里现有的节点和边重建 `node_ids` 与 `seen_edges`。
    ///
    /// 两者是进程内派生状态，重启后必须复原，否则重开的库会把已存在的边
    /// 当成新事实重复写入，也无法按字符串 id 定位节点。
    fn rebuild_lookups(&mut self) -> GraphStoreResult<()> {
        let mut by_node_id: HashMap<NodeId, String> = HashMap::new();
        for node in self.db.iter_nodes() {
            let id =
                read_text(|key| node.get_property(key).cloned(), PROP_ID).ok_or_else(|| {
                    GraphStoreError::Corrupted {
                        reason: format!("grafeo 节点 {:?} 缺少 {PROP_ID} 属性", node.id),
                    }
                })?;
            by_node_id.insert(node.id, id.clone());
            self.node_ids.insert(id, node.id);
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

    /// 按方向组装邻居视图。`outgoing` 为真表示出边（对端是 `to`）。
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
        Ok(self.db.node_count())
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
        self.node_ids.insert(node.id, created);
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
