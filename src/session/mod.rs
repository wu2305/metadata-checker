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

pub use auth::{AuthContext, AuthProvider, SecretStore, StaticAuthProvider};
pub use manager::SessionManager;
pub use manifest::{RemoteSessionFile, SessionManifest};
pub use remote_provider::RemoteSessionProvider;
