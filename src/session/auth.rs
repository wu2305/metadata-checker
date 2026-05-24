//! 远程 session 认证边界
//!
//! 这里只定义凭证来源语义，不实现具体登录流程，避免 token/cookie 进入 manifest。

use anyhow::Result;

/// 远程请求认证上下文
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthContext {
    /// 浏览器登录态，由 browser-wasm / Service Worker 环境借用。
    BrowserSession,
    /// Native cookie jar，由 CLI 登录流程维护。
    CookieJar { jar_ref: String },
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
}
