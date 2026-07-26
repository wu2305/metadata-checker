//! 远程元数据 session 基座
//!
//! M41 native 侧把远程项目同步到本地 session 目录后，继续复用现有 scanner/query。

pub mod auth;
pub mod manager;
pub mod manifest;
pub mod remote_provider;
pub mod remote_sync;
pub mod reqwest_provider;
pub mod sync;

use anyhow::{Context, Result};

pub use auth::{
    AccessTokenProvider, AuthContext, AuthProvider, AuthSessionErrorCode, AuthenticatedSession,
    SecretStore, SessionBootstrapper, StaticAuthProvider,
};
pub use manager::SessionManager;
pub use manifest::{RemoteSessionFile, SessionManifest};
pub use remote_provider::RemoteSessionProvider;
pub use remote_sync::{
    RefreshScope, RefreshScopeKind, RefreshScopeResolution, SessionRefreshDiagnostic,
    SessionRefreshFileReport, SessionRefreshFilter, SessionRefreshOptions, SessionRefreshReport,
    refresh_session_from_remote, resolve_refresh_scope, sync_project_from_remote_with_filter,
};
pub use reqwest_provider::ReqwestRemoteSessionProvider;
pub use sync::{SessionSyncMode, SessionSyncReport, sync_remote_files_to_session};

/// M55：进程内 diff refresh 绑定（启动时构造，不写入 manifest）。
///
/// 真实 BI 绑定持有已登录的 `ReqwestRemoteSessionProvider`（cookie jar 随
/// `reqwest::blocking::Client` clone 共享）、BI change source 与 SessionManager；
/// 测试可直接以 fixture source + stub provider 构造（fixture 不作为公开参数）。
/// 凭据只在 `bind_bi_session` 登录时消费，context 不保存 username/password，
/// 因此本类型不实现 Debug，避免意外输出内部状态。
pub struct DiffRefreshRuntimeContext {
    pub session_manager: SessionManager,
    pub session_id: String,
    pub manifest: SessionManifest,
    pub source: Box<dyn crate::diff_refresh::MetaFilesChangeSource>,
    pub provider: Box<dyn RemoteSessionProvider>,
}

impl DiffRefreshRuntimeContext {
    /// 真实 BI 绑定：读取 manifest、按 manifest 的 remote_server 登录、
    /// 构造 `BiMetaFilesChangeSource`。
    ///
    /// 登录一次后 clone provider：reqwest Client clone 共享 cookie jar，
    /// change source（active/deleted 清单）与 content provider（mirror fetch）
    /// 复用同一登录态。
    pub fn bind_bi_session(
        session_manager: SessionManager,
        session_id: &str,
        username: &str,
        password: &str,
    ) -> Result<Self> {
        let manifest = session_manager
            .read_manifest(session_id)
            .with_context(|| format!("failed to read session manifest for {session_id}"))?;
        let provider = ReqwestRemoteSessionProvider::new(&manifest.remote_server)
            .context("failed to create remote session provider")?;
        provider
            .login(username, password, "sys")
            .context("diff refresh remote login failed")?;
        let source = crate::diff_refresh::BiMetaFilesChangeSource::new(
            provider.clone(),
            manifest.project_ref.clone(),
        );
        Ok(Self {
            session_manager,
            session_id: session_id.to_string(),
            manifest,
            source: Box::new(source),
            provider: Box::new(provider),
        })
    }
}
