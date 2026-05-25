# M41 BI Login Interface

## Endpoint
- URL: /api/auth/signin
- Method: POST
- Body: {cipherPassport: Base64(JSON({user, password, remember, userDirectory}))}

## Cookie
- `ReqwestRemoteSessionProvider::new()` explicitly calls `Client::builder().cookie_store(true)`.
- reqwest auto-manages Set-Cookie from login response into an in-memory jar.
- Subsequent GET/POST requests automatically include Cookie header.
- No persistent cookie jar on disk (CLI re-login on each `--session-refresh`).
- Secrets must not leak to stdout/stderr/manifest/graphdb.

## Implementation
- `src/session/reqwest_provider.rs:login()` — 构造 cipherPassport 并 POST /api/auth/signin。
- `src/session/reqwest_provider.rs::new()` — 启用 `.cookie_store(true)`。

## Uncertainties
1. Some BI envs may use security.action encryptUserPassword
2. userDirectory default is sys but external exists
3. tenant/appName params not handled
4. CSRF token not observed
5. No automatic re-login on session expiry
