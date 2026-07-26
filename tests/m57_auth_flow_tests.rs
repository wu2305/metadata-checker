#![cfg(feature = "cli-local")]
//! M57-3：运行期鉴权失败分类的 TDD。

use anyhow::anyhow;
use metadata_checker::session::reqwest_provider::is_session_auth_error;

/// 401/403 即使被 refresh context 包裹，也必须映射为鉴权错误。
#[test]
fn m57_auth_classifier_matches_wrapped_401_and_403() {
    let unauthorized = anyhow!("remote session returned 401 Unauthorized: invalid credentials")
        .context("apply changeset to session mirror");
    let forbidden =
        anyhow!("remote session returned 403 Forbidden").context("poll meta files changes");

    assert_eq!(is_session_auth_error(&unauthorized), true);
    assert_eq!(is_session_auth_error(&forbidden), true);
}

/// 数据错误不能因为包含 refresh context 就被误报为鉴权错误。
#[test]
fn m57_auth_classifier_rejects_data_error() {
    let error =
        anyhow!("INVALID_ACTIVE_CHANGE_EVENT: missing revision").context("poll meta files changes");

    assert_eq!(is_session_auth_error(&error), false);
}
