/**!
 * M41.2：统一 reqwest-based remote metadata provider
 *
 * 作为 M41 的统一 HTTP transport，同时支持 native（cli-local）和 browser-wasm 两种 target。
 * URL 构造、响应解析、错误码映射仍在 remote_metadata.rs 中，此处只做 transport 层。
 */
use crate::remote_metadata::{
    AsyncRemoteMetadataProvider, RemoteFileContent, RemoteFileInfo, RemoteFileRef,
    RemoteMetadataError, RemoteMetadataErrorCode, extract_remote_content_payload,
};

/// reqwest-based 远程元数据 provider
pub struct ReqwestRemoteMetadataProvider {
    client: reqwest::Client,
    base_url: String,
}

impl ReqwestRemoteMetadataProvider {
    /// 创建默认 provider。创建失败时返回错误，不再 panic/expect。
    pub fn new(base_url: impl Into<String>) -> Result<Self, RemoteMetadataError> {
        let client = reqwest::Client::builder().build().map_err(|err| {
            RemoteMetadataError::new(
                RemoteMetadataErrorCode::ClientFetchProxyFailed,
                format!("failed to build reqwest client: {}", err),
            )
        })?;
        Ok(Self {
            client,
            base_url: base_url.into(),
        })
    }

    /// 创建 provider 兼容别名（兼容旧命名风格）。
    pub fn try_new(base_url: impl Into<String>) -> Result<Self, RemoteMetadataError> {
        Self::new(base_url)
    }

    /// 使用外部 reqwest::Client（便于注入 mock 或自定义配置）
    pub fn with_client(base_url: impl Into<String>, client: reqwest::Client) -> Self {
        Self {
            client,
            base_url: base_url.into(),
        }
    }

    /// 发送 GET 请求并获取文本响应
    async fn fetch_text(&self, url: &str) -> Result<String, RemoteMetadataError> {
        #[cfg(target_arch = "wasm32")]
        let request = {
            let mut r = self.client.get(url);
            r = r.fetch_credentials_include();
            r
        };
        #[cfg(not(target_arch = "wasm32"))]
        let request = self.client.get(url);

        let response = request.send().await.map_err(|err| {
            let message = err.to_string();
            let lower = message.to_lowercase();
            let code = if lower.contains("cors")
                || lower.contains("failed to fetch")
                || lower.contains("networkerror")
                || lower.contains("load failed")
            {
                RemoteMetadataErrorCode::RemoteFetchCorsBlocked
            } else {
                RemoteMetadataErrorCode::RemoteFetchFailed
            };
            RemoteMetadataError::new(code, message)
        })?;

        let status = response.status();
        if !status.is_success() {
            return Err(RemoteMetadataError::new(
                crate::remote_metadata::map_http_status_code(status.as_u16()),
                format!("remote metadata fetch failed with HTTP status {}", status),
            ));
        }

        let text = response.text().await.map_err(|err| {
            RemoteMetadataError::new(
                RemoteMetadataErrorCode::RemoteResponseInvalid,
                format!("failed to read response body: {}", err),
            )
        })?;

        if text.is_empty() {
            return Err(RemoteMetadataError::new(
                RemoteMetadataErrorCode::RemoteResponseInvalid,
                "remote metadata response is empty",
            ));
        }

        Ok(text)
    }

    /// 获取远程文件内容
    async fn fetch_content(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<RemoteFileContent, RemoteMetadataError> {
        let url = crate::remote_metadata::build_remote_content_url(&self.base_url, file_ref)
            .map_err(|e| {
                RemoteMetadataError::new(
                    RemoteMetadataErrorCode::RemoteFetchFailed,
                    format!("failed to build remote content url: {}", e.message),
                )
            })?;
        let response_text = self.fetch_text(&url).await?;
        extract_remote_content_payload(&response_text, file_ref)
    }
}

#[cfg(any(feature = "cli-local", feature = "browser-wasm"))]
impl AsyncRemoteMetadataProvider for ReqwestRemoteMetadataProvider {
    async fn get_file_info(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<RemoteFileInfo, RemoteMetadataError> {
        let content = self.fetch_content(file_ref).await?;
        Ok(RemoteFileInfo {
            source_path: content.source_path,
            file_id: content.file_id,
            revision: content.revision,
            content_type: content.content_type,
            updated_at: None,
        })
    }

    async fn get_file_content(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<RemoteFileContent, RemoteMetadataError> {
        self.fetch_content(file_ref).await
    }

    async fn get_related_files(
        &self,
        _file_ref: &RemoteFileRef,
    ) -> Result<Vec<RemoteFileRef>, RemoteMetadataError> {
        Ok(Vec::new())
    }
}

// ============================================================================
// WASM JS 绑定（M41.3）
// ============================================================================

#[cfg(feature = "browser-wasm")]
pub mod wasm_bindings {
    use wasm_bindgen::prelude::*;

    use crate::browser::{AnalysisStatus, BrowserAnalysisEnvelope, RuntimeOptions};
    use crate::remote_metadata::AsyncRemoteMetadataProvider;
    use crate::remote_metadata::RemoteFileRef;
    use crate::remote_metadata_provider::ReqwestRemoteMetadataProvider;

    fn make_error_envelope(message: impl Into<String>, code: &str) -> BrowserAnalysisEnvelope {
        BrowserAnalysisEnvelope {
            status: AnalysisStatus::Error,
            target: None,
            items: vec![],
            diagnostics: vec![crate::browser::AnalysisDiagnostic {
                severity: "error".to_string(),
                code: code.to_string(),
                message: message.into(),
            }],
        }
    }

    fn envelope_to_string(envelope: &BrowserAnalysisEnvelope) -> String {
        serde_json::to_string(envelope).unwrap_or_else(|_| "{\"status\":\"error\"}".to_string())
    }

    /// WASM API：获取远程文件信息
    #[wasm_bindgen(js_name = fetchRemoteFileInfo)]
    pub async fn js_fetch_remote_file_info(base_url: String, file_ref_json: String) -> String {
        let file_ref: RemoteFileRef = match serde_json::from_str(&file_ref_json) {
            Ok(v) => v,
            Err(e) => {
                return envelope_to_string(&make_error_envelope(
                    format!("Failed to parse fileRef JSON: {}", e),
                    "INVALID_FILE_REF",
                ));
            }
        };

        let provider = match ReqwestRemoteMetadataProvider::new(base_url) {
            Ok(p) => p,
            Err(err) => {
                return envelope_to_string(&make_error_envelope(
                    err.message,
                    "REMOTE_FETCH_INITIALIZE_FAILED",
                ));
            }
        };

        match AsyncRemoteMetadataProvider::get_file_info(&provider, &file_ref).await {
            Ok(info) => {
                let envelope = BrowserAnalysisEnvelope {
                    status: AnalysisStatus::Ready,
                    target: Some(info.source_path.clone()),
                    items: vec![crate::browser::AnalysisItem {
                        kind: "remote_file_info".to_string(),
                        label: "Remote File Info".to_string(),
                        detail: serde_json::to_value(info).unwrap_or_default(),
                    }],
                    diagnostics: vec![],
                };
                envelope_to_string(&envelope)
            }
            Err(err) => {
                envelope_to_string(&make_error_envelope(err.message, "REMOTE_FETCH_FAILED"))
            }
        }
    }

    /// WASM API：获取远程文件内容
    #[wasm_bindgen(js_name = fetchRemoteFileContent)]
    pub async fn js_fetch_remote_file_content(base_url: String, file_ref_json: String) -> String {
        let file_ref: RemoteFileRef = match serde_json::from_str(&file_ref_json) {
            Ok(v) => v,
            Err(e) => {
                return envelope_to_string(&make_error_envelope(
                    format!("Failed to parse fileRef JSON: {}", e),
                    "INVALID_FILE_REF",
                ));
            }
        };

        let provider = match ReqwestRemoteMetadataProvider::new(base_url) {
            Ok(p) => p,
            Err(err) => {
                return envelope_to_string(&make_error_envelope(
                    err.message,
                    "REMOTE_FETCH_INITIALIZE_FAILED",
                ));
            }
        };

        match AsyncRemoteMetadataProvider::get_file_content(&provider, &file_ref).await {
            Ok(content) => {
                let envelope = BrowserAnalysisEnvelope {
                    status: AnalysisStatus::Ready,
                    target: Some(content.source_path.clone()),
                    items: vec![crate::browser::AnalysisItem {
                        kind: "remote_file_content".to_string(),
                        label: "Remote File Content".to_string(),
                        detail: serde_json::to_value(content).unwrap_or_default(),
                    }],
                    diagnostics: vec![],
                };
                envelope_to_string(&envelope)
            }
            Err(err) => {
                envelope_to_string(&make_error_envelope(err.message, "REMOTE_FETCH_FAILED"))
            }
        }
    }

    /// WASM API：加载远程 SuperPage 文档（fetch + load 一步完成）
    #[wasm_bindgen(js_name = loadRemoteSuperpageDocument)]
    pub async fn js_load_remote_superpage_document(
        base_url: String,
        file_ref_json: String,
        options_json: String,
    ) -> String {
        let file_ref: RemoteFileRef = match serde_json::from_str(&file_ref_json) {
            Ok(v) => v,
            Err(e) => {
                return envelope_to_string(&make_error_envelope(
                    format!("Failed to parse fileRef JSON: {}", e),
                    "INVALID_FILE_REF",
                ));
            }
        };

        let _options: RuntimeOptions = match serde_json::from_str(&options_json) {
            Ok(v) => v,
            Err(e) => {
                return envelope_to_string(&make_error_envelope(
                    format!("Failed to parse options JSON: {}", e),
                    "INVALID_OPTIONS",
                ));
            }
        };

        let provider = match ReqwestRemoteMetadataProvider::new(base_url) {
            Ok(p) => p,
            Err(err) => {
                return envelope_to_string(&make_error_envelope(
                    err.message,
                    "REMOTE_FETCH_INITIALIZE_FAILED",
                ));
            }
        };

        match AsyncRemoteMetadataProvider::get_file_content(&provider, &file_ref).await {
            Ok(content) => crate::browser_wasm_bindgen::js_load_superpage_document(
                &content.source_path,
                &content.raw_text,
            ),
            Err(err) => {
                envelope_to_string(&make_error_envelope(err.message, "REMOTE_FETCH_FAILED"))
            }
        }
    }
}
