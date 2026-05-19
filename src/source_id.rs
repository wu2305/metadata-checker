use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 项目引用。
///
/// M36 起用于在结构化身份中标识项目范围，为跨项目链路预留身份。
/// 当前阶段不替换现有 graph node id，只在 SourceId / evidence / meta 中逐步引入。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRef {
    /// 命名空间，例如组织名或环境名。
    pub namespace: Option<String>,
    /// 项目标识，例如仓库名或项目 ID。
    pub project_id: String,
}

impl ProjectRef {
    /// 创建最小项目引用。
    pub fn new(project_id: impl Into<String>) -> Self {
        Self {
            namespace: None,
            project_id: project_id.into(),
        }
    }

    /// 带命名空间的项目引用。
    pub fn with_namespace(namespace: impl Into<String>, project_id: impl Into<String>) -> Self {
        Self {
            namespace: Some(namespace.into()),
            project_id: project_id.into(),
        }
    }
}

/// 来源标识。
///
/// M36 的边界定义：
/// - `source_path` 必须是项目内逻辑路径，例如 `app/.../*.spg`。
/// - 本地绝对路径、session local path、remote path 只能存在于 provider/session adapter。
/// - 先不改现有 graph node id，后续 evidence/meta 可逐步补 `project_ref + source_path`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceId {
    /// 项目引用，为跨项目链路预留结构化身份。
    pub project_ref: ProjectRef,
    /// 项目内逻辑路径，例如 `app/销售.app/销售/合同协议.spg`。
    pub source_path: String,
    /// 来源类型。
    pub source_kind: SourceKind,
    /// 来源方式：本地文件、会话缓存、远程 API、内存字节。
    pub origin: SourceOrigin,
    /// 可选版本/修订标识，例如 git commit 或同步版本号。
    pub revision: Option<String>,
}

/// 来源文件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Spg,
    Tbl,
    Unknown,
}

/// 来源获取方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceOrigin {
    /// 本地文件系统。
    Local,
    /// 会话/浏览器本地存储。
    Session,
    /// 远程 API。
    Remote,
    /// 内存字节（例如 WASM 传入或测试 fixture）。
    Memory,
}

impl SourceId {
    /// 从本地文件路径构造 SourceId。
    ///
    /// 若 `project_dir` 提供，则把绝对路径裁剪为相对 `source_path`；
    /// 否则保留原样（但仍建议调用方保证传入的是相对路径）。
    /// 从本地文件路径构造 SourceId。
    ///
    /// `source_path` 必须是项目内逻辑路径：
    /// - 提供 `project_dir` 时，从绝对路径裁剪出相对路径。
    /// - 未提供 `project_dir` 时，只允许相对路径；绝对路径被拒绝。
    pub fn from_local_path(
        project_ref: ProjectRef,
        path: &Path,
        project_dir: Option<&Path>,
    ) -> Result<Self> {
        let source_path = if let Some(base) = project_dir {
            path.strip_prefix(base)
                .map(|p| p.to_string_lossy().to_string())
                .with_context(|| {
                    format!(
                        "path {} is not inside project_dir {}",
                        path.display(),
                        base.display()
                    )
                })?
        } else if path.is_absolute() {
            anyhow::bail!(
                "absolute path {} cannot be used as source_path without project_dir. Use SourceId::from_memory for out-of-project sources or provide project_dir.",
                path.display()
            )
        } else {
            path.to_string_lossy().to_string()
        };

        if !is_project_internal_path(&source_path) {
            anyhow::bail!(
                "source_path '{}' is not a project-internal relative path",
                source_path
            );
        }

        let source_kind = if path.extension().map(|e| e == "spg").unwrap_or(false) {
            SourceKind::Spg
        } else if path.extension().map(|e| e == "tbl").unwrap_or(false) {
            SourceKind::Tbl
        } else {
            SourceKind::Unknown
        };

        Ok(Self {
            project_ref,
            source_path,
            source_kind,
            origin: SourceOrigin::Local,
            revision: None,
        })
    }

    pub fn from_memory(project_ref: ProjectRef, source_path: impl Into<String>) -> Self {
        Self {
            project_ref,
            source_path: source_path.into(),
            source_kind: SourceKind::Unknown,
            origin: SourceOrigin::Memory,
            revision: None,
        }
    }

    /// 将 source_path 解析为 Path 片段，用于拼接本地绝对路径。
    pub fn as_path(&self) -> &str {
        &self.source_path
    }
}

/// 判断路径是否为项目内逻辑路径。
///
/// 拒绝以 `/` 或 `\` 开头的绝对路径，以及包含 `..` 的逃逸路径。
pub fn is_project_internal_path(path: &str) -> bool {
    if path.starts_with('/') || path.starts_with('\\') {
        return false;
    }
    if cfg!(windows) && path.len() >= 2 && path.as_bytes()[1] == b':' {
        return false;
    }
    // 拒绝包含 .. 的路径片段
    path.split(['/', '\\']).any(|segment| segment == "..") == false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_ref_new() {
        let pr = ProjectRef::new("my_project");
        assert_eq!(pr.project_id, "my_project");
        assert!(pr.namespace.is_none());
    }

    #[test]
    fn test_project_ref_with_namespace() {
        let pr = ProjectRef::with_namespace("acme", "sales");
        assert_eq!(pr.namespace, Some("acme".to_string()));
        assert_eq!(pr.project_id, "sales".to_string());
    }

    #[test]
    fn test_source_id_from_local_path_relative() {
        let pr = ProjectRef::new("p1");
        let sid = SourceId::from_local_path(pr.clone(), Path::new("app/page.spg"), None).unwrap();
        assert_eq!(sid.source_path, "app/page.spg");
        assert_eq!(sid.source_kind, SourceKind::Spg);
        assert_eq!(sid.origin, SourceOrigin::Local);
        assert_eq!(sid.project_ref, pr);
    }

    #[test]
    fn test_source_id_from_local_path_strips_project_dir() {
        let pr = ProjectRef::new("p1");
        let sid = SourceId::from_local_path(
            pr,
            Path::new("/tmp/proj/app/page.spg"),
            Some(Path::new("/tmp/proj")),
        )
        .unwrap();
        assert_eq!(sid.source_path, "app/page.spg");
    }

    #[test]
    fn test_source_id_from_memory() {
        let pr = ProjectRef::new("p1");
        let sid = SourceId::from_memory(pr, "test.spg");
        assert_eq!(sid.source_path, "test.spg");
        assert_eq!(sid.origin, SourceOrigin::Memory);
    }

    #[test]
    fn test_is_project_internal_path_accepts_relative() {
        assert!(is_project_internal_path("app/page.spg"));
        assert!(is_project_internal_path("data/tables/bind.tbl"));
    }

    #[test]
    fn test_is_project_internal_path_rejects_absolute() {
        assert!(!is_project_internal_path("/app/page.spg"));
        assert!(!is_project_internal_path("\\app\\page.spg"));
    }

    #[test]
    fn test_is_project_internal_path_rejects_parent_escape() {
        assert!(!is_project_internal_path("../secret.spg"));
        assert!(!is_project_internal_path("app/../../../secret.spg"));
    }
}

#[test]
fn test_source_id_from_local_path_accepts_relative_without_project_dir() {
    let pr = ProjectRef::new("p1");
    let sid = SourceId::from_local_path(pr, Path::new("app/page.spg"), None).unwrap();
    assert_eq!(sid.source_path, "app/page.spg");
}

#[test]
fn test_source_id_from_local_path_strips_prefix_with_project_dir() {
    let pr = ProjectRef::new("p1");
    let sid = SourceId::from_local_path(
        pr,
        Path::new("/tmp/proj/app/page.spg"),
        Some(Path::new("/tmp/proj")),
    )
    .unwrap();
    assert_eq!(sid.source_path, "app/page.spg");
}

#[test]
fn test_source_id_from_local_path_rejects_absolute_without_project_dir() {
    let pr = ProjectRef::new("p1");
    let result = SourceId::from_local_path(pr, Path::new("/tmp/page.spg"), None);
    assert!(
        result.is_err(),
        "absolute path without project_dir must be rejected"
    );
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("absolute path"),
        "error must mention absolute path"
    );
}
