//! 远程 session 认证边界
//!
//! 这里只定义凭证来源语义，不实现具体登录流程，避免 token/cookie 进入 manifest。

use anyhow::Result;

/// 远程认证/会话错误码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthSessionErrorCode {
    /// 页面态无法获取一次性 access token。
    AccessTokenUnavailable,
    /// 使用 token 或账号密码建立远程 session 失败。
    SessionBootstrapFailed,
    /// whoami 返回匿名用户，说明 session 未建立为业务用户。
    SessionBootstrapAnonymous,
    /// bootstrap 请求成功但没有建立可复用的 session cookie。
    SessionCookieNotEstablished,
}

impl AuthSessionErrorCode {
    /// 返回面向机器的稳定错误码。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AccessTokenUnavailable => "ACCESS_TOKEN_UNAVAILABLE",
            Self::SessionBootstrapFailed => "SESSION_BOOTSTRAP_FAILED",
            Self::SessionBootstrapAnonymous => "SESSION_BOOTSTRAP_ANONYMOUS",
            Self::SessionCookieNotEstablished => "SESSION_COOKIE_NOT_ESTABLISHED",
        }
    }
}

/// 已认证远程 session 的安全摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedSession {
    /// 登录用户 ID。
    pub user_id: String,
    /// 登录用户显示名。
    pub user_name: Option<String>,
    /// 后续远程请求使用的认证上下文。
    pub auth_context: AuthContext,
}

/// 远程请求认证上下文
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthContext {
    /// 浏览器登录态，由 browser-wasm / Service Worker 环境借用。
    BrowserSession,
    /// Native cookie jar，由 CLI 登录流程维护。
    CookieJar { jar_ref: String },
    /// 运行时内存 session，由当前 provider/client 持有，不落盘。
    RuntimeSession { session_ref: String },
    /// 显式用户名密码或 token 的外部引用，不直接保存 secret 值。
    ExplicitCredentials { secret_ref: String },
    /// 不带认证信息。
    None,
}

/// Secret 存储抽象
pub trait SecretStore {
    /// 读取 secret。实现方负责脱敏和生命周期控制。
    fn get_secret(&self, secret_ref: &str) -> Result<Option<String>>;
}

/// 认证上下文提供者
pub trait AuthProvider {
    /// 返回当前请求应使用的认证上下文。
    fn auth_context(&self) -> Result<AuthContext>;
}

/// 一次性 access token 提供者。
pub trait AccessTokenProvider {
    /// 获取一次性 access token。实现方不得记录 token 明文。
    fn access_token(&self) -> Result<String>;
}

/// 远程 session bootstrap 抽象。
pub trait SessionBootstrapper {
    /// 通过一次性 access token 建立远程 session。
    fn bootstrap_with_access_token(&self, access_token: &str) -> Result<AuthenticatedSession>;
}

/// 固定认证上下文提供者，适合测试和 CLI 参数已解析后的场景。
#[derive(Debug, Clone)]
pub struct StaticAuthProvider {
    context: AuthContext,
}

impl StaticAuthProvider {
    /// 创建固定认证上下文提供者。
    pub fn new(context: AuthContext) -> Self {
        Self { context }
    }
}

impl AuthProvider for StaticAuthProvider {
    fn auth_context(&self) -> Result<AuthContext> {
        Ok(self.context.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EmptySecretStore;

    impl SecretStore for EmptySecretStore {
        fn get_secret(&self, _secret_ref: &str) -> Result<Option<String>> {
            Ok(None)
        }
    }

    #[test]
    fn auth_context_does_not_store_plain_secret_in_debug() {
        let auth = AuthContext::ExplicitCredentials {
            secret_ref: "env:MC_REMOTE_PASSWORD".to_string(),
        };
        let debug = format!("{auth:?}");
        assert!(debug.contains("secret_ref"));
        assert!(!debug.contains("123456"));
    }

    #[test]
    fn auth_session_error_codes_are_stable() {
        assert_eq!(
            AuthSessionErrorCode::AccessTokenUnavailable.as_str(),
            "ACCESS_TOKEN_UNAVAILABLE"
        );
        assert_eq!(
            AuthSessionErrorCode::SessionBootstrapFailed.as_str(),
            "SESSION_BOOTSTRAP_FAILED"
        );
        assert_eq!(
            AuthSessionErrorCode::SessionBootstrapAnonymous.as_str(),
            "SESSION_BOOTSTRAP_ANONYMOUS"
        );
        assert_eq!(
            AuthSessionErrorCode::SessionCookieNotEstablished.as_str(),
            "SESSION_COOKIE_NOT_ESTABLISHED"
        );
    }

    #[test]
    fn empty_secret_store_returns_none() {
        let store = EmptySecretStore;
        assert_eq!(store.get_secret("missing").unwrap(), None);
    }

    #[test]
    fn static_auth_provider_returns_context_without_secret_value() {
        let provider = StaticAuthProvider::new(AuthContext::CookieJar {
            jar_ref: "session-cookie-jar".to_string(),
        });
        let context = provider.auth_context().unwrap();

        assert_eq!(
            context,
            AuthContext::CookieJar {
                jar_ref: "session-cookie-jar".to_string()
            }
        );
    }

    #[test]
    fn authenticated_session_debug_does_not_contain_credentials() {
        let session = AuthenticatedSession {
            user_id: "alice".to_string(),
            user_name: Some("Alice".to_string()),
            auth_context: AuthContext::RuntimeSession {
                session_ref: "memory-session".to_string(),
            },
        };
        let debug = format!("{session:?}");

        assert!(debug.contains("alice"));
        assert!(!debug.contains("cookie"));
        assert!(!debug.contains("token"));
        assert!(!debug.contains("password"));
    }
}
