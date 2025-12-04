pub mod modelscope;

use anyhow::{Result, bail};

#[derive(Clone, Debug)]
pub struct DownloadItem {
    pub url: String,
    pub hash: Option<String>,
}

/// 根据 provider 名称获取下载链接列表
/// 当前仅支持 modelscope，后续可在此扩展 huggingface 等。
pub async fn fetch_urls(provider: &str, model: &str, revision: &str) -> Result<Vec<DownloadItem>> {
    match provider.to_lowercase().as_str() {
        "modelscope" => modelscope::fetch_modelscope_urls(model, revision).await,
        _ => bail!("暂不支持的 provider: {}", provider),
    }
}
