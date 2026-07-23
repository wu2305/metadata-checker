//! M55 BI META_FILES 活跃/删除事件变更源
//!
//! - 活跃清单复用 `GET /api/meta/services/getFileDescendant/{project_ref}`
//!   （含 `list_metafiles` fallback），完整数组、无分页；
//! - 删除清单走 `POST /api/meta/file/getRecyclebinFiles`，完整数组、无分页；
//! - 双 cursor 独立过滤与推进（active/deleted 互不影响）；
//! - HTTP 传输经 `BiMetaFilesTransport` 注入：真实环境用
//!   `ReqwestRemoteSessionProvider`，测试注入录制 fixture test double，
//!   真实 reqwest 不打外网。
//!
//! 错误语义：active 事件缺失/非法时间、file_id 或 revision →
//! `INVALID_ACTIVE_CHANGE_EVENT`；删除事件缺失 uuid 或 deleteTime →
//! `INVALID_DELETE_EVENT_ID`；任一错误使整次 poll 失败、双 cursor 均不推进。
//! bootstrap 的字段错误统一返回 `DIFF_REFRESH_BOOTSTRAP_FAILED`。

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Deserializer};

use super::source::MetaFilesChangeSource;
use super::types::{
    ChangeSet, ChangedRemoteFile, MetaFilesWatermark, advance_cursor, cursor_accepts,
    snapshot_boundary, sort_and_dedup_events,
};
use crate::remote_metadata::MetadataContentType;
use crate::session::RemoteSessionProvider;
use crate::session::manifest::{RemoteSessionFile, SessionManifest};
use crate::session::reqwest_provider::{ReqwestRemoteSessionProvider, normalize_source_path};

/// 归一化活跃文件信息（active 事件映射输入）。
///
/// 字段对齐 BI 响应：`id`、`path`（或 `parentDir + name` 兜底）、
/// `type`/`isFolder`、`modifyTime: u64`、`revision`。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiActiveFileInfo {
    #[serde(default, alias = "id", alias = "fileId", alias = "FILE_ID")]
    pub file_id: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub parent_dir: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type", default, alias = "TYPE")]
    pub file_type: Option<String>,
    #[serde(default)]
    pub is_folder: bool,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub revision: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_u64")]
    pub modify_time: Option<u64>,
}

/// 归一化 BI 资源路径为项目内逻辑路径。
///
/// `normalize_source_path` 只剥 `/proj/` 前缀（带前导斜杠）；
/// BI 响应也可能返回无前导斜杠的 `proj/...`，此处两种形式都容忍。
fn normalize_bi_resource_path(raw: &str, project_ref: &str) -> String {
    let normalized = normalize_source_path(raw, project_ref);
    let prefix = format!("{project_ref}/");
    normalized
        .strip_prefix(&prefix)
        .map(str::to_string)
        .unwrap_or(normalized)
}

impl BiActiveFileInfo {
    /// 还原项目内逻辑路径：`path` 优先，`parentDir + name` 兜底。
    fn resolved_source_path(&self, project_ref: &str) -> Option<String> {
        let raw = self
            .path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(|path| path.trim().to_string())
            .or_else(|| {
                let name = self.name.as_deref()?.trim();
                if name.is_empty() {
                    return None;
                }
                let parent = self
                    .parent_dir
                    .as_deref()
                    .filter(|parent| !parent.trim().is_empty())?;
                Some(format!("{}/{}", parent.trim_end_matches('/'), name))
            })?;
        let normalized = normalize_bi_resource_path(&raw, project_ref);
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    }

    /// 内容类型：`type` 字段优先，解析路径扩展名兜底。
    fn content_type(&self, source_path: &str) -> MetadataContentType {
        let ext = self
            .file_type
            .as_deref()
            .unwrap_or_else(|| source_path.rsplit('.').next().unwrap_or(""));
        MetadataContentType::from_extension(ext)
    }

    /// 是否可分析（spg/tbl 且非文件夹）。
    ///
    /// 真实 BI 活跃清单含 ts/docx/json 等非 spg/tbl 条目：source 侧在
    /// 事件映射与字段校验之前过滤，避免 mirror 对 docx 发起 fetch、
    /// bootstrap 把 docx 当成「新增文件」，以及未知类型缺 revision 拖垮整次 poll。
    fn is_analyzable(&self, project_ref: &str) -> bool {
        if self.is_folder {
            return false;
        }
        let Some(source_path) = self.resolved_source_path(project_ref) else {
            return false;
        };
        matches!(
            self.content_type(&source_path),
            MetadataContentType::SuperPage | MetadataContentType::Table
        )
    }
}

/// 回收站删除文件信息（deleted 事件映射输入）。
///
/// 字段对齐 BI 响应：`FILE_ID/PARENT_DIR/NAME/TYPE/deleteTime/uuid`
/// （同时容忍 camelCase 变体）；`file_id` 取原始 id（回收站中可重复），
/// 事件唯一性只依赖 `uuid`。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiDeletedMetaFileInfo {
    #[serde(default, alias = "id", alias = "FILE_ID")]
    pub file_id: Option<String>,
    #[serde(default, alias = "PARENT_DIR")]
    pub parent_dir: Option<String>,
    #[serde(default, alias = "NAME")]
    pub name: Option<String>,
    #[serde(rename = "type", default, alias = "TYPE")]
    pub file_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_u64")]
    pub delete_time: Option<u64>,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub is_folder: bool,
}

impl BiDeletedMetaFileInfo {
    /// 由 parentDir + name 还原项目内逻辑路径。
    fn resolved_source_path(&self, project_ref: &str) -> Option<String> {
        let name = self.name.as_deref()?.trim();
        if name.is_empty() {
            return None;
        }
        let parent = self
            .parent_dir
            .as_deref()
            .filter(|parent| !parent.trim().is_empty())?;
        let raw = format!("{}/{}", parent.trim_end_matches('/'), name);
        let normalized = normalize_bi_resource_path(&raw, project_ref);
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    }

    /// 内容类型：`TYPE` 字段优先，路径扩展名兜底。
    fn content_type(&self, source_path: &str) -> MetadataContentType {
        let ext = self
            .file_type
            .as_deref()
            .unwrap_or_else(|| source_path.rsplit('.').next().unwrap_or(""));
        MetadataContentType::from_extension(ext)
    }

    /// 是否可分析（spg/tbl 且非文件夹墓碑）。
    ///
    /// 真实回收站含 22 种类型与文件夹墓碑：非 spg/tbl 不产生删除事件
    /// （manifest 只登记 spg/tbl），但 bootstrap 的 deleted cursor 仍覆盖
    /// 全部墓碑，避免旧墓碑在后续 poll 中被重放。
    fn is_analyzable(&self, project_ref: &str) -> bool {
        if self.is_folder {
            return false;
        }
        let Some(source_path) = self.resolved_source_path(project_ref) else {
            return false;
        };
        matches!(
            self.content_type(&source_path),
            MetadataContentType::SuperPage | MetadataContentType::Table
        )
    }
}

/// BI META_FILES 清单传输抽象（注入录制 fixture 或真实 reqwest 实现）。
pub trait BiMetaFilesTransport {
    /// 拉取活跃文件完整清单（完整数组、无分页）。
    fn list_active_files(&self, project_ref: &str) -> Result<Vec<BiActiveFileInfo>>;
    /// 拉取回收站删除文件完整清单（完整数组、无分页）。
    fn list_deleted_files(&self, project_ref: &str) -> Result<Vec<BiDeletedMetaFileInfo>>;
}

impl BiMetaFilesTransport for ReqwestRemoteSessionProvider {
    /// 复用现有 getFileDescendant 与 list_metafiles fallback。
    fn list_active_files(&self, project_ref: &str) -> Result<Vec<BiActiveFileInfo>> {
        let entries = RemoteSessionProvider::list_metafiles(self, project_ref)?;
        Ok(entries
            .into_iter()
            .map(|entry| BiActiveFileInfo {
                file_id: entry.file_id,
                path: Some(entry.source_path),
                parent_dir: None,
                name: None,
                file_type: None,
                is_folder: false,
                revision: entry.revision,
                modify_time: entry.mtime,
            })
            .collect())
    }

    fn list_deleted_files(&self, project_ref: &str) -> Result<Vec<BiDeletedMetaFileInfo>> {
        self.list_recyclebin_files(project_ref)
    }
}

/// BI META_FILES 变更源。
pub struct BiMetaFilesChangeSource<T: BiMetaFilesTransport> {
    transport: T,
    project_ref: String,
}

impl<T: BiMetaFilesTransport> BiMetaFilesChangeSource<T> {
    pub fn new(transport: T, project_ref: impl Into<String>) -> Self {
        Self {
            transport,
            project_ref: project_ref.into(),
        }
    }

    /// 映射 active 事件；字段缺失/非法返回 `INVALID_ACTIVE_CHANGE_EVENT`。
    fn map_active_event(&self, info: &BiActiveFileInfo) -> Result<ChangedRemoteFile> {
        let file_id = info
            .file_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| anyhow!("INVALID_ACTIVE_CHANGE_EVENT: active entry missing id"))?;
        let revision = info
            .revision
            .as_deref()
            .filter(|revision| !revision.is_empty())
            .ok_or_else(|| anyhow!("INVALID_ACTIVE_CHANGE_EVENT: active entry missing revision"))?;
        let updated_at_ms = info.modify_time.ok_or_else(|| {
            anyhow!("INVALID_ACTIVE_CHANGE_EVENT: active entry missing/invalid modifyTime")
        })?;
        let source_path = info
            .resolved_source_path(&self.project_ref)
            .ok_or_else(|| anyhow!("INVALID_ACTIVE_CHANGE_EVENT: active entry missing path"))?;
        Ok(ChangedRemoteFile {
            event_id: format!("active:{file_id}:{revision}"),
            file_id: file_id.to_string(),
            source_path: source_path.clone(),
            previous_source_path: None,
            content_type: MetadataContentType::from_extension(
                source_path.rsplit('.').next().unwrap_or(""),
            ),
            updated_at_ms,
            deleted: false,
        })
    }

    /// 映射 deleted 事件；缺失 uuid/deleteTime 返回 `INVALID_DELETE_EVENT_ID`。
    fn map_deleted_event(&self, info: &BiDeletedMetaFileInfo) -> Result<ChangedRemoteFile> {
        let uuid = info
            .uuid
            .as_deref()
            .filter(|uuid| !uuid.is_empty())
            .ok_or_else(|| anyhow!("INVALID_DELETE_EVENT_ID: deleted entry missing uuid"))?;
        let updated_at_ms = info.delete_time.ok_or_else(|| {
            anyhow!("INVALID_DELETE_EVENT_ID: deleted entry missing/invalid deleteTime")
        })?;
        let file_id = info
            .file_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| anyhow!("INVALID_DELETE_EVENT_ID: deleted entry missing file id"))?;
        let source_path = info
            .resolved_source_path(&self.project_ref)
            .ok_or_else(|| anyhow!("INVALID_DELETE_EVENT_ID: deleted entry missing path"))?;
        Ok(ChangedRemoteFile {
            event_id: format!("deleted:{uuid}"),
            file_id: file_id.to_string(),
            content_type: info.content_type(&source_path),
            source_path,
            previous_source_path: None,
            updated_at_ms,
            deleted: true,
        })
    }
}

impl<T: BiMetaFilesTransport> MetaFilesChangeSource for BiMetaFilesChangeSource<T> {
    /// 双 cursor 独立过滤：active 只走 `since.active`，deleted 只走
    /// `since.deleted`；任一映射错误使整次 poll 失败、双 cursor 均不推进。
    fn poll(&self, since: &MetaFilesWatermark) -> Result<ChangeSet> {
        let active_list = self.transport.list_active_files(&self.project_ref)?;
        let deleted_list = self.transport.list_deleted_files(&self.project_ref)?;

        let mut accepted_active: Vec<ChangedRemoteFile> = Vec::new();
        for info in &active_list {
            // 先过滤非 spg/tbl 与文件夹（真实清单含 ts/docx/json 等），
            // 避免对无关文件做字段校验、产生事件并触发 mirror fetch
            if !info.is_analyzable(&self.project_ref) {
                continue;
            }
            let event = self.map_active_event(info)?;
            if cursor_accepts(&since.active, event.updated_at_ms, &event.event_id) {
                accepted_active.push(event);
            }
        }
        let mut accepted_deleted: Vec<ChangedRemoteFile> = Vec::new();
        for info in &deleted_list {
            if !info.is_analyzable(&self.project_ref) {
                continue;
            }
            let event = self.map_deleted_event(info)?;
            if cursor_accepts(&since.deleted, event.updated_at_ms, &event.event_id) {
                accepted_deleted.push(event);
            }
        }

        let next_watermark = MetaFilesWatermark {
            active: advance_cursor(&since.active, &accepted_active),
            deleted: advance_cursor(&since.deleted, &accepted_deleted),
        };
        let mut changed: Vec<ChangedRemoteFile> = accepted_active
            .into_iter()
            .chain(accepted_deleted)
            .collect();
        sort_and_dedup_events(&mut changed);
        Ok(ChangeSet::new(changed, next_watermark))
    }

    /// 真实 bootstrap：固定请求顺序 deleted 完整快照 → active 完整快照。
    ///
    /// active 按 file_id 对照 manifest 的 revision/path，只产出新增或不一致
    /// 文件；manifest 中 active 快照已缺失的文件产出删除；历史 recycle-bin
    /// 仅用于初始化 deleted cursor，不逐条重放。字段错误返回
    /// `DIFF_REFRESH_BOOTSTRAP_FAILED`。
    fn bootstrap(&self, manifest: &SessionManifest) -> Result<ChangeSet> {
        // 1. deleted 完整快照：只初始化 deleted cursor + 建墓碑索引，不重放事件
        let deleted_list = self
            .transport
            .list_deleted_files(&self.project_ref)
            .context("DIFF_REFRESH_BOOTSTRAP_FAILED: fetch deleted snapshot")?;
        let mut tombstones: HashMap<String, &BiDeletedMetaFileInfo> = HashMap::new();
        let mut deleted_boundary: Vec<(u64, String)> = Vec::new();
        for info in &deleted_list {
            let uuid = info
                .uuid
                .as_deref()
                .filter(|uuid| !uuid.is_empty())
                .ok_or_else(|| {
                    anyhow!("DIFF_REFRESH_BOOTSTRAP_FAILED: deleted entry missing uuid")
                })?;
            let delete_time = info.delete_time.ok_or_else(|| {
                anyhow!("DIFF_REFRESH_BOOTSTRAP_FAILED: deleted entry missing deleteTime")
            })?;
            deleted_boundary.push((delete_time, format!("deleted:{uuid}")));
            if let Some(file_id) = info.file_id.as_deref().filter(|id| !id.is_empty()) {
                // 同一 file_id 多次删除时保留最新墓碑（deleteTime 最大）
                let replace = tombstones
                    .get(file_id)
                    .is_none_or(|existing| existing.delete_time.unwrap_or(0) <= delete_time);
                if replace {
                    tombstones.insert(file_id.to_string(), info);
                }
            }
        }

        // 2. active 完整快照：按 file_id 对照 manifest，只产出新增/不一致文件
        let active_list = self
            .transport
            .list_active_files(&self.project_ref)
            .context("DIFF_REFRESH_BOOTSTRAP_FAILED: fetch active snapshot")?;
        let manifest_by_id: HashMap<&str, &RemoteSessionFile> = manifest
            .files
            .iter()
            .filter_map(|entry| entry.file_id.as_deref().map(|id| (id, entry)))
            .collect();

        let mut changed: Vec<ChangedRemoteFile> = Vec::new();
        let mut active_boundary: Vec<(u64, String)> = Vec::new();
        for info in &active_list {
            if info.is_folder {
                continue;
            }
            if !info.is_analyzable(&self.project_ref) {
                // 非 spg/tbl 文件（docx/ts/json 等）：不产出事件、不做严格
                // 字段校验，但 cursor 仍覆盖该快照条目，避免后续 poll 反复评估
                if let (Some(id), Some(revision), Some(ms)) = (
                    info.file_id.as_deref().filter(|id| !id.is_empty()),
                    info.revision.as_deref().filter(|rev| !rev.is_empty()),
                    info.modify_time,
                ) {
                    active_boundary.push((ms, format!("active:{id}:{revision}")));
                }
                continue;
            }
            let event = self
                .map_active_event(info)
                .map_err(|error| anyhow!("DIFF_REFRESH_BOOTSTRAP_FAILED: {error:#}"))?;
            active_boundary.push((event.updated_at_ms, event.event_id.clone()));

            let unchanged = manifest_by_id
                .get(event.file_id.as_str())
                .is_some_and(|entry| {
                    !entry.deleted
                        && entry.revision.as_deref() == info.revision.as_deref()
                        && entry.source_path == event.source_path
                });
            if unchanged {
                continue;
            }
            // 路径不一致视为改名，记录旧路径供 mirror 先写新路径再删旧路径
            let previous_source_path = manifest_by_id
                .get(event.file_id.as_str())
                .filter(|entry| entry.source_path != event.source_path)
                .map(|entry| entry.source_path.clone());
            changed.push(ChangedRemoteFile {
                previous_source_path,
                ..event
            });
        }

        // 3. manifest 中未标删除、但 active 快照已缺失的文件产出删除
        let active_max_ms = active_boundary.iter().map(|(ms, _)| *ms).max().unwrap_or(0);
        for entry in &manifest.files {
            if entry.deleted {
                continue;
            }
            let Some(file_id) = entry.file_id.as_deref() else {
                continue;
            };
            let still_active = active_list
                .iter()
                .any(|info| !info.is_folder && info.file_id.as_deref() == Some(file_id));
            if still_active {
                continue;
            }
            let tombstone = tombstones.get(file_id);
            let source_path = entry.source_path.clone();
            changed.push(ChangedRemoteFile {
                event_id: tombstone
                    .and_then(|info| info.uuid.clone())
                    .map(|uuid| format!("deleted:{uuid}"))
                    .unwrap_or_else(|| format!("deleted:{file_id}")),
                file_id: file_id.to_string(),
                content_type: MetadataContentType::from_extension(
                    source_path.rsplit('.').next().unwrap_or(""),
                ),
                source_path,
                previous_source_path: None,
                updated_at_ms: tombstone
                    .and_then(|info| info.delete_time)
                    .unwrap_or(active_max_ms),
                deleted: true,
            });
        }

        sort_and_dedup_events(&mut changed);
        let next_watermark = MetaFilesWatermark {
            active: snapshot_boundary(active_boundary.into_iter()),
            deleted: snapshot_boundary(deleted_boundary.into_iter()),
        };
        Ok(ChangeSet::new(changed, next_watermark))
    }
}

/// tick 侧连续网络错误指数退避计算器（1/2/4/8/16/32/60 秒，上限 60 秒）。
///
/// 放在 source 侧而非 orchestrator：退避只针对网络错误，数据错误应直接
/// 失败上报、不退避，错误分类与 source 同处一地；真实 tick 循环由上层组装。
/// 不得按 change_count 跳过非空集合（空集合零等待由调用方语义保证）。
#[derive(Debug, Default, Clone)]
pub struct BackoffSchedule {
    consecutive_failures: u32,
}

impl BackoffSchedule {
    /// 退避上限（秒）。
    const MAX_DELAY_SECS: u64 = 60;

    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一次连续失败，返回本次应等待的秒数。
    pub fn record_failure(&mut self) -> u64 {
        let delay = 1u64
            .checked_shl(self.consecutive_failures.min(6))
            .unwrap_or(0)
            .min(Self::MAX_DELAY_SECS);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        delay
    }

    /// 成功 poll 立即清零失败计数。
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    /// 当前连续失败次数。
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }
}

/// 将 BI 可能返回的字符串/数字版本号统一反序列化为字符串。
fn deserialize_optional_string<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| match value {
        serde_json::Value::String(text) => Some(text),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }))
}

/// 容忍字符串/数字时间；非法值归一为 None，由上层按字段缺失报错。
fn deserialize_optional_u64<'de, D>(deserializer: D) -> std::result::Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| match value {
        serde_json::Value::String(text) => text.parse::<u64>().ok(),
        serde_json::Value::Number(number) => number.as_u64(),
        _ => None,
    }))
}
