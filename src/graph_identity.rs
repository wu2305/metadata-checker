//! M59-1（A1/A2）：页面局部节点 id 文法与统一路径归一化。
//!
//! # 身份文法（A1）
//!
//! - 页面局部节点：`<kind>:<PAGE>|<local>`，`<PAGE>` 是归一化后的源文件相对路径，
//!   `<local>` 是页面局部 id（如 `model1`、`model1.fieldName`、`sourceId`）。
//!   **竖线即判据**：kind 前缀后带 `|` 的是页面局部节点，不带的是全局节点。
//! - 物理表模型/字段保持全局：`model:<tableName>` / `field:<tableName>.<name>`
//!   是 tbl 扫描的聚合产物，不归属单一页面；页面局部模型通过
//!   DataflowInput/DataflowOutput/FieldAlias 边与物理表关联。
//! - 页面局部模型：页面 sources 里的 dataflow / dwtable / filter 作用域模型，
//!   归属所在页面；同一局部名在两个页面是两个节点，互不塌陷。
//! - `cond` / `comp` / `action` / `param` 的既有文法已带页面段，与本模块同一解析。
//!
//! # 旧 target 显式解析（A1）
//!
//! 裸 `model:<local>` 不再默认绑定全局节点：唯一命中直达；多页同名（含旧全局
//! 节点与新局部节点混存的过渡期图）交回全部候选，由调用方生成歧义诊断，
//! 不得静默挑选——与 m58_3 裸 target「逐个候选作答」契约同口径。
//!
//! # 共享目标归属（A3，M59-2 落地）
//!
//! 页面局部节点归属其页面文件；全局物理节点不归属单一页，删除页面不得牵连
//! 其它页仍在引用的物理节点。origin_file 与删除按 origin 牵连在 M59-2 实现。
//!
//! # 路径归一化（A2）
//!
//! `<PAGE>` 与引用解析都先过 [`normalize_project_path`]：分隔符统一 `/`、
//! 消解 `.` 与 `..`、越界报错。**本模块在 M59-2 的 schema 版本开关就绪前
//! 不接入扫描写入与引用解析**——不得在旧 schema 下默认写入新 id 或混写
//! 新旧路径形态（plan 交付边界）。

use crate::graph::NodeType;
use crate::graph_store::{GraphReadStore, GraphStoreResult};
use std::fmt;

/// 图节点 id 的 kind 段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeIdKind {
    Model,
    Field,
    Cond,
    Comp,
    Action,
    Param,
    Page,
}

impl NodeIdKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeIdKind::Model => "model",
            NodeIdKind::Field => "field",
            NodeIdKind::Cond => "cond",
            NodeIdKind::Comp => "comp",
            NodeIdKind::Action => "action",
            NodeIdKind::Param => "param",
            NodeIdKind::Page => "page",
        }
    }

    pub fn from_str(kind: &str) -> Option<Self> {
        match kind {
            "model" => Some(NodeIdKind::Model),
            "field" => Some(NodeIdKind::Field),
            "cond" => Some(NodeIdKind::Cond),
            "comp" => Some(NodeIdKind::Comp),
            "action" => Some(NodeIdKind::Action),
            "param" => Some(NodeIdKind::Param),
            "page" => Some(NodeIdKind::Page),
            _ => None,
        }
    }
}

/// 身份构造/归一化的稳定错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    /// `..` 越出项目根：引用解析不得静默截断，必须报给调用方。
    EscapeBeyondRoot { path: String },
    /// kind / 页面段 / 局部名为空，无法构成合法 id。
    EmptySegment { id: String },
    /// 保留分隔符不能出现在身份段中。
    ReservedSeparator { value: String },
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdentityError::EscapeBeyondRoot { path } => {
                write!(f, "path escapes the project root: {}", path)
            }
            IdentityError::ReservedSeparator { value } => {
                write!(f, "identity segment contains reserved separator: {}", value)
            }
            IdentityError::EmptySegment { id } => {
                write!(f, "node id has an empty kind/page/local segment: {}", id)
            }
        }
    }
}

impl std::error::Error for IdentityError {}

/// A2：项目内相对路径归一化（扫描与引用解析共用的唯一实现）。
///
/// 输入必须是**根锚定**的项目内相对路径（扫描产出的页面路径天然满足；
/// 引用侧由调用方先与所在文件目录拼接，见 [`resolve_relative_reference`]）。
/// `\` 统一为 `/`；`.` 段与空段（开头/结尾/连续分隔符）直接消解；
/// `..` 在根锚定语义下越出根时返回 [`IdentityError::EscapeBeyondRoot`]——
/// 这是「引用逃出项目范围」的稳定诊断；调用方不得用未锚定的输入绕过它。
/// 中文段原样保留。
pub fn normalize_project_path(path: &str) -> Result<String, IdentityError> {
    let unified = path.replace('\\', "/");
    let mut segments: Vec<&str> = Vec::new();
    for segment in unified.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments
                    .pop()
                    .ok_or_else(|| IdentityError::EscapeBeyondRoot {
                        path: path.to_string(),
                    })?;
            }
            other => segments.push(other),
        }
    }
    Ok(segments.join("/"))
}

/// A2：从 `current_file` 所在目录解析相对引用，结果与扫描产出的页面路径同口径。
///
/// 引用本身是项目内相对路径（`$TAPP:` / `$DATA:` 前缀的展开由调用方先行处理）。
pub fn resolve_relative_reference(
    current_file: &str,
    reference: &str,
) -> Result<String, IdentityError> {
    let normalized_current = normalize_project_path(current_file)?;
    let current_dir = match normalized_current.rsplit_once('/') {
        Some((dir, _)) => dir,
        None => "",
    };
    if current_dir.is_empty() {
        normalize_project_path(reference)
    } else {
        normalize_project_path(&format!("{}/{}", current_dir, reference))
    }
}

/// 解析出的节点 id 三段。
///
/// **文法不变式**：kind 段、页面段、局部名都不得含 `|`——竖线是「页面局部 vs
/// 全局」的唯一判据，取 kind 前缀后第一个 `|` 切分；含 `|` 的全局名（物理表名、
/// 物理字段名可能来自允许竖线的文件系统，必须在写入边界验证）在文法之外，不受支持。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedNodeId {
    pub kind: NodeIdKind,
    /// 页面段；`None` 表示全局/物理节点（kind 前缀后不带 `|`）。
    pub page: Option<String>,
    /// 局部名。页面局部 field 的 local 形如 `model1.fieldName`（含字段后缀）。
    pub local: String,
}

/// A1：解析节点 id。竖线即判据：kind 前缀后第一个 `|` 之前是页面段。
///
/// 页面路径与局部名都不含 `|`（构造函数拒绝含竖线的页面路径和局部 id），因此取第一个 `|` 即可无歧义切分。
pub fn parse_node_id(id: &str) -> Option<ParsedNodeId> {
    let (kind_str, rest) = id.split_once(':')?;
    let kind = NodeIdKind::from_str(kind_str)?;
    match rest.split_once('|') {
        Some((page, local)) if !page.is_empty() && !local.is_empty() && !local.contains('|') => Some(ParsedNodeId {
            kind,
            page: Some(page.to_string()),
            local: local.to_string(),
        }),
        None if !rest.is_empty() => Some(ParsedNodeId {
            kind,
            page: None,
            local: rest.to_string(),
        }),
        _ => None,
    }
}

/// A1：判断 id 是否页面局部节点（kind 前缀后带 `|`）。
pub fn is_page_scoped_id(id: &str) -> bool {
    parse_node_id(id).is_some_and(|parsed| parsed.page.is_some())
}

/// A1：构造页面局部节点 id `<kind>:<PAGE>|<local>`。
///
/// 页面路径先过 [`normalize_project_path`]，保证 `app/./a.spg` 与 `app\a.spg`
/// 编码出同一个 id；局部名原样使用（field 的 local 已含 `.字段` 后缀）。
pub fn page_local_node_id(
    kind: NodeIdKind,
    page_path: &str,
    local: &str,
) -> Result<String, IdentityError> {
    for value in [page_path, local] {
        if value.contains('|') {
            return Err(IdentityError::ReservedSeparator { value: value.to_string() });
        }
    }
    if local.is_empty() {
        return Err(IdentityError::EmptySegment {
            id: format!("{}:{}", kind.as_str(), local),
        });
    }
    let page = normalize_project_path(page_path)?;
    if page.is_empty() {
        return Err(IdentityError::EmptySegment {
            id: format!("{}:|{}", kind.as_str(), local),
        });
    }
    Ok(format!("{}:{}|{}", kind.as_str(), page, local))
}

/// A1：旧 target 解析结果。歧义时交回全部候选，不静默挑选。
#[derive(Debug, Clone, PartialEq)]
pub enum TargetResolution {
    /// 唯一命中：scoped 精确 id，或裸名在图中只有一个同局部名节点。
    Unique(crate::graph::Node),
    /// 多个同局部名候选（跨页同名 / 旧全局节点与新局部节点混存）。
    /// 按 id 升序排序保证诊断顺序确定。
    Ambiguous(Vec<crate::graph::Node>),
    /// 无候选，或 target 前缀与 kind 不匹配。
    Missing,
}

/// A1：旧 target 显式解析，覆盖 model 与 field 两类页面局部身份。
///
/// `<kind>:<PAGE>|<local>` 精确查表（任意 kind 均可）；裸 `<kind>:<local>`
/// 收集全部同局部名、同 node_type 的节点——scoped 节点取 `|` 后的局部名，
/// 旧全局节点（物理表/物理字段或历史写入）取 `<kind>:` 后的整体。
/// 跨 kind 同名互不干扰；cond/comp/action/param 的文法恒为 scoped，
/// 裸名解析对它们未定义，返回 Missing。
pub fn resolve_node_target(
    graph: &dyn GraphReadStore,
    kind: NodeIdKind,
    target: &str,
) -> GraphStoreResult<TargetResolution> {
    let prefix = format!("{}:", kind.as_str());
    let Some(rest) = target.strip_prefix(&prefix) else {
        return Ok(TargetResolution::Missing);
    };
    if rest.is_empty() {
        return Ok(TargetResolution::Missing);
    }
    if rest.contains('|') {
        return Ok(match graph.get_node(target)? {
            Some(node) => TargetResolution::Unique(node),
            None => TargetResolution::Missing,
        });
    }

    let node_type = match kind {
        NodeIdKind::Model => NodeType::Model,
        NodeIdKind::Field => NodeType::Field,
        // 裸名解析只为 model / field 定义；其余 kind 文法恒为 scoped
        _ => return Ok(TargetResolution::Missing),
    };

    let mut candidates: Vec<crate::graph::Node> = Vec::new();
    for node in graph.iter_nodes()? {
        if node.node_type != node_type {
            continue;
        }
        let Some(node_rest) = node.id.strip_prefix(&prefix) else {
            continue;
        };
        let node_local = match node_rest.split_once('|') {
            Some((_, local)) => local,
            None => node_rest,
        };
        if node_local == rest {
            candidates.push(node);
        }
    }
    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    match candidates.pop() {
        None => Ok(TargetResolution::Missing),
        Some(only) if candidates.is_empty() => Ok(TargetResolution::Unique(only)),
        Some(last) => {
            candidates.push(last);
            Ok(TargetResolution::Ambiguous(candidates))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_unifies_separators_and_dot_segments() {
        assert_eq!(normalize_project_path("app/./a.spg").unwrap(), "app/a.spg");
        assert_eq!(
            normalize_project_path("app\\子目录\\a.spg").unwrap(),
            "app/子目录/a.spg"
        );
        assert_eq!(normalize_project_path("app/../b.spg").unwrap(), "b.spg");
        assert_eq!(
            normalize_project_path("页面/销售/合同协议.spg").unwrap(),
            "页面/销售/合同协议.spg"
        );
        assert_eq!(normalize_project_path("//a//b.spg").unwrap(), "a/b.spg");
    }

    #[test]
    fn normalize_rejects_escape_beyond_root() {
        assert_eq!(
            normalize_project_path("../escape.spg"),
            Err(IdentityError::EscapeBeyondRoot {
                path: "../escape.spg".to_string()
            })
        );
        assert_eq!(
            normalize_project_path("app/../../escape.spg"),
            Err(IdentityError::EscapeBeyondRoot {
                path: "app/../../escape.spg".to_string()
            })
        );
    }

    #[test]
    fn resolve_relative_reference_collapses_parent_segments() {
        assert_eq!(
            resolve_relative_reference("app/a.spg", "../b.spg").unwrap(),
            "b.spg"
        );
        assert_eq!(
            resolve_relative_reference("页面/销售/合同协议.spg", "./子页.spg").unwrap(),
            "页面/销售/子页.spg"
        );
        assert_eq!(
            resolve_relative_reference("a.spg", "b.spg").unwrap(),
            "b.spg"
        );
    }

    #[test]
    fn page_local_ids_isolate_same_local_name_across_pages() {
        let first = page_local_node_id(NodeIdKind::Model, "app/页面一.spg", "model1").unwrap();
        let second = page_local_node_id(NodeIdKind::Model, "app/页面二.spg", "model1").unwrap();
        assert_ne!(first, second, "跨页同名局部模型必须得到不同节点 id");
        assert_eq!(first, "model:app/页面一.spg|model1");

        // 分隔符与点段不改变编码：同一页面的不同写法收敛到同一 id
        let alternate = page_local_node_id(NodeIdKind::Model, "app\\页面一.spg", "model1").unwrap();
        assert_eq!(alternate, first, "页面路径归一化必须先于编码");
    }

    #[test]
    fn field_local_name_keeps_field_suffix_and_round_trips() {
        let id = page_local_node_id(NodeIdKind::Field, "app/页面一.spg", "model1.金额").unwrap();
        assert_eq!(id, "field:app/页面一.spg|model1.金额");
        let parsed = parse_node_id(&id).unwrap();
        assert_eq!(parsed.kind, NodeIdKind::Field);
        assert_eq!(parsed.page.as_deref(), Some("app/页面一.spg"));
        assert_eq!(parsed.local, "model1.金额");
    }

    #[test]
    fn parse_distinguishes_scoped_and_global_ids() {
        let scoped = parse_node_id("model:app/a.spg|model1").unwrap();
        assert!(scoped.page.is_some());
        assert_eq!(scoped.local, "model1");

        let physical = parse_node_id("model:fact_testDrive").unwrap();
        assert!(physical.page.is_none(), "物理表模型必须保持全局语义");
        assert_eq!(physical.local, "fact_testDrive");

        let legacy = parse_node_id("field:model1.x").unwrap();
        assert!(legacy.page.is_none(), "旧全局 field id 不带页面段");
        assert_eq!(legacy.local, "model1.x");

        assert!(is_page_scoped_id("cond:app/a.spg|cond1"));
        assert!(!is_page_scoped_id("model:fact_testDrive"));
        assert!(parse_node_id("model:|model1").is_none(), "空页面段非法");
        assert!(parse_node_id("unknown:model1").is_none(), "未知 kind 拒绝");
    }

    #[test]
    fn page_local_node_id_rejects_empty_segments() {
        assert!(page_local_node_id(NodeIdKind::Model, "app/a.spg", "").is_err());
        assert!(page_local_node_id(NodeIdKind::Model, "../escape.spg", "model1").is_err());
    }
}
