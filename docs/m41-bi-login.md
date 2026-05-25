# M41 BI Login Interface

## Endpoint
- URL: /api/auth/signin
- Method: POST
- Body: {cipherPassport: Base64(JSON({user, password, remember, userDirectory}))}

## Cookie
- reqwest cookie_store auto-manages Set-Cookie
- No persistent cookie jar on disk
- Secrets must not leak to stdout/stderr/manifest

## Uncertainties
1. Some BI envs may use security.action encryptUserPassword
2. userDirectory default is sys but external exists
3. tenant/appName params not handled
4. CSRF token not observed
5. No automatic re-login on session expiry
