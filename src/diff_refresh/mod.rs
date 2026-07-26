//! LongLived runtime 差量刷新模块
//!
//! M54：定义 typed ChangeSet 契约（types）、真源接口（source），
//! 后续任务在此扩展 fixture/BI change source、文件 mirror 与编排器。
//!
//! `types` 不依赖 cli-local（graph_store 的 `IndexCommit` 需要引用
//! `DiffRefreshCheckpoint`）；source/fixture/mirror 依赖 session，
//! 仅在 `cli-local` 下编译。

#[cfg(feature = "cli-local")]
pub mod bi_meta_files_source;
#[cfg(feature = "cli-local")]
pub mod fixture_source;
#[cfg(feature = "cli-local")]
pub mod mirror;
#[cfg(feature = "cli-local")]
pub mod orchestrator;
#[cfg(feature = "cli-local")]
pub mod source;
#[cfg(feature = "cli-local")]
pub mod tick;
pub mod types;

#[cfg(feature = "cli-local")]
pub use bi_meta_files_source::{
    BackoffSchedule, BiActiveFileInfo, BiDeletedMetaFileInfo, BiMetaFilesChangeSource,
    BiMetaFilesTransport,
};
#[cfg(feature = "cli-local")]
pub use fixture_source::FixtureMetaFilesChangeSource;
#[cfg(feature = "cli-local")]
pub use mirror::{MirrorApplyReport, apply_changeset_to_mirror};
#[cfg(feature = "cli-local")]
pub use orchestrator::{DiffRefreshOrchestrator, DiffRefreshReport, DiffRefreshTiming};
#[cfg(feature = "cli-local")]
pub use source::MetaFilesChangeSource;
#[cfg(feature = "cli-local")]
pub use tick::{DiffRefreshTickReport, run_tick_loop_with_hooks};
pub use types::{
    ChangeSet, ChangedRemoteFile, DiffRefreshCheckpoint, MetaFilesWatermark, SourceCursor,
    cursor_accepts,
};
