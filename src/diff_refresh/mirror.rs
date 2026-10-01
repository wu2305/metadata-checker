//! M54 文件 mirror 差量应用
//!
//! 把 `ChangeSet` 中的变更事件按序应用到 session 项目镜像：
//! 活跃事件拉取内容后原子写入，删除事件移除镜像文件并把 manifest
//! 记录标为 deleted，改名事件先写新路径再删除同 file_id 旧路径，
//! 防止失败时丢失唯一副本。
//!
//! manifest 只在本函数内做内存更新，持久化沿用既有
//! `SessionManager::write_manifest` 路径，由调用方（orchestrator）负责。
//!
//! **失败回滚**：任一事件失败时，内存 manifest 回滚到调用前快照，
//! 保证内存与磁盘一致（调用方不写盘）。

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};

use super::types::{ChangeSet, ChangedRemoteFile};
use crate::remote_metadata::RemoteFileRef;
use crate::session::RemoteSessionProvider;
use crate::session::manifest::{RemoteSessionFile, SessionManifest};
use crate::session::sync::{
    atomic_write_text, checked_relative_path, hash_text, project_mirror_root,
};

/// mirror 差量应用统计。
///
/// 四类计数互斥：改名只计入 `renamed`，不再计入 `written`。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirrorApplyReport {
    /// 原子写入的非改名文件数。
    pub written: usize,
    /// 删除的文件数。
    pub deleted: usize,
    /// 改名（同 file_id 路径迁移）文件数。
    pub renamed: usize,
    /// 内容与本地 hash 一致、跳过写入的文件数。
    pub unchanged: usize,
}

/// 把 ChangeSet 按事件顺序应用到 session 镜像并更新 manifest（内存）。
///
/// 非删除事件逐个调用 provider 拉取内容（fetch 次数等于非删除事件数）；
/// 任一事件失败即返回 Err，内存 manifest 回滚到调用前快照，
/// 调用方不得在推进 checkpoint 前忽略该错误。
pub fn apply_changeset_to_mirror(
    session_dir: &Path,
    manifest: &mut SessionManifest,
    changeset: &ChangeSet,
    provider: &dyn RemoteSessionProvider,
) -> Result<MirrorApplyReport> {
    let project_root = project_mirror_root(session_dir);
    fs::create_dir_all(&project_root).with_context(|| {
        format!(
            "failed to create session project mirror {}",
            project_root.display()
        )
    })?;

    // 快照 manifest，失败时回滚（保证内存与磁盘一致）
    let manifest_snapshot = manifest.clone();
    let mut report = MirrorApplyReport::default();
    for event in &changeset.changed {
        if event.deleted {
            if let Err(error) = apply_delete_event(&project_root, manifest, event, &mut report) {
                *manifest = manifest_snapshot;
                return Err(error);
            }
        } else if let Err(error) =
            apply_active_event(&project_root, manifest, event, provider, &mut report)
        {
            *manifest = manifest_snapshot;
            return Err(error);
        }
    }
    Ok(report)
}

/// 应用删除事件：移除镜像文件并把 manifest 记录标为 deleted；
/// manifest 无记录时补一条墓碑记录，与既有 full-sync 删除语义对齐。
fn apply_delete_event(
    project_root: &Path,
    manifest: &mut SessionManifest,
    event: &ChangedRemoteFile,
    report: &mut MirrorApplyReport,
) -> Result<()> {
    let record_index = manifest
        .files
        .iter()
        .position(|file| file.file_id.as_deref() == Some(event.file_id.as_str()));
    let mirror_source_path = record_index
        .map(|index| manifest.files[index].source_path.clone())
        .unwrap_or_else(|| event.source_path.clone());

    let mirror_path = project_root.join(checked_relative_path(&mirror_source_path)?);
    if mirror_path.exists() {
        fs::remove_file(&mirror_path).with_context(|| {
            format!(
                "failed to remove deleted session file {}",
                mirror_path.display()
            )
        })?;
    }

    if let Some(index) = record_index {
        manifest.files[index].deleted = true;
    } else {
        manifest.files.push(RemoteSessionFile {
            source_path: event.source_path.clone(),
            file_id: Some(event.file_id.clone()),
            revision: None,
            etag: None,
            mtime: Some(event.updated_at_ms),
            size: None,
            hash: None,
            indexed_hash: None,
            deleted: true,
        });
        manifest
            .files
            .sort_by(|left, right| left.source_path.cmp(&right.source_path));
    }
    report.deleted += 1;
    Ok(())
}

/// 应用活跃事件：拉取内容，hash 一致则跳过写入；
/// 改名时先写新路径再删旧路径，manifest 只保留新路径记录。
fn apply_active_event(
    project_root: &Path,
    manifest: &mut SessionManifest,
    event: &ChangedRemoteFile,
    provider: &dyn RemoteSessionProvider,
    report: &mut MirrorApplyReport,
) -> Result<()> {
    let file_ref = RemoteFileRef::try_new(
        &manifest.project_ref,
        &event.source_path,
        Some(event.file_id.clone()),
    )
    .map_err(|error| anyhow!("invalid diff refresh file ref: {error}"))?;
    let content = provider
        .fetch_metafile_content(&file_ref)
        .with_context(|| format!("failed to fetch diff refresh file {}", event.source_path))?;
    let hash = hash_text(&content.raw_text);

    // 旧路径优先取事件携带的 previous_source_path；
    // 未携带时按 file_id 在 manifest 中定位，路径不一致同样视为改名
    let record_index = manifest
        .files
        .iter()
        .position(|file| file.file_id.as_deref() == Some(event.file_id.as_str()));
    let previous_path = event
        .previous_source_path
        .clone()
        .or_else(|| {
            record_index
                .map(|index| manifest.files[index].source_path.clone())
                .filter(|path| path != &event.source_path)
        })
        .filter(|path| path != &event.source_path);

    // M59-2 B：改名时先取走旧记录的 `indexed_hash`，后面的 `retain` 会把旧
    // 记录摘掉（改名后 manifest 只保留新路径），此时再按 index 取会越界。
    // `indexed_hash` 是「最近一次成功入图的内容 hash」，必须随 file_id 迁移。
    let carried_indexed_hash = record_index
        .map(|index| manifest.files[index].indexed_hash.clone())
        .unwrap_or(None);

    // 内容未变且非改名：跳过写入，只保留既有记录
    let unchanged = previous_path.is_none()
        && record_index.is_some_and(|index| {
            let record = &manifest.files[index];
            !record.deleted && record.hash.as_deref() == Some(hash.as_str())
        });
    if unchanged {
        report.unchanged += 1;
        return Ok(());
    }

    // 先写新路径（原子写），成功后再删旧路径，保证任何失败都至少保留一份副本
    let new_path = project_root.join(checked_relative_path(&event.source_path)?);
    atomic_write_text(&new_path, &content.raw_text)?;

    if let Some(previous_path) = &previous_path {
        let old_path = project_root.join(checked_relative_path(previous_path)?);
        if old_path.exists() {
            fs::remove_file(&old_path).with_context(|| {
                format!(
                    "failed to remove renamed session file {}",
                    old_path.display()
                )
            })?;
        }
        // 移除旧路径记录，保证改名后 manifest 只保留新路径
        manifest
            .files
            .retain(|file| file.source_path != *previous_path);
    }

    // upsert 新路径 manifest 记录（改名后只保留新路径）
    // M59-2 B：`hash` 记录「镜像已获取到这个内容」；`indexed_hash` 保持调用方
    // 传入的既有值——它只在文件成功解析并入图后才由 orchestrator 推进。
    // 解析失败时两者失配，下一轮 bootstrap 据此重投重试。
    let record = RemoteSessionFile {
        source_path: event.source_path.clone(),
        file_id: Some(event.file_id.clone()),
        revision: content.revision.clone(),
        etag: None,
        mtime: Some(event.updated_at_ms),
        size: Some(content.raw_text.len() as u64),
        hash: Some(hash.clone()),
        indexed_hash: carried_indexed_hash,
        deleted: false,
    };
    let upsert_index = manifest
        .files
        .iter()
        .position(|file| file.source_path == event.source_path);
    if let Some(index) = upsert_index {
        manifest.files[index] = record;
    } else {
        manifest.files.push(record);
        manifest
            .files
            .sort_by(|left, right| left.source_path.cmp(&right.source_path));
    }

    if previous_path.is_some() {
        report.renamed += 1;
    } else {
        report.written += 1;
    }
    Ok(())
}
