//! M54 差量刷新 typed ChangeSet 契约
//!
//! 固定真源字段语义：
//! - 活跃事件字段 `ID/PARENT_DIR/NAME/TYPE/modifyTime/revision`，
//!   `event_id = active:<file_id>:<revision>`；
//! - 删除事件字段 `FILE_ID/PARENT_DIR/NAME/TYPE/deleteTime/uuid`，
//!   `event_id = deleted:<uuid>`；
//! - 时间统一为 Unix 毫秒。

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

use crate::remote_metadata::MetadataContentType;

/// 单路（active 或 deleted）事件游标。
///
/// `boundary_event_ids` 记录 `updated_at_ms` 这一毫秒内已见的全部事件 ID，
/// 用于区分同毫秒的新事件；构造、反序列化与序列化前均排序去重。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCursor {
    /// 已见事件的最大 Unix 毫秒时间戳
    pub updated_at_ms: u64,
    /// `updated_at_ms` 毫秒内已见事件 ID（保持排序去重）
    pub boundary_event_ids: Vec<String>,
}

impl SourceCursor {
    /// 构造游标，对 `boundary_event_ids` 排序去重。
    pub fn new(updated_at_ms: u64, boundary_event_ids: Vec<String>) -> Self {
        Self {
            updated_at_ms,
            boundary_event_ids: normalize_boundary_event_ids(boundary_event_ids),
        }
    }
}

/// 对 boundary 事件 ID 排序去重，保证同毫秒边界比较与序列化输出稳定。
fn normalize_boundary_event_ids(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids.dedup();
    ids
}

/// 序列化前兜底排序去重：公开字段允许绕过构造函数赋值，此处保证输出稳定。
impl Serialize for SourceCursor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let normalized = normalize_boundary_event_ids(self.boundary_event_ids.clone());
        let mut state = serializer.serialize_struct("SourceCursor", 2)?;
        state.serialize_field("updated_at_ms", &self.updated_at_ms)?;
        state.serialize_field("boundary_event_ids", &normalized)?;
        state.end()
    }
}

/// 反序列化后走构造函数，恢复排序去重不变式。
impl<'de> Deserialize<'de> for SourceCursor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SourceCursorRaw {
            updated_at_ms: u64,
            #[serde(default)]
            boundary_event_ids: Vec<String>,
        }
        let raw = SourceCursorRaw::deserialize(deserializer)?;
        Ok(SourceCursor::new(raw.updated_at_ms, raw.boundary_event_ids))
    }
}

/// active/deleted 双游标水位。
///
/// 两路独立推进：deleted cursor 不会推进 active cursor，反之亦然。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetaFilesWatermark {
    pub active: SourceCursor,
    pub deleted: SourceCursor,
}

/// 单个远程文件变更事件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangedRemoteFile {
    /// 唯一事件 ID：`active:<file_id>:<revision>` 或 `deleted:<uuid>`
    pub event_id: String,
    /// 稳定文件 ID（改名事件保持同一 file_id）
    pub file_id: String,
    /// 当前项目内逻辑路径
    pub source_path: String,
    /// 改名前的路径；非改名事件为 None
    pub previous_source_path: Option<String>,
    pub content_type: MetadataContentType,
    /// 事件时间（Unix 毫秒）
    pub updated_at_ms: u64,
    /// 是否为删除事件
    pub deleted: bool,
}

/// graph+watermark 原子提交的 checkpoint。
///
/// 与 `MetaFilesWatermark` 结构相同，但语义上是持久化到 redb
/// `META_TABLE` 的提交单元；由 `ChangeSet.next_watermark` 转换而来。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffRefreshCheckpoint {
    pub active: SourceCursor,
    pub deleted: SourceCursor,
}

impl From<MetaFilesWatermark> for DiffRefreshCheckpoint {
    fn from(watermark: MetaFilesWatermark) -> Self {
        Self {
            active: watermark.active,
            deleted: watermark.deleted,
        }
    }
}

/// 一次 poll/bootstrap 产出的变更集合。
///
/// 不变式：`change_count == changed.len()`，必须使用 `ChangeSet::new` 构造。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangeSet {
    /// 合并后按 `(updated_at_ms, event_id)` 升序的变更事件
    pub changed: Vec<ChangedRemoteFile>,
    /// 冗余计数，恒等于 `changed.len()`
    pub change_count: usize,
    /// 消费完本集合后应推进到的水位
    pub next_watermark: MetaFilesWatermark,
}

impl ChangeSet {
    /// 构造 ChangeSet，`change_count` 由 `changed` 长度派生。
    pub fn new(changed: Vec<ChangedRemoteFile>, next_watermark: MetaFilesWatermark) -> Self {
        let change_count = changed.len();
        Self {
            changed,
            change_count,
            next_watermark,
        }
    }
}

/// 判定事件是否位于 cursor 之后（即应被本轮 poll 接收）。
///
/// 接受时间更大的事件；同毫秒时只接受未出现在
/// `boundary_event_ids` 中的事件，更早时间的事件一律拒绝。
pub fn cursor_accepts(cursor: &SourceCursor, updated_at_ms: u64, event_id: &str) -> bool {
    if updated_at_ms > cursor.updated_at_ms {
        return true;
    }
    if updated_at_ms < cursor.updated_at_ms {
        return false;
    }
    !cursor
        .boundary_event_ids
        .iter()
        .any(|seen| seen == event_id)
}

/// 按 `(updated_at_ms, event_id)` 升序排序并以唯一 event_id 去重（保留首个）。
///
/// 去重键使用唯一 event_id，不能用回收站可重复的 file_id。
pub(crate) fn sort_and_dedup_events(events: &mut Vec<ChangedRemoteFile>) {
    events.sort_by(|left, right| {
        left.updated_at_ms
            .cmp(&right.updated_at_ms)
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    events.dedup_by(|next, prev| next.event_id == prev.event_id);
}

/// 按单路已接收事件推进 cursor：
/// 无新事件时原样保留；有新事件时取该路最大事件时间，
/// boundary 保存该毫秒全部已见 IDs，最大时间等于旧 cursor 时间时与旧 boundary 取并集。
pub(crate) fn advance_cursor(
    cursor: &SourceCursor,
    accepted: &[ChangedRemoteFile],
) -> SourceCursor {
    let Some(max_ms) = accepted.iter().map(|event| event.updated_at_ms).max() else {
        return cursor.clone();
    };
    let mut boundary: Vec<String> = accepted
        .iter()
        .filter(|event| event.updated_at_ms == max_ms)
        .map(|event| event.event_id.clone())
        .collect();
    if max_ms == cursor.updated_at_ms {
        boundary.extend(cursor.boundary_event_ids.iter().cloned());
    }
    SourceCursor::new(max_ms, boundary)
}

/// 由快照条目（时间 + 事件 ID）构造边界 cursor：
/// 时间取快照最大毫秒，boundary 保存该毫秒全部事件 ID；空快照归零。
pub(crate) fn snapshot_boundary(entries: impl Iterator<Item = (u64, String)>) -> SourceCursor {
    let entries: Vec<(u64, String)> = entries.collect();
    let Some(max_ms) = entries.iter().map(|(ms, _)| *ms).max() else {
        return SourceCursor::new(0, Vec::new());
    };
    let boundary = entries
        .into_iter()
        .filter(|(ms, _)| *ms == max_ms)
        .map(|(_, event_id)| event_id)
        .collect();
    SourceCursor::new(max_ms, boundary)
}
