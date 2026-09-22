//! M54 fixture META_FILES 变更源
//!
//! 从 JSON fixture 读取事件流与快照，实现 `MetaFilesChangeSource`，
//! 用于在无真实 BI 环境下验证差量刷新通路。
//!
//! fixture JSON schema（见 `tests/fixtures/diff_refresh/meta_files_delta.json`）：
//! - `schema_version: u32`：当前固定为 1；
//! - `events: ChangedRemoteFile[]`：poll 事件流，字段与 `ChangedRemoteFile` 对齐；
//!   `content_type` 使用 snake_case（`super_page`/`table`/`unknown`），
//!   `previous_source_path` 仅改名事件非 null；允许乱序与重复 event_id，
//!   poll 负责过滤、排序与去重；
//! - `snapshot.active`：bootstrap 活跃快照
//!   `{ file_id, source_path, revision, content_type, updated_at_ms }`；
//! - `snapshot.deleted`：bootstrap 删除快照
//!   `{ file_id, uuid, source_path, updated_at_ms }`。

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::source::MetaFilesChangeSource;
use super::types::{
    ChangeSet, ChangedRemoteFile, MetaFilesWatermark, advance_cursor, cursor_accepts,
    snapshot_boundary, sort_and_dedup_events,
};
use crate::remote_metadata::MetadataContentType;
use crate::session::{RemoteSessionFile, SessionManifest};

/// fixture META_FILES 变更源。
///
/// 内存中持有事件流与快照；`poll` 按双 cursor 过滤并推进水位，
/// `bootstrap` 对照 manifest 与快照产出差异事件。
pub struct FixtureMetaFilesChangeSource {
    events: Vec<ChangedRemoteFile>,
    snapshot: FixtureSnapshot,
}

/// fixture 文件顶层结构。
#[derive(Deserialize)]
struct FixtureFile {
    schema_version: u32,
    #[serde(default)]
    events: Vec<ChangedRemoteFile>,
    #[serde(default)]
    snapshot: FixtureSnapshot,
}

/// bootstrap 用快照：活跃文件清单 + 回收站删除清单。
#[derive(Default, Deserialize)]
struct FixtureSnapshot {
    #[serde(default)]
    active: Vec<FixtureActiveFile>,
    #[serde(default)]
    deleted: Vec<FixtureDeletedFile>,
}

/// 活跃快照条目：远端当前存在的文件及其修订与修改时间。
#[derive(Deserialize)]
struct FixtureActiveFile {
    file_id: String,
    source_path: String,
    revision: String,
    content_type: MetadataContentType,
    updated_at_ms: u64,
}

/// 删除快照条目：回收站墓碑，`uuid` 用于生成唯一 `deleted:<uuid>` 事件 ID。
///
/// JSON 中的 `source_path` 仅作文档性字段，删除事件路径以 manifest 记录为准。
#[derive(Deserialize)]
struct FixtureDeletedFile {
    file_id: String,
    uuid: String,
    updated_at_ms: u64,
}

impl FixtureMetaFilesChangeSource {
    /// 从 JSON 文本构造 fixture 变更源。
    pub fn from_json_str(json: &str) -> Result<Self> {
        let fixture: FixtureFile =
            serde_json::from_str(json).context("invalid diff refresh fixture json")?;
        anyhow::ensure!(
            fixture.schema_version == 1,
            "unsupported diff refresh fixture schema_version: {}",
            fixture.schema_version
        );
        Ok(Self {
            events: fixture.events,
            snapshot: fixture.snapshot,
        })
    }

    /// 从 JSON 文件构造 fixture 变更源。
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let json = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read diff refresh fixture: {}", path.display()))?;
        Self::from_json_str(&json)
    }
}

impl MetaFilesChangeSource for FixtureMetaFilesChangeSource {
    /// 按双 cursor 过滤事件：活跃事件走 `since.active`，删除事件走
    /// `since.deleted`；合并后按 `(updated_at_ms, event_id)` 升序，
    /// 以唯一 `event_id` 去重（回收站 file_id 可重复，不能作去重键）。
    fn poll(&self, since: &MetaFilesWatermark) -> Result<ChangeSet> {
        let accepted_active: Vec<ChangedRemoteFile> = self
            .events
            .iter()
            .filter(|event| !event.deleted)
            .filter(|event| cursor_accepts(&since.active, event.updated_at_ms, &event.event_id))
            .cloned()
            .collect();
        let accepted_deleted: Vec<ChangedRemoteFile> = self
            .events
            .iter()
            .filter(|event| event.deleted)
            .filter(|event| cursor_accepts(&since.deleted, event.updated_at_ms, &event.event_id))
            .cloned()
            .collect();

        let mut changed: Vec<ChangedRemoteFile> = accepted_active
            .iter()
            .chain(accepted_deleted.iter())
            .cloned()
            .collect();
        sort_and_dedup_events(&mut changed);

        let next_watermark = MetaFilesWatermark {
            active: advance_cursor(&since.active, &accepted_active),
            deleted: advance_cursor(&since.deleted, &accepted_deleted),
        };
        Ok(ChangeSet::new(changed, next_watermark))
    }

    /// 对照 manifest 与 fixture 快照：
    /// 活跃快照中 revision/path 与 manifest 不一致（或 manifest 缺失）的文件产出活跃事件；
    /// manifest 中未标删除但不在活跃快照的文件产出删除事件；
    /// 未变文件不产出事件。双 cursor 初始化到各自快照边界。
    fn bootstrap(&self, manifest: &SessionManifest) -> Result<ChangeSet> {
        // manifest 按 file_id 建索引；无 file_id 的记录无法对照快照，跳过
        let manifest_by_id: HashMap<&str, &RemoteSessionFile> = manifest
            .files
            .iter()
            .filter_map(|entry| entry.file_id.as_deref().map(|id| (id, entry)))
            .collect();

        let mut changed: Vec<ChangedRemoteFile> = Vec::new();
        for active in &self.snapshot.active {
            let previous = manifest_by_id.get(active.file_id.as_str());
            // 与真实 BI 源同一口径：镜像已获取不等于图已成功索引。
            let unchanged = previous.is_some_and(|entry| {
                !entry.deleted
                    && entry.revision.as_deref() == Some(active.revision.as_str())
                    && entry.source_path == active.source_path
                    && !entry.needs_index_retry()
            });
            if unchanged {
                continue;
            }
            // 路径不一致视为改名，记录旧路径供 mirror 先写新路径再删旧路径
            let previous_source_path = previous
                .filter(|entry| entry.source_path != active.source_path)
                .map(|entry| entry.source_path.clone());
            changed.push(ChangedRemoteFile {
                event_id: format!("active:{}:{}", active.file_id, active.revision),
                file_id: active.file_id.clone(),
                source_path: active.source_path.clone(),
                previous_source_path,
                content_type: active.content_type.clone(),
                updated_at_ms: active.updated_at_ms,
                deleted: false,
            });
        }

        // manifest 中未标删除、但活跃快照已缺失的文件产出删除事件；
        // 优先复用回收站墓碑的 uuid/时间，缺失时回退为 deleted:<file_id>
        let active_max_ms = self
            .snapshot
            .active
            .iter()
            .map(|active| active.updated_at_ms)
            .max()
            .unwrap_or(0);
        for entry in &manifest.files {
            if entry.deleted {
                continue;
            }
            let Some(file_id) = entry.file_id.as_deref() else {
                continue;
            };
            if self
                .snapshot
                .active
                .iter()
                .any(|active| active.file_id == file_id)
            {
                continue;
            }
            let tombstone = self
                .snapshot
                .deleted
                .iter()
                .find(|deleted| deleted.file_id == file_id);
            changed.push(ChangedRemoteFile {
                event_id: tombstone
                    .map(|deleted| format!("deleted:{}", deleted.uuid))
                    .unwrap_or_else(|| format!("deleted:{file_id}")),
                file_id: file_id.to_string(),
                source_path: entry.source_path.clone(),
                previous_source_path: None,
                content_type: MetadataContentType::from_extension(
                    entry.source_path.rsplit('.').next().unwrap_or(""),
                ),
                updated_at_ms: tombstone
                    .map(|deleted| deleted.updated_at_ms)
                    .unwrap_or(active_max_ms),
                deleted: true,
            });
        }

        sort_and_dedup_events(&mut changed);
        let next_watermark = MetaFilesWatermark {
            active: snapshot_boundary(self.snapshot.active.iter().map(|active| {
                (
                    active.updated_at_ms,
                    format!("active:{}:{}", active.file_id, active.revision),
                )
            })),
            deleted: snapshot_boundary(
                self.snapshot
                    .deleted
                    .iter()
                    .map(|deleted| (deleted.updated_at_ms, format!("deleted:{}", deleted.uuid))),
            ),
        };
        Ok(ChangeSet::new(changed, next_watermark))
    }
}
