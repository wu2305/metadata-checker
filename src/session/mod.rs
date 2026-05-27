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

pub use auth::{
    AccessTokenProvider, AuthContext, AuthProvider, AuthSessionErrorCode, AuthenticatedSession,
    SecretStore, SessionBootstrapper, StaticAuthProvider,
};
pub use manager::SessionManager;
pub use manifest::{RemoteSessionFile, SessionManifest};
pub use remote_provider::RemoteSessionProvider;
pub use remote_sync::{SessionRefreshOptions, SessionRefreshReport, refresh_session_from_remote};
pub use reqwest_provider::ReqwestRemoteSessionProvider;
pub use sync::{SessionSyncMode, SessionSyncReport, sync_remote_files_to_session};
