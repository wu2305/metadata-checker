use crate::graph::{Edge, EdgeType, Node, NodeType};
use crate::graph_store::GraphWriteStore;
use anyhow::{Context, Result};

/// 向图存储写入**已完成身份构造**的节点。
///
/// 调用方必须先经 [`crate::graph_identity`] 或 [`crate::scanner::spg::PageScope`]
/// 决定页面局部/全局身份；本函数只做写入边界的歧义拒绝，不再二次转换 id——
/// 页面局部 id 过 [`add_node`] 的全局转换会丢掉页面段，正是 M59-2 自环与身份
/// 塌陷的根因之一。
pub fn add_identified_node(
    graph: &mut dyn GraphWriteStore,
    id: String,
    node_type: NodeType,
    path: String,
    name: String,
    meta: Option<serde_json::Value>,
) -> Result<()> {
    // 竖线是「页面局部 vs 全局」的唯一判据：全局名不得含分隔符；页面局部 id
    // 的页面段与局部名各自也不得再含分隔符（否则解析侧无法判定作用域）。
    if let Some(rest) = id
        .strip_prefix("model:")
        .or_else(|| id.strip_prefix("field:"))
    {
        crate::graph_identity::reject_reserved_separator(rest, &id)?;
    }
    graph
        .upsert_node(Node {
            id,
            node_type,
            path,
            name,
            meta,
            origin_file: None,
        })
        .with_context(|| "Failed to upsert graph node")
}

/// 向图存储写入节点（旧全局身份入口；model/field 仅限 `LegacyGlobal` 路径，
/// 页面/组件等非 model/field 节点不受身份模式影响）。
pub fn add_node(
    graph: &mut dyn GraphWriteStore,
    id: String,
    node_type: NodeType,
    path: String,
    name: String,
    meta: Option<serde_json::Value>,
) -> Result<()> {
    // LegacyGlobal 模式下 model/field 保持全局编码；页面局部身份只允许经
    // [`add_identified_node`] 写入（调用方先用 `PageScope` 构造完整 id），
    // 此处强制走全局形态并在写入边界拒绝歧义分隔符。
    let id = if let Some(local) = id.strip_prefix("model:") {
        crate::graph_identity::global_node_id(crate::graph_identity::NodeIdKind::Model, local)?
    } else if let Some(local) = id.strip_prefix("field:") {
        crate::graph_identity::global_node_id(crate::graph_identity::NodeIdKind::Field, local)?
    } else {
        id
    };
    graph
        .upsert_node(Node {
            id,
            node_type,
            path,
            name,
            meta,
            origin_file: None,
        })
        .with_context(|| "Failed to upsert graph node")
}

/// 向图存储写入带元数据的边。
pub fn add_edge_with_meta(
    graph: &mut dyn GraphWriteStore,
    from: &str,
    to: &str,
    edge_type: EdgeType,
    field_path: Option<String>,
    meta: Option<serde_json::Value>,
) -> Result<()> {
    graph
        .add_edge(Edge {
            from: from.to_string(),
            to: to.to_string(),
            edge_type,
            field_path,
            meta,
            origin_file: None,
        })
        .with_context(|| format!("Failed to add graph edge from {} to {}", from, to))
}

/// 引用路径无法落到图里一个页面节点上的原因。
///
/// 解析不了就是解析不了——调用方**不得**退回「按原样拼接」造出一个磁盘上不存在的
/// Page 节点（语料里曾因此出现 42 个幽灵页面）；应跳过这条边并如实记诊断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceUnresolved {
    /// `referenceResources` 里没有这个下标
    IndexOutOfRange { index: usize, len: usize },
    /// 引用没有填写目标下标（link 的 `path` 或内嵌页的 `resPath` 缺失或为 null）：配置不完整，没有可连的页面
    MissingIndex,
    /// `resPath` / `path` 不是合法的非负整数下标（如 `"abc"`、`-1`）
    MalformedIndex { value: String },
    /// 前缀（`$APP:` 等）需要项目的 app 目录结构才能展开，但本文件不在 `app/<name>.app/` 之下
    NoAppRoot { prefix: &'static str },
    /// 不认识的 `$XXX:` 前缀（含图片类 `$ICON:` 等资源前缀）
    UnknownPrefix { reference: String },
    /// 绝对路径（如 `/sysdata/...`）：指向平台系统工程，不在本仓库里
    AbsolutePath { reference: String },
    /// URI（如 `https://host/page.spg`）：不是项目内路径
    Uri { reference: String },
    /// `..` 越出项目根
    EscapesRoot { reference: String },
    /// 目标不是 `.spg` 页面（`.rpt`、`.action`、`.docx` 等），图里没有对应的页面节点
    NotAPage { target: String },
}

impl std::fmt::Display for ReferenceUnresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IndexOutOfRange { index, len } => write!(
                f,
                "referenceResources index {index} is out of range (len {len})"
            ),
            Self::MissingIndex => {
                write!(f, "reference has no target index (path/resPath is missing)")
            }
            Self::MalformedIndex { value } => write!(
                f,
                "reference index is not a non-negative integer: {value:?}"
            ),
            Self::Uri { reference } => {
                write!(f, "URI reference is not a project path: {reference}")
            }
            Self::NoAppRoot { prefix } => write!(
                f,
                "prefix {prefix} needs an app/<name>.app directory in the file path"
            ),
            Self::UnknownPrefix { reference } => {
                write!(f, "unrecognized path prefix in reference: {reference}")
            }
            Self::AbsolutePath { reference } => write!(
                f,
                "absolute reference points outside the project: {reference}"
            ),
            Self::EscapesRoot { reference } => {
                write!(f, "reference escapes the project root: {reference}")
            }
            Self::NotAPage { target } => {
                write!(f, "reference target is not a .spg page: {target}")
            }
        }
    }
}

/// 在 `rel_path` 的各段里找 `app/<name>.app`，返回 `(app 目录之前的段数, .app 目录所在下标)`。
///
/// 只认「父段恰为 `app` 且本段以 `.app` 结尾」，这样无论扫描根是 `projects/xiaoshouyi`
/// 还是更上一层都能定位到当前应用。
fn locate_app_dir(segments: &[&str]) -> Option<usize> {
    (1..segments.len())
        .find(|&index| segments[index - 1] == "app" && segments[index].ends_with(".app"))
}

/// 推断项目根（根下并列 `app/`、`ana/`、`data/`）在 `rel_path` 里的位置，返回根之前的段。
///
/// 在某个 `app/<name>.app` 之下时，根就是 `app` 之前的段；否则找第一个 `ana` / `data` 目录段
/// （它们是根的同级目录，不是文件）。都找不到就无从推断，调用方按 `NoAppRoot` 处理，
/// 不去猜一个根——猜错会造出磁盘上不存在的页面路径。
fn locate_project_root<'a>(
    segments: &'a [&'a str],
    app_dir: Option<usize>,
) -> Option<&'a [&'a str]> {
    if let Some(index) = app_dir {
        return Some(&segments[..index - 1]);
    }
    let directory_count = segments.len().saturating_sub(1);
    (0..directory_count)
        .find(|&index| matches!(segments[index], "ana" | "data"))
        .map(|index| &segments[..index])
}

/// 把一条引用字符串展开为项目根锚定的页面路径。
///
/// 前缀语义（对照真实语料确认）：
/// - `$TAPP:/X` → 当前应用目录（`<root>/app/<当前>.app/`）下的 `X`
/// - `$APP:/X` → `<root>/app/` 下的 `X`（跨应用引用，`X` 以 `<别的>.app/` 开头）
/// - `$ANA:/X` → `<root>/ana/` 下的 `X`
/// - `$DATA:/X` → `<root>/data/` 下的 `X`
/// - 无前缀 → 相对当前文件所在目录，`..` 与 `.` 归一
///
/// 其余（`/` 开头的绝对路径、`$ICON:` 等未知前缀）一律返回 [`ReferenceUnresolved`]。
/// 结果必须是 `.spg`，否则图里没有可挂的页面节点，同样返回 `NotAPage`。
pub fn resolve_reference_target(
    rel_path: &str,
    reference: &str,
) -> Result<String, ReferenceUnresolved> {
    let current = rel_path.replace('\\', "/");
    let segments: Vec<&str> = current.split('/').filter(|part| !part.is_empty()).collect();
    let joined = if let Some((prefix, rest)) = split_dollar_prefix(reference) {
        let app_dir = locate_app_dir(&segments);
        // `$APP:` / `$ANA:` / `$DATA:` 以项目根为锚，不要求当前文件在某个应用之下；
        // `$TAPP:` 才必须有当前应用。
        let root_segments = locate_project_root(&segments, app_dir);
        let base = match (prefix, app_dir, root_segments) {
            ("$TAPP:", Some(index), _) => segments[..=index].join("/"),
            ("$APP:", _, Some(root)) => join_root_sibling(root, "app"),
            ("$ANA:", _, Some(root)) => join_root_sibling(root, "ana"),
            ("$DATA:", _, Some(root)) => join_root_sibling(root, "data"),
            ("$TAPP:", None, _) => return Err(ReferenceUnresolved::NoAppRoot { prefix: "$TAPP:" }),
            ("$APP:", _, None) => return Err(ReferenceUnresolved::NoAppRoot { prefix: "$APP:" }),
            ("$ANA:", _, None) => return Err(ReferenceUnresolved::NoAppRoot { prefix: "$ANA:" }),
            ("$DATA:", _, None) => return Err(ReferenceUnresolved::NoAppRoot { prefix: "$DATA:" }),
            _ => {
                return Err(ReferenceUnresolved::UnknownPrefix {
                    reference: reference.to_string(),
                });
            }
        };
        // 前缀后面跟 URI（`$TAPP:https://host/p.spg`）同样不是项目内路径；
        // 不拦的话 `https://` 会被归一成相对目录 `https:/`，造出磁盘上没有的页面。
        let rest = rest.trim_start_matches(['/', '\\']);
        if has_uri_scheme(rest) {
            return Err(ReferenceUnresolved::Uri {
                reference: reference.to_string(),
            });
        }
        // 先在前缀自己的命名空间里归一：`..` 不得跳出前缀锚点
        // （如 `$TAPP:/../b.app/x.spg` 不能借 `..` 离开当前应用）。
        let anchored = crate::graph_identity::normalize_project_path(rest).map_err(|_| {
            ReferenceUnresolved::EscapesRoot {
                reference: reference.to_string(),
            }
        })?;
        format!("{base}/{anchored}")
    } else if has_uri_scheme(reference) {
        return Err(ReferenceUnresolved::Uri {
            reference: reference.to_string(),
        });
    } else if reference.starts_with(['/', '\\']) || has_drive_prefix(reference) {
        return Err(ReferenceUnresolved::AbsolutePath {
            reference: reference.to_string(),
        });
    } else {
        // 相对路径：从当前文件所在目录出发
        let dir = segments[..segments.len().saturating_sub(1)].join("/");
        if dir.is_empty() {
            reference.to_string()
        } else {
            format!("{dir}/{reference}")
        }
    };
    let normalized = crate::graph_identity::normalize_project_path(&joined).map_err(|_| {
        ReferenceUnresolved::EscapesRoot {
            reference: reference.to_string(),
        }
    })?;
    // 扫描根可能在项目根之上（`projects/<x>/app/...`）：`..` 不得借此跳进兄弟项目。
    // 能推断出项目根时，结果必须仍在根之下；推断不出（没有 app/ana/data 线索）时只受扫描根约束。
    if let Some(root) = locate_project_root(&segments, locate_app_dir(&segments))
        && !root.is_empty()
    {
        let root_prefix = format!("{}/", root.join("/"));
        if !normalized.starts_with(&root_prefix) {
            return Err(ReferenceUnresolved::EscapesRoot {
                reference: reference.to_string(),
            });
        }
    }
    // 与文件发现（`discover_files` 只认小写 `spg` 扩展名）同一口径：大小写不同的文件
    // 不会被索引，指向它的引用只会留下没有定义的空页面。
    if !normalized.ends_with(".spg") {
        return Err(ReferenceUnresolved::NotAPage { target: normalized });
    }
    Ok(normalized)
}

/// Windows 盘符前缀（`C:\x`、`C:/x`）：拼到当前目录之后 `normalize_project_path` 就认不出它了，
/// 所以必须在拼接前拒绝。
fn has_drive_prefix(reference: &str) -> bool {
    let bytes = reference.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// URI scheme（`https:`、`file:`、`mailto:` 等，至少两个字符，与单字符盘符区分）。
fn has_uri_scheme(reference: &str) -> bool {
    let Some(colon) = reference.find(':') else {
        return false;
    };
    let scheme = &reference[..colon];
    scheme.len() >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// 拆出 `$XXX:` 前缀；`$` 开头但没有冒号的不算前缀。
fn split_dollar_prefix(reference: &str) -> Option<(&str, &str)> {
    if !reference.starts_with('$') {
        return None;
    }
    let colon = reference.find(':')?;
    Some((&reference[..=colon], &reference[colon + 1..]))
}

/// 项目根（`app/` 之前的段）下的同级目录；根为空时就是目录名本身。
fn join_root_sibling(root_segments: &[&str], name: &str) -> String {
    if root_segments.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", root_segments.join("/"), name)
    }
}

/// 按 `referenceResources` 下标取引用并解析为页面路径，见 [`resolve_reference_target`]。
pub fn resolve_reference_path(
    rel_path: &str,
    ref_idx: usize,
    reference_resources: &[String],
) -> Result<String, ReferenceUnresolved> {
    let reference =
        reference_resources
            .get(ref_idx)
            .ok_or(ReferenceUnresolved::IndexOutOfRange {
                index: ref_idx,
                len: reference_resources.len(),
            })?;
    resolve_reference_target(rel_path, reference)
}
