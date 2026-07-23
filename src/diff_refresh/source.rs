//! M54 META_FILES 变更源接口
//!
//! `MetaFilesChangeSource` 统一 fixture 与 BI 真源的产出契约：
//! `poll` 按水位增量拉取，`bootstrap` 供无 checkpoint 的既有 session 安全初始化。

use anyhow::Result;

use super::types::{ChangeSet, MetaFilesWatermark};
use crate::session::SessionManifest;

/// META_FILES 变更源契约。
///
/// 实现方负责按真源字段语义（见 `types` 模块注释）生成事件，
/// 且任一事件非法时整次调用失败、双 cursor 均不推进。
pub trait MetaFilesChangeSource {
    /// 拉取严格晚于 `since` 的变更；无新事件时返回空集合且 cursor 原样保留。
    fn poll(&self, since: &MetaFilesWatermark) -> Result<ChangeSet>;

    /// 无 checkpoint 的既有 session 安全初始化：
    /// 对照 manifest 与远端快照，只产出新增/不一致/远端已缺失的文件，
    /// 并把 active/deleted cursor 初始化到各自快照边界。
    fn bootstrap(&self, manifest: &SessionManifest) -> Result<ChangeSet>;
}
