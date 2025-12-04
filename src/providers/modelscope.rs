use anyhow::{Context, Result, bail, anyhow};
use serde::Deserialize;

use super::DownloadItem;

#[derive(Deserialize)]
struct ModelScopeResponse {
    #[serde(rename = "Code")]
    code: Option<i32>,
    #[serde(rename = "Data")]
    data: Option<ModelScopeData>,
    #[serde(rename = "Message")]
    message: Option<String>,
    #[serde(rename = "Success")]
    success: Option<bool>,
}

#[derive(Deserialize)]
struct ModelScopeData {
    #[serde(rename = "Files")]
    files: Vec<ModelScopeFile>,
}

#[derive(Deserialize)]
struct ModelScopeFile {
    #[serde(rename = "Path")]
    path: String,
    #[serde(rename = "Sha256")]
    sha256: String,
}

pub async fn fetch_modelscope_urls(model: &str, revision: &str) -> Result<Vec<DownloadItem>> {
    let api_url = format!("https://modelscope.cn/api/v1/models/{}/repo/files", model);
    let client = reqwest::Client::builder()
        .user_agent("RustDownloadTool/0.1.0")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let resp = client.get(&api_url).send().await.context("请求 ModelScope 文件列表失败")?;
    if !resp.status().is_success() {
        bail!("请求失败，状态码：{}", resp.status());
    }

    let body = resp.text().await.context("读取 ModelScope 响应失败")?;
    let parsed: ModelScopeResponse = serde_json::from_str(&body).context("解析 ModelScope 响应 JSON 失败")?;

    let data = parsed.data.ok_or_else(|| anyhow!("响应缺少 Data 字段"))?;
    if data.files.is_empty() {
        bail!("文件列表为空");
    }

    let mut items = Vec::with_capacity(data.files.len());
    for file in data.files {
        let url = format!(
            "https://modelscope.cn/models/{}/resolve/{}/{}",
            model,
            revision,
            file.path
        );
        items.push(DownloadItem {
            url,
            hash: Some(file.sha256),
        });
    }

    Ok(items)
}
