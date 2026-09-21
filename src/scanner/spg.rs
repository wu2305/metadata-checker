use super::{add_edge_with_meta, add_identified_node, add_node, resolve_reference_path};
use crate::graph::{EdgeType, NodeType};
use crate::graph_store::GraphWriteStore;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, Default)]
struct ComponentContext {
    json_path: String,
    parent_id: Option<String>,
    source: Option<String>,
    data_set: Option<String>,
    inherited_data_context_component_id: Option<String>,
    inherited_data_context_json_path: Option<String>,
    inherited_data_context_source: Option<String>,
    inherited_data_context_data_set: Option<String>,
}

/// 页面局部身份开关（M59-1 交接约束：身份编码与 schema 版本切换共同发布）。
///
/// 旧 schema 的库不得写入页面局部 id，也不得新旧形态混写；只有启用来源账本的
/// 索引路径才允许写入 `<kind>:<PAGE>|<local>`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageIdentityMode {
    /// 旧 schema：模型/字段保持全局 id（现状行为，不动）。
    LegacyGlobal,
    /// 来源账本 schema：页面局部模型/字段写入带页面段的 id。
    OwnershipPageLocal,
}

/// 页面局部身份作用域：把**页面局部实体**与**物理实体**在写入前分开。
///
/// M59-2 根因：局部 source 与它引用的物理表同名时，若先按旧全局 id 建临时图，
/// 两类实体会塌成同一个节点（后写覆盖前写的 `path`/`meta`），身份信息在任何
/// 转换之前就已经丢失，事后只能靠猜测恢复。本作用域让 model/field 在建节点时
/// 就带上页面段，物理实体保持全局，二者不再共享 id。
struct PageScope<'a> {
    /// 归一化后的页面路径（id 的页面段）。
    page: &'a str,
    /// 页面局部模型的局部名（source id）集合。
    local_models: &'a HashSet<String>,
    /// 是否启用页面局部身份；false 时所有 model/field 保持全局。
    enabled: bool,
}

impl<'a> PageScope<'a> {
    fn is_local(&self, model: &str) -> bool {
        self.enabled && self.local_models.contains(model)
    }

    /// 模型 id：局部 source 带页面段，其余走全局身份。
    fn model_id(&self, model: &str) -> Result<String> {
        if self.is_local(model) {
            crate::graph_identity::page_local_node_id(
                crate::graph_identity::NodeIdKind::Model,
                self.page,
                model,
            )
            .map_err(|error| anyhow::anyhow!("invalid page-local model id {model}: {error}"))
        } else {
            crate::graph_identity::global_node_id(
                crate::graph_identity::NodeIdKind::Model,
                model,
            )
            .map_err(|error| anyhow::anyhow!("invalid global model id {model}: {error}"))
        }
    }

    /// 物理模型 id：物理表不归属任何页面，永远走全局身份。
    ///
    /// 即使物理表名与某个页面局部 source 同名，也不能被作用域改写——那正是
    /// 「局部 source 与物理表同名」场景里必须区分开的两个实体。
    fn physical_model_id(&self, model: &str) -> Result<String> {
        crate::graph_identity::global_node_id(crate::graph_identity::NodeIdKind::Model, model)
            .map_err(|error| anyhow::anyhow!("invalid physical model id {model}: {error}"))
    }

    /// 字段 id：归属判断沿用其模型（`model.field` 的 model 段）。
    fn field_id(&self, model: &str, field: &str) -> Result<String> {
        let local = format!("{model}.{field}");
        if self.is_local(model) {
            crate::graph_identity::page_local_node_id(
                crate::graph_identity::NodeIdKind::Field,
                self.page,
                &local,
            )
            .map_err(|error| anyhow::anyhow!("invalid page-local field id {local}: {error}"))
        } else {
            crate::graph_identity::global_node_id(crate::graph_identity::NodeIdKind::Field, &local)
                .map_err(|error| anyhow::anyhow!("invalid global field id {local}: {error}"))
        }
    }

    /// 物理字段 id：与 [`Self::physical_model_id`] 同口径，永远全局。
    fn physical_field_id(&self, model: &str, field: &str) -> Result<String> {
        crate::graph_identity::global_node_id(
            crate::graph_identity::NodeIdKind::Field,
            &format!("{model}.{field}"),
        )
        .map_err(|error| anyhow::anyhow!("invalid physical field id {model}.{field}: {error}"))
    }

    /// 写入已完成身份构造的节点（含页面局部 id，不再经全局 id 转换）。
    fn add_node(
        &self,
        graph: &mut dyn GraphWriteStore,
        id: &str,
        node_type: NodeType,
        path: String,
        name: String,
        meta: Option<serde_json::Value>,
    ) -> Result<()> {
        add_identified_node(graph, id.to_string(), node_type, path, name, meta)
    }
}

fn json_scalar_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Array(arr) => arr
            .iter()
            .find_map(|item| item.as_str().map(|s| s.to_string())),
        _ => None,
    }
}

fn collect_component_contexts(
    value: &serde_json::Value,
) -> std::collections::HashMap<String, ComponentContext> {
    let mut contexts = std::collections::HashMap::new();
    // 解析路径只消费组件上下文；扫描诊断由 scan_raw_counts 对同一份原始 JSON
    // 单独遍历采集。两条路径共用同一个带形态感知递归的内层遍历函数，规则不漂移。
    let mut diags = ScanDiagnostics::default();
    if let Some(canvas) = value.get("canvas") {
        collect_component_contexts_inner_with_context_and_diagnostics(
            canvas,
            "canvas",
            None,
            None,
            &mut contexts,
            &mut diags,
        );
    }
    contexts
}

/// 对原始 SPG JSON 采集扫描诊断计数（未识别容器键 / 重复组件 id）。
#[cfg(any(test, feature = "cli-local"))]
pub(crate) fn scan_raw_counts(value: &serde_json::Value) -> ScanDiagnostics {
    let mut diags = ScanDiagnostics::default();
    if let Some(canvas) = value.get("canvas") {
        collect_component_contexts_inner_with_context_and_diagnostics(
            canvas,
            "canvas",
            None,
            None,
            &mut std::collections::HashMap::new(),
            &mut diags,
        );
    }
    diags
}

/// 对原始 SPG JSON 采集扫描诊断（未识别容器键 / 重复组件 id）。
///
/// 生产路径由 indexer 按文件采集计数并持久化到 redb（`per_file_scan_diagnostic_entry` /
/// `merge_scanner_diagnostic_entries`），集成测试亦直接使用本函数。
#[cfg(any(test, feature = "cli-local"))]
pub fn scan_raw_diagnostics(value: &serde_json::Value) -> Vec<crate::output::Diagnostic> {
    scan_raw_counts(value).to_diagnostics()
}

/// 扫描阶段的轻量诊断计数（PR1 落地，未识别容器键/重复组件 id）。
#[derive(Debug, Default, Clone)]
pub(crate) struct ScanDiagnostics {
    pub(crate) unrecognized_container_key: usize,
    pub(crate) duplicate_component_id: usize,
    /// M59-B3：本文件解析失败、未进候选图（图内容陈旧）。
    /// 不由 `scan_raw_counts` 产出——扫描能跑到这里说明已经解析成功了；
    /// 由 indexer 在解析阶段失败时直接构造。
    pub(crate) parse_failed: usize,
    pub(crate) sample_unrecognized_location: Option<crate::output::Location>,
    pub(crate) sample_duplicate_location: Option<crate::output::Location>,
    pub(crate) sample_parse_failed_location: Option<crate::output::Location>,
    /// 解析失败的原因（serde/UTF-8 报错原文），随诊断 message 透出
    pub(crate) parse_failed_reason: Option<String>,
}

impl ScanDiagnostics {
    /// 合并另一份计数（跨文件聚合；样本位置保留首个非空）。
    #[cfg(any(test, feature = "cli-local"))]
    pub(crate) fn merge(&mut self, other: &ScanDiagnostics) {
        self.unrecognized_container_key += other.unrecognized_container_key;
        self.duplicate_component_id += other.duplicate_component_id;
        self.parse_failed += other.parse_failed;
        if self.sample_unrecognized_location.is_none() {
            self.sample_unrecognized_location = other.sample_unrecognized_location.clone();
        }
        if self.sample_duplicate_location.is_none() {
            self.sample_duplicate_location = other.sample_duplicate_location.clone();
        }
        if self.sample_parse_failed_location.is_none() {
            self.sample_parse_failed_location = other.sample_parse_failed_location.clone();
            self.parse_failed_reason = other.parse_failed_reason.clone();
        }
    }

    pub(crate) fn to_diagnostics(&self) -> Vec<crate::output::Diagnostic> {
        let mut out = Vec::new();
        if self.unrecognized_container_key > 0 {
            let loc = self
                .sample_unrecognized_location
                .clone()
                .unwrap_or_default();
            out.push(crate::diagnostics::envelope_diagnostic(
                crate::diagnostics::CODE_SCANNER_UNRECOGNIZED_CONTAINER_KEY,
                self.unrecognized_container_key,
                loc,
                format!(
                    "Scanner encountered {} unrecognized container keys",
                    self.unrecognized_container_key
                ),
            ));
        }
        if self.duplicate_component_id > 0 {
            let loc = self.sample_duplicate_location.clone().unwrap_or_default();
            out.push(crate::diagnostics::envelope_diagnostic(
                crate::diagnostics::CODE_SCANNER_DUPLICATE_COMPONENT_ID,
                self.duplicate_component_id,
                loc,
                format!(
                    "Scanner encountered {} duplicate component ids",
                    self.duplicate_component_id
                ),
            ));
        }
        if self.parse_failed > 0 {
            let loc = self
                .sample_parse_failed_location
                .clone()
                .unwrap_or_default();
            // 消息里点名「图内容为上一次成功解析的结果」——读到这条的人需要知道
            // 这不是「没有数据」，而是「数据是旧的」。
            let reason = self
                .parse_failed_reason
                .clone()
                .unwrap_or_else(|| "unknown parse error".to_string());
            out.push(crate::diagnostics::envelope_diagnostic(
                crate::diagnostics::CODE_SCANNER_FILE_PARSE_FAILED,
                self.parse_failed,
                loc,
                format!(
                    "{} source file(s) failed to parse and were skipped; \
                     their graph content is the last successfully parsed version (stale). \
                     First failure: {}",
                    self.parse_failed, reason
                ),
            ));
        }
        out
    }
}

fn collect_component_contexts_inner_with_context_and_diagnostics(
    node: &serde_json::Value,
    json_path: &str,
    parent_id: Option<String>,
    inherited_context: Option<ComponentContext>,
    contexts: &mut std::collections::HashMap<String, ComponentContext>,
    diagnostics: &mut ScanDiagnostics,
) {
    let current_id = node
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // M58.3 复核返修 P1-7：组件身份判定对齐 superpage 提取侧口径——id 与 type
    // 同时为非空字符串才是组件。id 有、type 无/空的对象不是组件：不注册组件
    // 上下文（否则 pass2 会向不存在的 comp 节点建 comp→comp Contains，被存储层
    // 静默丢弃，且 SpgComponent.parent_id 与 ctx.parent_id 两个事实源互相矛盾），
    // 其子组件的 parent_id 透传祖父（与 extract_components 的透传一致）；同时按
    // 「对象形态未识别」计入 SCANNER_UNRECOGNIZED_CONTAINER_KEY 安全网（spec 中
    // 该 code 的定义覆盖容器子键/对象形态未识别），不允许继续静默
    let has_type = node
        .get("type")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    let is_component = current_id.is_some() && has_type;
    if current_id.is_some() && !has_type {
        diagnostics.unrecognized_container_key += 1;
        if diagnostics.sample_unrecognized_location.is_none() {
            diagnostics.sample_unrecognized_location = Some(crate::output::Location {
                source_file: None,
                node_id: current_id.clone(),
                json_path: Some(json_path.to_string()),
            });
        }
    }

    let source = node.get("source").and_then(json_scalar_to_string);
    let data_set = node.get("dataSet").and_then(json_scalar_to_string);
    let own_context = if source.is_some() || data_set.is_some() {
        current_id.as_ref().map(|id| ComponentContext {
            json_path: json_path.to_string(),
            parent_id: parent_id.clone(),
            source: source.clone(),
            data_set: data_set.clone(),
            inherited_data_context_component_id: Some(id.clone()),
            inherited_data_context_json_path: Some(json_path.to_string()),
            inherited_data_context_source: source.clone(),
            inherited_data_context_data_set: data_set.clone(),
        })
    } else {
        None
    };
    let active_context = own_context.or(inherited_context);

    if is_component && let Some(id) = &current_id {
        if contexts.contains_key(id) {
            diagnostics.duplicate_component_id += 1;
            if diagnostics.sample_duplicate_location.is_none() {
                diagnostics.sample_duplicate_location = Some(crate::output::Location {
                    source_file: None,
                    node_id: Some(id.clone()),
                    json_path: Some(json_path.to_string()),
                });
            }
        }
        contexts.insert(
            id.clone(),
            ComponentContext {
                json_path: json_path.to_string(),
                parent_id: parent_id.clone(),
                source: source.clone(),
                data_set: data_set.clone(),
                inherited_data_context_component_id: active_context
                    .as_ref()
                    .and_then(|ctx| ctx.inherited_data_context_component_id.clone()),
                inherited_data_context_json_path: active_context
                    .as_ref()
                    .and_then(|ctx| ctx.inherited_data_context_json_path.clone()),
                inherited_data_context_source: active_context
                    .as_ref()
                    .and_then(|ctx| ctx.inherited_data_context_source.clone()),
                inherited_data_context_data_set: active_context
                    .as_ref()
                    .and_then(|ctx| ctx.inherited_data_context_data_set.clone()),
            },
        );
    }

    // 组件身份成立时子组件的父为当前节点；非组件节点（无 id，或 id 有 type 无）
    // 透传祖父 parent_id——与 superpage 提取侧 extract_components 对 id/type
    // 缺失节点的 parent_id 透传保持同口径，消除两个 parent 事实源的矛盾
    let child_parent = if is_component {
        current_id.clone()
    } else {
        parent_id.clone()
    };
    for child_key in ["components", "panels", "steps", "comps"] {
        if let Some(children) = node.get(child_key).and_then(|v| v.as_array()) {
            for (idx, child) in children.iter().enumerate() {
                let child_path = format!("{}.{}[{}]", json_path, child_key, idx);
                collect_component_contexts_inner_with_context_and_diagnostics(
                    child,
                    &child_path,
                    child_parent.clone(),
                    active_context.clone(),
                    contexts,
                    diagnostics,
                );
            }
        }
    }
    // 白名单路径之外的未知子键（M58.3 F1 形态感知递归）：
    // - 排除列表键：已知非组件，不递归也不计数（排除优先于形态判定）；
    // - 形态吻合（非空数组且每个元素都带字符串 id+type）：子组件数组，递归遍历，不计数；
    // - 混合形态（含对象但不满足组件形态）：整体判非组件，计入 SCANNER_UNRECOGNIZED_CONTAINER_KEY。
    if let Some(obj) = node.as_object() {
        for (key, value) in obj {
            if crate::superpage::NON_COMPONENT_CONTAINER_KEYS.contains(&key.as_str())
                || ["components", "panels", "steps", "comps", "id", "type"].contains(&key.as_str())
            {
                continue;
            }
            let Some(arr) = value.as_array() else {
                continue;
            };
            if arr.is_empty() {
                continue;
            }
            if crate::superpage::is_component_array(value) {
                for (idx, child) in arr.iter().enumerate() {
                    let child_path = format!("{}.{}[{}]", json_path, key, idx);
                    collect_component_contexts_inner_with_context_and_diagnostics(
                        child,
                        &child_path,
                        child_parent.clone(),
                        active_context.clone(),
                        contexts,
                        diagnostics,
                    );
                }
                continue;
            }
            let has_object = arr.iter().any(|item| item.is_object());
            if has_object {
                diagnostics.unrecognized_container_key += 1;
                if diagnostics.sample_unrecognized_location.is_none() {
                    diagnostics.sample_unrecognized_location = Some(crate::output::Location {
                        source_file: None,
                        node_id: current_id.clone(),
                        json_path: Some(format!("{}.{}", json_path, key)),
                    });
                }
            }
        }
    }
}

/// 识别 `${FIELD}` 这种由数据容器上下文补全的裸字段引用
fn extract_single_bare_symbol(raw_expr: &str) -> Option<String> {
    let trimmed = raw_expr.trim();
    let inner = trimmed
        .strip_prefix("${")
        .and_then(|s| s.strip_suffix('}'))?
        .trim();
    if inner.is_empty() || inner.contains('.') {
        return None;
    }
    if inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some(inner.to_string())
    } else {
        None
    }
}

/// 确保 model 和 field 节点存在，并建立 Contains 关系。
/// 返回 (model_id, field_id)。
/// 确保 `model:{model}` 节点存在；字段名非空时一并确保 `field:{model}.{field}`
/// 节点与 model→field `Contains` 边。
///
/// M58.3 相邻缺口修复：字段名为空时（裸 `${modelN}` 被 `expr_ast` 判为
/// `ModelField(modelN, "")`）**不再**造 `field:modelN.` 尾点节点——那是个没有字段
/// 语义的垃圾节点，只会在 dataflow/lineage 里冒充一个字段。返回值第二项因此
/// 改为 `Option`：`None` 表示本次只有模型级语义，调用方不应建字段级边。
fn ensure_model_field(
    graph: &mut dyn GraphWriteStore,
    model: &str,
    field: &str,
    model_path: &str,
    field_meta: Option<serde_json::Value>,
) -> Result<(String, Option<String>)> {
    // 旧全局口径：所有 model/field 都是全局 id（无页面段），保持既有行为。
    ensure_model_field_with_scope(
        graph,
        model,
        field,
        model_path,
        field_meta,
        ModelIdentity::Global,
        None,
    )
}

/// 身份归属：决定一个 model/field 名字解析成页面局部 id 还是全局 id。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelIdentity {
    /// 页面局部实体（页面 sources 里声明的 source）。
    Local,
    /// 物理实体（物理表/物理字段），不归属任何页面。
    Global,
}

/// 确保 model 与 field 节点存在并建立 Contains 关系，身份由 `scope` 决定。
///
/// `identity == Global` 时**强制**全局 id，即便该名字同时是某个页面局部
/// source——「一个 source 名碰撞另一个 source 的物理表名」的场景靠这条区分：
/// 物理表 `beta` 与局部 source `beta` 是两个实体，不能塌成一个节点。
fn ensure_model_field_with_scope(
    graph: &mut dyn GraphWriteStore,
    model: &str,
    field: &str,
    model_path: &str,
    field_meta: Option<serde_json::Value>,
    identity: ModelIdentity,
    scope: Option<&PageScope<'_>>,
) -> Result<(String, Option<String>)> {
    let model_id = resolve_model_identity(model, identity, scope)?;
    add_identified_node(
        graph,
        model_id.clone(),
        NodeType::Model,
        model_path.to_string(),
        model.to_string(),
        None,
    )?;
    if field.is_empty() {
        return Ok((model_id, None));
    }
    let field_id = resolve_field_identity(model, field, identity, scope)?;
    add_identified_node(
        graph,
        field_id.clone(),
        NodeType::Field,
        model_path.to_string(),
        field.to_string(),
        field_meta,
    )?;
    add_edge_with_meta(graph, &model_id, &field_id, EdgeType::Contains, None, None)?;
    Ok((model_id, Some(field_id)))
}

/// 模型名 → 节点 id（按归属决策，不是按名字猜测）。
fn resolve_model_identity(
    model: &str,
    identity: ModelIdentity,
    scope: Option<&PageScope<'_>>,
) -> Result<String> {
    match (scope, identity) {
        (Some(scope), ModelIdentity::Global) => scope.physical_model_id(model),
        (Some(scope), ModelIdentity::Local) => scope.model_id(model),
        (None, _) => crate::graph_identity::global_node_id(
            crate::graph_identity::NodeIdKind::Model,
            model,
        )
        .map_err(|error| anyhow::anyhow!("invalid model id {model}: {error}")),
    }
}

/// 字段名 → 节点 id（归属与所属模型一致）。
fn resolve_field_identity(
    model: &str,
    field: &str,
    identity: ModelIdentity,
    scope: Option<&PageScope<'_>>,
) -> Result<String> {
    match (scope, identity) {
        (Some(scope), ModelIdentity::Global) => scope.physical_field_id(model, field),
        (Some(scope), ModelIdentity::Local) => scope.field_id(model, field),
        (None, _) => crate::graph_identity::global_node_id(
            crate::graph_identity::NodeIdKind::Field,
            &format!("{model}.{field}"),
        )
        .map_err(|error| anyhow::anyhow!("invalid field id {model}.{field}: {error}")),
    }
}

/// 模型字段引用的 field_path 文案：字段名为空时只写模型名，避免 `modelN.` 尾点。
fn model_field_path(model: &str, field: &str) -> String {
    if field.is_empty() {
        model.to_string()
    } else {
        format!("{}.{}", model, field)
    }
}

/// 从模型路径中提取物理表名（去除路径前缀和 .tbl 后缀）。
/// - "$DATA:/主数据/fact_qwSidebar.tbl" -> "fact_qwSidebar"
/// - "data/table1.tbl" -> "table1"
/// - "model1.tbl" -> "model1"
fn resolve_physical_table_name(model_path: &str) -> Option<String> {
    let path = model_path.trim_end_matches('/');
    let stem = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())?;
    if stem.is_empty() {
        return None;
    }
    Some(stem)
}

/// 添加从 from_id 读取 model.field 的关系边。
fn add_model_read(
    graph: &mut dyn GraphWriteStore,
    from_id: &str,
    model: &str,
    field: &str,
    model_path: &str,
    edge_meta: serde_json::Value,
    edge_type: EdgeType,
) -> Result<()> {
    add_model_read_with_scope(
        graph,
        from_id,
        model,
        field,
        model_path,
        edge_meta,
        edge_type,
        None,
    )
}

fn add_model_read_with_scope(
    graph: &mut dyn GraphWriteStore,
    from_id: &str,
    model: &str,
    field: &str,
    model_path: &str,
    edge_meta: serde_json::Value,
    edge_type: EdgeType,
    scope: Option<&PageScope<'_>>,
) -> Result<()> {
    let (model_id, field_id) = ensure_model_field_with_scope(
        graph,
        model,
        field,
        model_path,
        None,
        ModelIdentity::Local,
        scope,
    )?;
    add_edge_with_meta(
        graph,
        from_id,
        &model_id,
        edge_type.clone(),
        Some(model_field_path(model, field)),
        Some(edge_meta.clone()),
    )?;
    // 字段级读取边：组件直接指向字段节点（字段名为空时无字段节点，只保留模型级边）
    if let Some(field_id) = &field_id {
        add_edge_with_meta(
            graph,
            from_id,
            field_id,
            edge_type.clone(),
            Some(model_field_path(model, field)),
            Some(edge_meta.clone()),
        )?;
    }

    // 同时创建到物理表的读取边（如果局部模型 ID 与物理表名不同）
    if let Some(physical_name) = resolve_physical_table_name(model_path) {
        // 物理表身份恒为全局：即便与局部 source 同名也是另一个实体，必须各写各的节点。
        if model_id != resolve_model_identity(&physical_name, ModelIdentity::Global, scope)? {
            let (phy_model_id, phy_field_id) = ensure_model_field_with_scope(
                graph,
                &physical_name,
                field,
                model_path,
                None,
                ModelIdentity::Global,
                scope,
            )?;
            add_edge_with_meta(
                graph,
                from_id,
                &phy_model_id,
                edge_type.clone(),
                Some(model_field_path(&physical_name, field)),
                Some(edge_meta.clone()),
            )?;
            // 字段级读取边：组件直接指向物理字段节点（字段名为空时无字段节点）
            if let Some(phy_field_id) = &phy_field_id {
                add_edge_with_meta(
                    graph,
                    from_id,
                    phy_field_id,
                    edge_type.clone(),
                    Some(model_field_path(&physical_name, field)),
                    Some(edge_meta.clone()),
                )?;
                // 局部模型字段到物理表字段的别名映射
                if let Some(field_id) = &field_id {
                    add_edge_with_meta(
                        graph,
                        field_id,
                        phy_field_id,
                        EdgeType::FieldAlias,
                        Some(model_field_path(model, field)),
                        None,
                    )?;
                }
            }
        }
    }
    Ok(())
}

/// 添加从 from_id 写入 model.field 的关系边。
fn add_model_write(
    graph: &mut dyn GraphWriteStore,
    from_id: &str,
    model: &str,
    field: &str,
    model_path: &str,
    edge_type: EdgeType,
    edge_meta: serde_json::Value,
) -> Result<()> {
    add_model_write_with_scope(
        graph,
        from_id,
        model,
        field,
        model_path,
        edge_type,
        edge_meta,
        None,
    )
}

fn add_model_write_with_scope(
    graph: &mut dyn GraphWriteStore,
    from_id: &str,
    model: &str,
    field: &str,
    model_path: &str,
    edge_type: EdgeType,
    edge_meta: serde_json::Value,
    scope: Option<&PageScope<'_>>,
) -> Result<()> {
    let (model_id, field_id) = ensure_model_field_with_scope(
        graph,
        model,
        field,
        model_path,
        None,
        ModelIdentity::Local,
        scope,
    )?;
    add_edge_with_meta(
        graph,
        from_id,
        &model_id,
        edge_type.clone(),
        Some(model_field_path(model, field)),
        Some(edge_meta.clone()),
    )?;
    // 字段级写入边：action 直接指向字段节点（字段名为空时无字段节点，只保留模型级边）
    if let Some(field_id) = &field_id {
        add_edge_with_meta(
            graph,
            from_id,
            field_id,
            EdgeType::FieldWrite,
            Some(model_field_path(model, field)),
            Some(edge_meta.clone()),
        )?;
    }

    // 同时创建到物理表的写入边（如果局部模型 ID 与物理表名不同）
    if let Some(physical_name) = resolve_physical_table_name(model_path) {
        if model_id != resolve_model_identity(&physical_name, ModelIdentity::Global, scope)? {
            let (phy_model_id, phy_field_id) = ensure_model_field_with_scope(
                graph,
                &physical_name,
                field,
                model_path,
                None,
                ModelIdentity::Global,
                scope,
            )?;
            add_edge_with_meta(
                graph,
                from_id,
                &phy_model_id,
                edge_type.clone(),
                Some(model_field_path(&physical_name, field)),
                Some(edge_meta.clone()),
            )?;
            // 字段级写入边：action 直接指向物理字段节点（字段名为空时无字段节点）
            if let Some(phy_field_id) = &phy_field_id {
                add_edge_with_meta(
                    graph,
                    from_id,
                    phy_field_id,
                    EdgeType::FieldWrite,
                    Some(model_field_path(&physical_name, field)),
                    Some(edge_meta.clone()),
                )?;
                // 局部模型字段到物理表字段的别名映射
                if let Some(field_id) = &field_id {
                    add_edge_with_meta(
                        graph,
                        field_id,
                        phy_field_id,
                        EdgeType::FieldAlias,
                        Some(model_field_path(model, field)),
                        None,
                    )?;
                }
            }
        }
    }
    Ok(())
}

/// 兼容旧签名：按旧全局身份建图（无页面局部 id），保持既有测试与 CLI 行为。
pub fn process_spg_file_from_value(
    graph: &mut dyn GraphWriteStore,
    rel_path: &str,
    raw_value: serde_json::Value,
) -> Result<Vec<String>> {
    process_spg_file_from_value_with_identity(
        graph,
        rel_path,
        raw_value,
        PageIdentityMode::LegacyGlobal,
    )
}

/// 按指定身份模式处理一个 SPG 文件。
///
/// M59-2：启用页面局部身份时，模型/字段**在建节点之前**就带上页面段，与物理
/// 表区分开。身份必须早于任何有损合并（同一 id 的 upsert 覆盖 path/meta），
/// 否则局部 source 与同名物理表会塌成一个节点，之后再无法分辨。
pub fn process_spg_file_from_value_with_identity(
    graph: &mut dyn GraphWriteStore,
    rel_path: &str,
    raw_value: serde_json::Value,
    identity_mode: PageIdentityMode,
) -> Result<Vec<String>> {
    let component_contexts = collect_component_contexts(&raw_value);
    let meta = crate::superpage::parse_superpage_from_value(raw_value)?;
    let mut node_ids = std::collections::HashSet::new();

    // Build model_id -> path mapping from page sources for accurate model references
    let source_path_map: std::collections::HashMap<String, String> = meta
        .sources
        .iter()
        .filter_map(|s| {
            let path = s.path.clone()?;
            Some((s.id.clone(), path))
        })
        .collect();

    let page_name = Path::new(rel_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // M59-2：在建任何 model/field 之前先定作用域。局部模型集合 = 页面 sources
    // 里声明的 source id（dataflow/dwtable/filter 等页面局部实体）；物理表来自
    // source.path 的文件 stem，永远全局——即便与某个 source id 同名。
    let page_key = crate::graph_identity::normalize_project_path(&rel_path.replace('\\', "/"))
        .with_context(|| format!("invalid page path for {rel_path}"))?;
    let local_models: HashSet<String> = meta
        .sources
        .iter()
        .map(|source| source.id.clone())
        .collect();
    let scope = PageScope {
        page: &page_key,
        local_models: &local_models,
        enabled: identity_mode == PageIdentityMode::OwnershipPageLocal,
    };

    // Use normalized relative path for unique page_id to avoid collisions
    let page_id = format!("page:{}", rel_path.replace("\\", "/"));
    add_node(
        graph,
        page_id.clone(),
        NodeType::Page,
        rel_path.to_string(),
        page_name.clone(),
        None,
    )?;
    node_ids.insert(page_id.clone());

    // Process embedded DataFlow models in sources
    for source in &meta.sources {
        if source.model_type.as_deref() != Some("dataflow") {
            continue;
        }
        let Some(content) = &source.content else {
            continue;
        };
        let model_name = &source.id;
        // 页面局部实体：身份在建节点前确定，不能先按全局 id 写再事后转换
        let model_id = scope.model_id(model_name)?;

        scope.add_node(
            graph,
            &model_id,
            NodeType::Model,
            rel_path.to_string(),
            model_name.clone(),
            Some(serde_json::json!({"modelType": "DataFlow", "embeddedIn": page_name})),
        )?;
        if !node_ids.contains(&model_id) {
            node_ids.insert(model_id.clone());
        }

        // Extract dimensions as fields
        if let Some(dimensions) = content.get("dimensions").and_then(|d| d.as_array()) {
            for dim in dimensions {
                if let Some(name) = dim.get("name").and_then(|n| n.as_str()) {
                    let field_id = scope.field_id(model_name, name)?;
                    scope.add_node(
                        graph,
                        &field_id,
                        NodeType::Field,
                        rel_path.to_string(),
                        name.to_string(),
                        Some(dim.clone()),
                    )?;
                    if !node_ids.contains(&field_id) {
                        node_ids.insert(field_id.clone());
                    }
                    add_edge_with_meta(
                        graph,
                        &model_id,
                        &field_id,
                        EdgeType::Contains,
                        None,
                        None,
                    )?;
                }
            }
        }

        // Extract dataFlow input sources (external tables referenced by ModelTable nodes)
        if let Some(nodes) = content
            .get("dataFlow")
            .and_then(|d| d.get("nodes"))
            .and_then(|n| n.as_object())
        {
            for (_, node) in nodes {
                if let Some(module_table_path) =
                    node.get("moduleTablePath").and_then(|p| p.as_str())
                {
                    let ref_model = Path::new(module_table_path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| module_table_path.to_string());
                    // 物理表：恒全局，不得被页面作用域改写
                    let ref_model_id = scope.physical_model_id(&ref_model)?;
                    scope.add_node(
                        graph,
                        &ref_model_id,
                        NodeType::Model,
                        module_table_path.to_string(),
                        ref_model.clone(),
                        None,
                    )?;
                    add_edge_with_meta(
                        graph,
                        &model_id,
                        &ref_model_id,
                        EdgeType::DataflowInput,
                        Some(module_table_path.to_string()),
                        None,
                    )?;
                }
            }
        }
    }

    // Build a map of component id -> submitField for action processing
    let mut submit_field_map: HashMap<String, String> = HashMap::new();
    for comp in &meta.components {
        // Skip components explicitly marked as not submitting data
        if comp.submit_data == Some(false) {
            continue;
        }
        if let Some(sf) = comp.properties.get("submitField") {
            submit_field_map.insert(comp.id.clone(), sf.clone());
        }
    }

    let mut expr_map: std::collections::HashMap<&str, Vec<&crate::superpage::ComponentExpr>> =
        std::collections::HashMap::new();
    for expr in &meta.expressions {
        expr_map.entry(&expr.component_id).or_default().push(expr);
    }
    let component_id_set: std::collections::HashSet<&str> =
        meta.components.iter().map(|c| c.id.as_str()).collect();
    // 页面 param 集合（M58.3 复核返修 P1-6）：裸 `${paramN}` 是参数引用而非数据
    // 上下文字段，不得落入下方「裸字段 + 继承 dataSet」的 Reads 建边分支
    let param_id_set: std::collections::HashSet<&str> =
        meta.params.iter().map(|p| p.id.as_str()).collect();
    // 组件处理拆成两遍：第一遍先为 meta.components 中所有组件注册节点，
    // 第二遍再建边。原因：GraphWriteStore 在边任一端点节点尚不存在时会静默
    // 丢弃该边（见 graph_redb.rs / memory_graph_store.rs 的 add_edge 实现）；
    // 若表达式引用的目标组件在数组中排在引用方之后（前向引用），单遍顺序
    // 处理会让 comp→comp DependsOn / Contains 边被无声丢失。先全量注册节点
    // 可消除这一顺序依赖。
    //
    // 第一遍：注册全部组件节点（节点 meta 内容与拆分前完全一致）。
    for comp in &meta.components {
        let comp_id = format!("comp:{}|{}", rel_path.replace("\\", "/"), comp.id);
        let ctx = component_contexts.get(&comp.id);
        let mut comp_meta = serde_json::json!({
            "component_type": comp.component_type,
            "properties": comp.properties,
        });
        if let Some(ctx) = ctx
            && let Some(obj) = comp_meta.as_object_mut()
        {
            obj.insert("json_path".to_string(), serde_json::json!(ctx.json_path));
            if let Some(parent_id) = &ctx.parent_id {
                obj.insert("parent_id".to_string(), serde_json::json!(parent_id));
            }
            if let Some(source) = &ctx.source {
                obj.insert("source".to_string(), serde_json::json!(source));
            }
            if let Some(data_set) = &ctx.data_set {
                obj.insert("dataSet".to_string(), serde_json::json!(data_set));
            }
            if let Some(context_component_id) = &ctx.inherited_data_context_component_id {
                obj.insert(
                    "data_context_component_id".to_string(),
                    serde_json::json!(context_component_id),
                );
            }
            if let Some(context_json_path) = &ctx.inherited_data_context_json_path {
                obj.insert(
                    "data_context_json_path".to_string(),
                    serde_json::json!(context_json_path),
                );
            }
            if let Some(context_source) = &ctx.inherited_data_context_source {
                obj.insert(
                    "data_context_source".to_string(),
                    serde_json::json!(context_source),
                );
            }
            if let Some(context_data_set) = &ctx.inherited_data_context_data_set {
                obj.insert(
                    "data_context_dataSet".to_string(),
                    serde_json::json!(context_data_set),
                );
            }
        }
        add_node(
            graph,
            comp_id.clone(),
            NodeType::Component,
            rel_path.to_string(),
            comp.id.clone(),
            Some(comp_meta),
        )?;
        node_ids.insert(comp_id.clone());
    }

    // 第二遍：建 page→comp / comp→comp Contains 边，并处理表达式引用建边。
    // 此时所有组件节点均已注册，前向引用的目标节点必定存在，不会再丢边。
    for comp in &meta.components {
        let comp_id = format!("comp:{}|{}", rel_path.replace("\\", "/"), comp.id);
        let ctx = component_contexts.get(&comp.id);
        add_edge_with_meta(graph, &page_id, &comp_id, EdgeType::Contains, None, None)?;
        // comp→comp Contains：父组件取自组件上下文（说明 A：每个嵌套组件恰好一个
        // Component 父；page→comp 边保留）。白名单嵌套与形态感知递归新发现的组件
        // 走同一条建边路径。
        if let Some(ctx) = ctx
            && let Some(parent_id) = &ctx.parent_id
        {
            let parent_comp_id = format!("comp:{}|{}", rel_path.replace("\\", "/"), parent_id);
            add_edge_with_meta(
                graph,
                &parent_comp_id,
                &comp_id,
                EdgeType::Contains,
                None,
                None,
            )?;
        }

        // Process expressions (reads)
        if let Some(exprs) = expr_map.get(comp.id.as_str()) {
            for expr in exprs {
                if expr.field == "value"
                    && let Some(bare_symbol) = extract_single_bare_symbol(&expr.raw_expr)
                    && !source_path_map.contains_key(bare_symbol.as_str())
                    && !component_id_set.contains(bare_symbol.as_str())
                    && !param_id_set.contains(bare_symbol.as_str())
                    && let Some(ctx) = ctx
                    && let Some(data_set) = &ctx.inherited_data_context_data_set
                {
                    let model_path = source_path_map
                        .get(data_set.as_str())
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| format!("{}.tbl", data_set));
                    let edge_meta = serde_json::json!({
                        "reason": format!(
                            "Component '{}' reads bare field '{}' from inherited dataSet '{}'",
                            comp.id, bare_symbol, data_set
                        ),
                        "actor_kind": "component",
                        "actor_id": comp.id,
                        "operation": "Reads",
                        "source_expr": expr.raw_expr,
                        "source_field": expr.field,
                        "json_path": format!("{}.{}", ctx.json_path, expr.field),
                        "bare_symbol": bare_symbol,
                        "resolution": "inherited_container_data_context",
                        "target_model": data_set,
                        "target_field": bare_symbol,
                        "target_model_path": model_path,
                        "data_context_component_id": ctx.inherited_data_context_component_id.as_deref(),
                        "data_context_json_path": ctx.inherited_data_context_json_path.as_deref(),
                        "data_context_source": ctx.inherited_data_context_source.as_deref(),
                        "data_context_dataSet": data_set,
                    });
                    add_model_read_with_scope(
                        graph,
                        &comp_id,
                        data_set,
                        &bare_symbol,
                        &model_path,
                        edge_meta,
                        EdgeType::Reads,
                            Some(&scope),
                    )?;
                }
                for ref_type in &expr.refs {
                    match ref_type {
                        crate::superpage::RefType::ModelField(model, field) => {
                            let model_path = source_path_map
                                .get(model.as_str())
                                .map(|p| p.to_string())
                                .unwrap_or_else(|| format!("{}.tbl", model));
                            let edge_meta = serde_json::json!({
                                "reason": format!("Component '{}' reads from model '{}'", comp.id, model),
                                "actor_kind": "component",
                                "actor_id": comp.id,
                                "operation": "Reads",
                                "target_model": model,
                                "target_field": field,
                                "source_expr": expr.raw_expr,
                                "source_field": expr.field,
                                "json_path": ctx.map(|c| format!("{}.{}", c.json_path, expr.field)),
                            });
                            add_model_read_with_scope(
                                graph,
                                &comp_id,
                                model,
                                field,
                                &model_path,
                                edge_meta,
                                EdgeType::Reads,
                                    Some(&scope),
                            )?;
                        }
                        crate::superpage::RefType::ComponentValue(target_id, _) => {
                            let target_comp_id =
                                format!("comp:{}|{}", rel_path.replace(r"\", "/"), target_id);
                            let edge_meta = serde_json::json!({
                                "reason": format!(
                                    "Component '{}' depends on component '{}' via field '{}'",
                                    comp.id, target_id, expr.field
                                ),
                                "actor_kind": "component",
                                "actor_id": comp.id,
                                "operation": "DependsOn",
                                "target_component": target_id,
                                "source_expr": expr.raw_expr,
                                "source_field": expr.field,
                                "source_file": rel_path,
                                "json_path": ctx.map(|context| format!("{}.{}", context.json_path, expr.field)),
                            });
                            add_edge_with_meta(
                                graph,
                                &comp_id,
                                &target_comp_id,
                                EdgeType::DependsOn,
                                Some(format!("comp:{}.value", target_id)),
                                Some(edge_meta),
                            )?;
                        }
                        crate::superpage::RefType::ComponentProperty(target_id, property) => {
                            // 说明 D：与 dependency.rs 的语义对齐——ComponentProperty 与
                            // ComponentValue 一样建 comp→comp DependsOn 边，并带属性名。
                            // 不变式：parse 层（superpage/mod.rs resolve_ref_type /
                            // resolve_ref_token）保证 ComponentProperty 的 property 永不为空；
                            // 裸 `${id}` 全组件引用已归一为 ComponentValue（见上方分支），
                            // 此处直接按 comp:id.prop 生成 field_path。
                            let target_comp_id =
                                format!("comp:{}|{}", rel_path.replace(r"\", "/"), target_id);
                            let field_path = format!("comp:{}.{}", target_id, property);
                            let edge_meta = serde_json::json!({
                                "reason": format!(
                                    "Component '{}' depends on component '{}' property '{}' via field '{}'",
                                    comp.id, target_id, property, expr.field
                                ),
                                "actor_kind": "component",
                                "actor_id": comp.id,
                                "operation": "DependsOn",
                                "target_component": target_id,
                                "target_property": property,
                                "source_expr": expr.raw_expr,
                                "source_field": expr.field,
                                "source_file": rel_path,
                                "json_path": ctx.map(|context| format!("{}.{}", context.json_path, expr.field)),
                            });
                            add_edge_with_meta(
                                graph,
                                &comp_id,
                                &target_comp_id,
                                EdgeType::DependsOn,
                                Some(field_path),
                                Some(edge_meta),
                            )?;
                        }
                        crate::superpage::RefType::Param(param_name) => {
                            let param_id =
                                format!("param:{}|{}", rel_path.replace(r"\", "/"), param_name);
                            add_node(
                                graph,
                                param_id.clone(),
                                NodeType::Field,
                                rel_path.to_string(),
                                param_name.clone(),
                                Some(serde_json::json!({"kind": "param"})),
                            )?;
                            let edge_meta = serde_json::json!({
                                "reason": format!(
                                    "Component '{}' depends on param '{}' via field '{}'",
                                    comp.id, param_name, expr.field
                                ),
                                "actor_kind": "component",
                                "actor_id": comp.id,
                                "operation": "DependsOn",
                                "target_param": param_name,
                                "source_expr": expr.raw_expr,
                                "source_field": expr.field,
                            });
                            add_edge_with_meta(
                                graph,
                                &comp_id,
                                &param_id,
                                EdgeType::DependsOn,
                                Some(format!("param:{}", param_name)),
                                Some(edge_meta),
                            )?;
                        }
                        crate::superpage::RefType::UserProperty(prop) => {
                            let user_id = format!("user:{}", prop);
                            add_node(
                                graph,
                                user_id.clone(),
                                NodeType::Field,
                                "system".to_string(),
                                format!("$user.{}", prop),
                                Some(serde_json::json!({"kind": "user_property"})),
                            )?;
                            let edge_meta = serde_json::json!({
                                "reason": format!(
                                    "Component '{}' depends on user property '$user.{}' via field '{}'",
                                    comp.id, prop, expr.field
                                ),
                                "actor_kind": "component",
                                "actor_id": comp.id,
                                "operation": "DependsOn",
                                "target_user_property": prop,
                                "source_expr": expr.raw_expr,
                                "source_field": expr.field,
                            });
                            add_edge_with_meta(
                                graph,
                                &comp_id,
                                &user_id,
                                EdgeType::DependsOn,
                                Some(format!("$user.{}", prop)),
                                Some(edge_meta),
                            )?;
                        }
                        crate::superpage::RefType::SystemVar(var_name) => {
                            let sys_id = format!("system:{}", var_name);
                            add_node(
                                graph,
                                sys_id.clone(),
                                NodeType::Field,
                                "system".to_string(),
                                format!("${}", var_name),
                                Some(serde_json::json!({"kind": "system_var"})),
                            )?;
                            let edge_meta = serde_json::json!({
                                "reason": format!(
                                    "Component '{}' depends on system variable '${}' via field '{}'",
                                    comp.id, var_name, expr.field
                                ),
                                "actor_kind": "component",
                                "actor_id": comp.id,
                                "operation": "DependsOn",
                                "target_system_var": var_name,
                                "source_expr": expr.raw_expr,
                                "source_field": expr.field,
                            });
                            add_edge_with_meta(
                                graph,
                                &comp_id,
                                &sys_id,
                                EdgeType::DependsOn,
                                Some(format!("${}", var_name)),
                                Some(edge_meta),
                            )?;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Process submitField writes (implicit writes from component bindings)
    for comp in &meta.components {
        let comp_id = format!("comp:{}|{}", rel_path.replace("\\", "/"), comp.id);

        if let Some(submit_field) = comp.properties.get("submitField") {
            let parts: Vec<&str> = submit_field.split('.').collect();
            if parts.len() >= 2 {
                let model = parts[0];
                let model_path = source_path_map
                    .get(model)
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| format!("{}.tbl", model));
                let field = parts[1..].join(".");

                let submit_meta = serde_json::json!({
                    "reason": format!("Component '{}' binds submitField to model '{}'", comp.id, model),
                    "actor_kind": "component",
                    "actor_id": comp.id,
                    "operation": "Writes",
                    "target_model": model,
                    "target_field": field,
                                    "source_expr": format!("{}.{}", model, field),
                    "source_expr": format!("{}.{}", model, field),
                });
                add_model_write_with_scope(
                    graph,
                    &comp_id,
                    model,
                    &field,
                    &model_path,
                    EdgeType::Writes,
                    submit_meta,
                        Some(&scope),
                )?;
            }
        }
    }

    // Process embedsuperpage components (page embedding)
    for comp in &meta.components {
        if comp.component_type != "embedsuperpage" {
            continue;
        }
        let comp_id = format!("comp:{}|{}", rel_path.replace("\\", "/"), comp.id);

        // Try to resolve resPath as integer index into referenceResources
        if let Some(ref res_path_str) = comp.res_path
            && let Ok(ref_idx) = res_path_str.parse::<usize>()
            && let Some(target_rel) =
                resolve_reference_path(rel_path, ref_idx, &meta.reference_resources)
        {
            let target_name = Path::new(&target_rel)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| target_rel.clone());
            let target_page_id = format!("page:{}", target_rel.replace(r"\", "/"));
            add_node(
                graph,
                target_page_id.clone(),
                NodeType::Page,
                target_rel.clone(),
                target_name.clone(),
                None,
            )?;
            add_edge_with_meta(
                graph,
                &comp_id,
                &target_page_id,
                EdgeType::EmbedsPage,
                Some(target_rel.clone()),
                None,
            )?;
        }
    }

    // Process actions (explicit writes from interactions)
    for comp in &meta.components {
        let comp_id = format!("comp:{}|{}", rel_path.replace("\\", "/"), comp.id);
        for action in &comp.actions {
            let action_id = format!(
                "action:{}|{}|{}",
                rel_path.replace("\\", "/"),
                comp.id,
                action.id
            );
            add_node(
                graph,
                action_id.clone(),
                NodeType::Action,
                rel_path.to_string(),
                format!("{}:{}", action.action_type, action.id),
                Some(serde_json::json!({
                    "waitPrev": action.wait_prev,
                    "condition": action.condition,
                    "conditionExp": action.condition_exp,
                    "triggerType": action.trigger_type,
                })),
            )?;
            node_ids.insert(action_id.clone());
            add_edge_with_meta(graph, &comp_id, &action_id, EdgeType::Triggers, None, None)?;

            match action.action_type.as_str() {
                "submitData" => {
                    let target_comps: Vec<String> = match action.submit_range.as_deref() {
                        Some("page") | Some("dataset") => {
                            // Submit ALL components with submitField
                            submit_field_map.keys().cloned().collect()
                        }
                        Some("component") | Some("dialog") => {
                            // Only submit explicitly specified components
                            action.submit_component.clone()
                        }
                        _ => {
                            // Default: if submitComponent specified, use it; otherwise collect all
                            if action.submit_component.is_empty() {
                                submit_field_map.keys().cloned().collect()
                            } else {
                                action.submit_component.clone()
                            }
                        }
                    };
                    for target_comp_id in target_comps {
                        if let Some(submit_field) = submit_field_map.get(&target_comp_id) {
                            let parts: Vec<&str> = submit_field.split('.').collect();
                            if parts.len() >= 2 {
                                let model = parts[0];
                                let model_path = source_path_map
                                    .get(model)
                                    .map(|p| p.to_string())
                                    .unwrap_or_else(|| format!("{}.tbl", model));
                                let field = parts[1..].join(".");
                                let action_meta = serde_json::json!({
                                    "reason": format!("Action 'submitData' writes to field '{}'", field),
                                    "actor_kind": "action",
                                    "actor_id": action_id,
                                    "operation": "ActionWrites",
                                    "trigger": action.trigger_type,
                                    "target_model": model,
                                    "target_field": field,
                                    "source_expr": format!("{}.{}", model, field),
                                });
                                add_model_write_with_scope(
                                    graph,
                                    &action_id,
                                    model,
                                    &field,
                                    &model_path,
                                    EdgeType::ActionWrites,
                                    action_meta,
                                        Some(&scope),
                                )?;
                            }
                        }
                    }
                }
                "updateData" | "insertData" | "deleteData" => {
                    if let Some(ref data_set) = action.data_set {
                        let data_set_path = source_path_map
                            .get(data_set)
                            .map(|p| p.to_string())
                            .unwrap_or_else(|| format!("{}.tbl", data_set));
                        for (field_name, field_value, value_type) in &action.field_values {
                            let ud_meta = serde_json::json!({
                                "reason": format!("Action '{}' writes to field '{}'", action.action_type, field_name),
                                "actor_kind": "action",
                                "actor_id": action_id,
                                "operation": "ActionWrites",
                                "trigger": action.trigger_type,
                                "target_model": data_set,
                                "target_field": field_name,
                                "source_expr": field_value,
                            });
                            add_model_write_with_scope(
                                graph,
                                &action_id,
                                data_set,
                                field_name,
                                &data_set_path,
                                EdgeType::ActionWrites,
                                ud_meta,
                                    Some(&scope),
                            )?;
                            // If value_type is "exp", parse expression refs for dependency analysis
                            if value_type == "exp" {
                                let refs = crate::superpage::parse_expression_refs(field_value);
                                for ref_type in refs {
                                    if let crate::superpage::RefType::ModelField(model, field) =
                                        ref_type
                                    {
                                        let model_path = source_path_map
                                            .get(&model)
                                            .map(|p| p.to_string())
                                            .unwrap_or_else(|| format!("{}.tbl", model));
                                        let read_meta = serde_json::json!({
                                                "actor_kind": "action",
                                                "actor_id": action_id,
                                                "operation": "Reads",
                                                "trigger": action.trigger_type,
                                                "target_model": model,
                                                "target_field": field,
                                        "source_expr": format!("{}.{}", model, field),
                                                "reason": format!("Action '{}' reads from model '{}'", action.action_type, model),
                                            });
                                        add_model_read_with_scope(
                                            graph,
                                            &action_id,
                                            &model,
                                            &field,
                                            &model_path,
                                            read_meta,
                                            EdgeType::ActionReads,
                                                Some(&scope),
                                        )?;
                                    }
                                }
                            }
                        }
                    }
                }
                "link" if action.target_type.as_str() == "app" => {
                    // Try to resolve path as integer index into referenceResources
                    if let Some(ref path_str) = action.path
                        && let Ok(ref_idx) = path_str.parse::<usize>()
                        && let Some(target_rel) =
                            resolve_reference_path(rel_path, ref_idx, &meta.reference_resources)
                    {
                        let target_name = Path::new(&target_rel)
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| target_rel.clone());
                        let target_page_id = format!("page:{}", target_rel.replace(r"\", "/"));
                        add_node(
                            graph,
                            target_page_id.clone(),
                            NodeType::Page,
                            target_rel.clone(),
                            target_name.clone(),
                            None,
                        )?;
                        let opens_meta = serde_json::json!({
                            "reason": format!("Link action opens page '{}'", target_name),
                            "actor_kind": "action",
                            "actor_id": action_id,
                            "operation": "ActionNavigates",
                            "trigger": action.trigger_type,
                            "target_model": target_name,
                        });
                        add_edge_with_meta(
                            graph,
                            &action_id,
                            &target_page_id,
                            EdgeType::ActionNavigates,
                            Some(target_rel.clone()),
                            Some(opens_meta),
                        )?;

                        // Process parameter passing via data array
                        for (param_name, param_value) in &action.data {
                            let param_id = format!("param:{}/{}", target_name, param_name);
                            add_node(
                                graph,
                                param_id.clone(),
                                NodeType::Field,
                                target_rel.clone(),
                                param_name.clone(),
                                None,
                            )?;
                            let pass_meta = serde_json::json!({
                                "reason": format!("Link action passes param '{}'", param_name),
                                "actor_kind": "action",
                                "actor_id": action_id,
                                "operation": "PassesParam",
                                "trigger": action.trigger_type,
                                "target_field": param_name,
                                "source_expr": param_value,
                            });
                            add_edge_with_meta(
                                graph,
                                &action_id,
                                &param_id,
                                EdgeType::PassesParam,
                                Some(param_value.clone()),
                                Some(pass_meta),
                            )?;
                            // Parse expression refs from param_value for dependency analysis
                            let refs = crate::superpage::parse_expression_refs(param_value);
                            for ref_type in refs {
                                if let crate::superpage::RefType::ModelField(model, field) =
                                    ref_type
                                {
                                    let model_path = source_path_map
                                        .get(&model)
                                        .map(|p| p.to_string())
                                        .unwrap_or_else(|| format!("{}.tbl", model));
                                    let read_meta = serde_json::json!({
                                        "actor_kind": "action",
                                        "actor_id": action_id,
                                        "operation": "Reads",
                                        "trigger": action.trigger_type,
                                        "target_model": model,
                                        "target_field": field,
                                    "source_expr": format!("{}.{}", model, field),
                                        "reason": format!("Action '{}' reads from model '{}'", action.action_type, model),
                                    });
                                    add_model_read_with_scope(
                                        graph,
                                        &action_id,
                                        &model,
                                        &field,
                                        &model_path,
                                        read_meta,
                                        EdgeType::ActionReads,
                                            Some(&scope),
                                    )?;
                                }
                            }
                        }
                    }
                }
                "setParamValue" => {
                    for (param_name, param_value) in &action.params {
                        let param_id =
                            format!("param:{}|{}", rel_path.replace("\\", "/"), param_name);
                        add_node(
                            graph,
                            param_id.clone(),
                            NodeType::Field,
                            rel_path.to_string(),
                            param_name.clone(),
                            None,
                        )?;
                        let sets_meta = serde_json::json!({
                            "reason": format!("setParamValue sets param '{}'", param_name),
                            "actor_kind": "action",
                            "actor_id": action_id,
                            "operation": "ActionSetsParam",
                            "trigger": action.trigger_type,
                            "target_field": param_name,
                            "source_expr": param_value,
                        });
                        add_edge_with_meta(
                            graph,
                            &action_id,
                            &param_id,
                            EdgeType::ActionSetsParam,
                            Some(param_value.clone()),
                            Some(sets_meta),
                        )?;
                        // Parse expression refs from param_value for dependency analysis
                        let refs = crate::superpage::parse_expression_refs(param_value);
                        for ref_type in refs {
                            if let crate::superpage::RefType::ModelField(model, field) = ref_type {
                                let model_path = source_path_map
                                    .get(&model)
                                    .map(|p| p.to_string())
                                    .unwrap_or_else(|| format!("{}.tbl", model));
                                let read_meta = serde_json::json!({
                                    "actor_kind": "action",
                                    "actor_id": action_id,
                                    "operation": "Reads",
                                    "trigger": action.trigger_type,
                                    "target_model": model,
                                    "target_field": field,
                                    "source_expr": format!("{}.{}", model, field),
                                    "reason": format!("Action '{}' reads from model '{}'", action.action_type, model),
                                });
                                add_model_read_with_scope(
                                    graph,
                                    &action_id,
                                    &model,
                                    &field,
                                    &model_path,
                                    read_meta,
                                    EdgeType::ActionReads,
                                        Some(&scope),
                                )?;
                            }
                        }
                    }
                }
                "showComponent" | "hideComponent" => {
                    for target_comp in &action.target_component {
                        let target_comp_id =
                            format!("comp:{}|{}", rel_path.replace(r"\", "/"), target_comp);
                        let ctrl_meta = serde_json::json!({
                            "reason": format!("Action '{}' controls component '{}'", action.action_type, target_comp),
                            "actor_kind": "action",
                            "actor_id": action_id,
                            "operation": "ActionControlsComponent",
                            "trigger": action.trigger_type,
                            "target_component": target_comp,
                        });
                        add_edge_with_meta(
                            graph,
                            &action_id,
                            &target_comp_id,
                            EdgeType::ActionControlsComponent,
                            None,
                            Some(ctrl_meta),
                        )?;
                    }
                }
                "showDialog" => {
                    if let Some(ref dialog_id) = action.dialog {
                        let dialog_comp_id =
                            format!("comp:{}|{}", rel_path.replace(r"\", "/"), dialog_id);
                        // 通过 upsert 确保目标节点存在；None meta 不覆盖已有节点 meta。
                        add_node(
                            graph,
                            dialog_comp_id.clone(),
                            NodeType::Component,
                            rel_path.to_string(),
                            dialog_id.clone(),
                            None,
                        )?;
                        let dialog_meta = serde_json::json!({
                            "reason": format!("Action 'showDialog' opens dialog '{}'", dialog_id),
                            "actor_kind": "action",
                            "actor_id": action_id,
                            "operation": "ActionControlsComponent",
                            "trigger": action.trigger_type,
                            "dialog": dialog_id,
                        });
                        add_edge_with_meta(
                            graph,
                            &action_id,
                            &dialog_comp_id,
                            EdgeType::ActionControlsComponent,
                            None,
                            Some(dialog_meta),
                        )?;
                    }
                }
                "closeDialog" => {
                    let close_meta = serde_json::json!({
                        "reason": "Action 'closeDialog' closes current dialog",
                        "actor_kind": "action",
                        "actor_id": action_id,
                        "operation": "ActionControlsComponent",
                        "trigger": action.trigger_type,
                    });
                    add_edge_with_meta(
                        graph,
                        &action_id,
                        &comp_id,
                        EdgeType::ActionControlsComponent,
                        None,
                        Some(close_meta),
                    )?;
                }
                "switchPanel" => {
                    if let Some(ref pb) = action.panelbook {
                        let panelbook_id = format!("comp:{}|{}", rel_path.replace(r"\", "/"), pb);
                        // 通过 upsert 确保目标节点存在；None meta 不覆盖已有节点 meta。
                        add_node(
                            graph,
                            panelbook_id.clone(),
                            NodeType::Component,
                            rel_path.to_string(),
                            pb.clone(),
                            None,
                        )?;
                        let ctrl_meta = serde_json::json!({
                            "reason": format!("Action 'switchPanel' controls panelbook '{}'", pb),
                            "actor_kind": "action",
                            "actor_id": action_id,
                            "operation": "ActionControlsComponent",
                            "trigger": action.trigger_type,
                            "panelbook": pb,
                            "panel": action.panel,
                        });
                        add_edge_with_meta(
                            graph,
                            &action_id,
                            &panelbook_id,
                            EdgeType::ActionControlsComponent,
                            None,
                            Some(ctrl_meta),
                        )?;
                    }
                }
                "validateData" => {
                    let target_comps: Vec<String> = match action.submit_range.as_deref() {
                        Some("page") | Some("dataset") => {
                            submit_field_map.keys().cloned().collect()
                        }
                        Some("component") | Some("dialog") => action.submit_component.clone(),
                        _ => {
                            if action.submit_component.is_empty() {
                                submit_field_map.keys().cloned().collect()
                            } else {
                                action.submit_component.clone()
                            }
                        }
                    };
                    for target_comp_id in target_comps {
                        if let Some(submit_field) = submit_field_map.get(&target_comp_id) {
                            let parts: Vec<&str> = submit_field.split('.').collect();
                            if parts.len() >= 2 {
                                let model = parts[0];
                                let model_path = source_path_map
                                    .get(model)
                                    .map(|p| p.to_string())
                                    .unwrap_or_else(|| format!("{}.tbl", model));
                                let field = parts[1..].join(".");
                                let val_meta = serde_json::json!({
                                    "reason": format!("Action 'validateData' validates field '{}'", field),
                                    "actor_kind": "action",
                                    "actor_id": action_id,
                                    "operation": "ActionValidates",
                                    "trigger": action.trigger_type,
                                    "target_model": model,
                                    "target_field": field,
                                    "source_expr": format!("{}.{}", model, field),
                                });
                                add_model_write_with_scope(
                                    graph,
                                    &action_id,
                                    model,
                                    &field,
                                    &model_path,
                                    EdgeType::ActionValidates,
                                    val_meta,
                                        Some(&scope),
                                )?;
                            }
                        }
                    }
                }
                "resetData" | "newData" | "refreshModels" | "refreshData" | "loadData" => {
                    let mut targets: Vec<(String, String)> = Vec::new();
                    if let Some(ref ds) = action.data_set {
                        targets.push((ds.clone(), format!("{}.tbl", ds)));
                    }
                    for sc in &action.submit_component {
                        if let Some(submit_field) = submit_field_map.get(sc) {
                            let parts: Vec<&str> = submit_field.split('.').collect();
                            if !parts.is_empty() {
                                let model = parts[0].to_string();
                                let model_path = source_path_map
                                    .get(&model)
                                    .map(|p| p.to_string())
                                    .unwrap_or_else(|| format!("{}.tbl", model));
                                targets.push((model, model_path));
                            }
                        }
                    }
                    if targets.is_empty() && !action.submit_component.is_empty() {
                        for sc in &action.submit_component {
                            let comp_full_id =
                                format!("comp:{}|{}", rel_path.replace(r"\", "/"), sc);
                            let load_meta = serde_json::json!({
                                "reason": format!("Action '{}' loads/resets component '{}'", action.action_type, sc),
                                "actor_kind": "action",
                                "actor_id": action_id,
                                "operation": "ActionLoadsData",
                                "trigger": action.trigger_type,
                                "target_component": sc,
                            });
                            add_edge_with_meta(
                                graph,
                                &action_id,
                                &comp_full_id,
                                EdgeType::ActionLoadsData,
                                None,
                                Some(load_meta),
                            )?;
                        }
                    }
                    for (model, model_path) in targets {
                        let load_meta = serde_json::json!({
                            "reason": format!("Action '{}' loads/resets model '{}'", action.action_type, model),
                            "actor_kind": "action",
                            "actor_id": action_id,
                            "operation": "ActionLoadsData",
                            "trigger": action.trigger_type,
                            "target_model": model,
                        });
                        add_model_write_with_scope(
                            graph,
                            &action_id,
                            &model,
                            "*",
                            &model_path,
                            EdgeType::ActionLoadsData,
                            load_meta,
                                Some(&scope),
                        )?;
                    }
                }
                _ => {}
            }
        }
    }

    // 写入 condition 节点和依赖边
    let conditions = crate::conditions::scan_conditions(&meta, Some(rel_path));
    for cond in &conditions {
        let cond_node_id = format!("cond:{}|{}", rel_path.replace(r"\", "/"), cond.condition_id);
        let cond_name = format!("{:?}:{}", cond.condition_type, cond.condition_id);
        let cond_meta = serde_json::json!({
            "condition_type": format!("{:?}", cond.condition_type),
            "effect_type": format!("{:?}", cond.effect_type),
            "subject_type": format!("{:?}", cond.subject_type),
            "raw_expr": cond.raw_expr,
            "normalized_expr": cond.normalized_expr,
            "json_path": cond.json_path,
            "owner_type": format!("{:?}", cond.owner_type),
            "owner_id": cond.owner_id,
            "referenced_symbols": cond.referenced_symbols,
        });
        add_node(
            graph,
            cond_node_id.clone(),
            NodeType::Condition,
            cond.source_file.as_deref().unwrap_or(rel_path).to_string(),
            cond_name,
            Some(cond_meta),
        )?;
        if !node_ids.contains(&cond_node_id) {
            node_ids.insert(cond_node_id.clone());
        }

        // condition -> owner 边
        let owner_node_id = match cond.owner_type {
            crate::conditions::OwnerType::Component => {
                format!("comp:{}|{}", rel_path.replace(r"\", "/"), cond.owner_id)
            }
            crate::conditions::OwnerType::Action => {
                let parts: Vec<&str> = cond.owner_id.split(':').collect();
                if parts.len() >= 2 {
                    format!(
                        "action:{}|{}|{}",
                        rel_path.replace(r"\", "/"),
                        parts[0],
                        parts[1]
                    )
                } else {
                    continue;
                }
            }
            crate::conditions::OwnerType::ModelSource => scope.model_id(&cond.owner_id)?,
            crate::conditions::OwnerType::FieldDefault => {
                format!("comp:{}|{}", rel_path.replace(r"\", "/"), cond.owner_id)
            }
            crate::conditions::OwnerType::Page => page_id.clone(),
        };
        let owner_edge_meta = serde_json::json!({
            "reason": format!("Condition '{}' belongs to owner '{}'", cond.condition_id, cond.owner_id),
            "actor_kind": "condition",
            "actor_id": cond.condition_id,
            "operation": "Conditions",
            "json_path": cond.json_path,
            "condition_type": format!("{:?}", cond.condition_type),
        });
        add_edge_with_meta(
            graph,
            &cond_node_id,
            &owner_node_id,
            EdgeType::DependsOn,
            Some(cond.json_path.clone()),
            Some(owner_edge_meta),
        )?;

        // condition -> upstream dependency 边
        for sym in &cond.referenced_symbols {
            let sym_parts: Vec<&str> = sym.splitn(2, ':').collect();
            if sym_parts.len() < 2 {
                continue;
            }
            let (kind, target_id) = (sym_parts[0], sym_parts[1]);
            // M58.3 复核返修 P1-4：component 臂与 model 臂对齐剥掉 `.后缀`——
            // 符号 `component:{id}.{prop}` 的节点 id 只到组件 id（真实节点是
            // `comp:<page>|{id}`，带后缀的 id 不存在，边会被两端存储静默丢弃）；
            // 属性名不丢弃，保留在边 meta 的 target_property（field_path 仍携带
            // 完整符号串），与上方 ComponentProperty 建边路径同口径
            // M58.3 相邻缺口修复：param / user / system 三类目标节点此前只在
            // **组件表达式**那条路径上创建（见上方 RefType::Param/UserProperty/
            // SystemVar 分支）。只出现在条件里的符号没有对应节点，边被两端存储
            // 静默丢弃——整整三个类别的 cond→依赖边悬挂。这里按与组件表达式路径
            // 完全一致的 id / NodeType / meta 补建节点（add_node 幂等），符号本身
            // 就是事实，不存在凭空造节点的问题。
            let (target_node_id, target_property) = match kind {
                "param" => {
                    let param_id = format!("param:{}|{}", rel_path.replace(r"\", "/"), target_id);
                    add_node(
                        graph,
                        param_id.clone(),
                        NodeType::Field,
                        rel_path.to_string(),
                        target_id.to_string(),
                        Some(serde_json::json!({"kind": "param"})),
                    )?;
                    (param_id, None)
                }
                "model" => {
                    let model_name = target_id.split('.').next().unwrap_or(target_id);
                    (scope.model_id(model_name)?, None)
                }
                "component" => {
                    let mut segments = target_id.splitn(2, '.');
                    let comp_name = segments.next().unwrap_or(target_id);
                    (
                        format!("comp:{}|{}", rel_path.replace(r"\", "/"), comp_name),
                        segments.next().filter(|prop| !prop.is_empty()),
                    )
                }
                "user" => {
                    let user_id = format!("user:{}", target_id);
                    add_node(
                        graph,
                        user_id.clone(),
                        NodeType::Field,
                        "system".to_string(),
                        format!("$user.{}", target_id),
                        Some(serde_json::json!({"kind": "user_property"})),
                    )?;
                    (user_id, None)
                }
                "system" => {
                    let sys_id = format!("system:{}", target_id);
                    add_node(
                        graph,
                        sys_id.clone(),
                        NodeType::Field,
                        "system".to_string(),
                        format!("${}", target_id),
                        Some(serde_json::json!({"kind": "system_var"})),
                    )?;
                    (sys_id, None)
                }
                _ => continue,
            };
            let mut dep_edge_meta = serde_json::json!({
                "reason": format!("Condition '{}' depends on symbol '{}'", cond.condition_id, sym),
                "actor_kind": "condition",
                "actor_id": cond.condition_id,
                "operation": "DependsOn",
                "source_expr": cond.raw_expr,
                "json_path": cond.json_path,
            });
            if let Some(prop) = target_property
                && let Some(obj) = dep_edge_meta.as_object_mut()
            {
                obj.insert("target_property".to_string(), serde_json::json!(prop));
            }
            add_edge_with_meta(
                graph,
                &cond_node_id,
                &target_node_id,
                EdgeType::DependsOn,
                Some(sym.clone()),
                Some(dep_edge_meta),
            )?;
        }
    }

    // 建立 model.filter 与 model.totalRowCount__ 的隐式关系
    for cond in &conditions {
        if matches!(
            cond.condition_type,
            crate::conditions::ConditionType::SourceFilterExp
        ) && matches!(cond.owner_type, crate::conditions::OwnerType::ModelSource)
        {
            let model_name = &cond.owner_id;
            let trc_field_id = scope.field_id(model_name, "totalRowCount__")?;
            let trc_model_id = scope.model_id(model_name)?;
            scope.add_node(
                graph,
                &trc_field_id,
                NodeType::Field,
                rel_path.to_string(),
                "totalRowCount__".to_string(),
                Some(serde_json::json!({
                    "kind": "implicit",
                    "description": "模型过滤后的隐式行数字段",
                })),
            )?;
            add_edge_with_meta(
                graph,
                &trc_model_id,
                &trc_field_id,
                EdgeType::Contains,
                None,
                None,
            )?;

            let cond_node_id =
                format!("cond:{}|{}", rel_path.replace(r"\", "/"), cond.condition_id);
            let trc_edge_meta = serde_json::json!({
                "reason": format!("Filter condition determines {}.totalRowCount__", model_name),
                "actor_kind": "condition",
                "actor_id": cond.condition_id.clone(),
                "operation": "DependsOn",
                "source_expr": cond.raw_expr.clone(),
                "json_path": cond.json_path.clone(),
            });
            add_edge_with_meta(
                graph,
                &cond_node_id,
                &trc_field_id,
                EdgeType::DependsOn,
                Some(format!("{}.totalRowCount__", model_name)),
                Some(trc_edge_meta),
            )?;
        }
    }

    // Process dwtable sources: establish DataflowInput from local model to physical table
    for source in &meta.sources {
        if source.model_type.as_deref() != Some("dwtable") {
            continue;
        }
        let Some(ref path) = source.path else {
            continue;
        };

        // Extract physical table name from path like "$DATA:/主数据/fact_qwSidebar.tbl"
        let physical_table = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(path);
        // 局部实体与物理实体分别构造身份：二者同名时也不得塌成一个节点，
        // 否则边会退化成自环（source id == 物理表名 的典型场景）。
        let model_id = scope.model_id(&source.id)?;
        let physical_model_id = scope.physical_model_id(physical_table)?;
        scope.add_node(
            graph,
            &model_id,
            NodeType::Model,
            path.clone(),
            source.id.clone(),
            Some(serde_json::json!({
                "modelType": "dwtable",
                "sourcePath": path,
            })),
        )?;
        if !node_ids.contains(&model_id) {
            node_ids.insert(model_id.clone());
        }
        // 写入端会保留已有 DataFlow/App modelType，避免物理表占位覆盖真实模型。
        scope.add_node(
            graph,
            &physical_model_id,
            NodeType::Model,
            path.clone(),
            physical_table.to_string(),
            Some(serde_json::json!({"modelType": "PhysicalTable", "sourcePath": path})),
        )?;
        // 同名时局部与物理仍是两个节点，边是有向的两个方向、不是自环
        add_edge_with_meta(
            graph,
            &model_id,
            &physical_model_id,
            EdgeType::DataflowInput,
            Some(path.clone()),
            None,
        )?;
        // 建立反向边，使 BFS 能从物理表回溯到局部模型
        add_edge_with_meta(
            graph,
            &physical_model_id,
            &model_id,
            EdgeType::DataflowOutput,
            Some(path.clone()),
            None,
        )?;
    }

    Ok(node_ids.into_iter().collect())
}
