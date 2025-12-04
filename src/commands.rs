use anyhow::{Context, Result, bail};
use indicatif::HumanBytes;
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Semaphore;
use num_cpus;

use crate::downloader::Downloader;
use crate::state::DownloadState;
use crate::providers::{self, DownloadItem};
use crate::cli::VerifyMode;

pub async fn get_total_size(items: &[DownloadItem]) -> HashMap<String, u64> {
    let client = reqwest::Client::builder()
        .user_agent("rdl/0.1.0")
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let mut handles = vec![];

    for item in items {
        let client = client.clone();
        let url = item.url.clone();
        handles.push(tokio::spawn(async move {
            if let Ok(resp) = client.head(&url).send().await {
                (url, resp.content_length().unwrap_or(0))
            } else {
                (url, 0)
            }
        }));
    }

    let mut map = HashMap::new();
    for handle in handles {
        if let Ok((url, size)) = handle.await {
            if size > 0 {
                map.insert(url, size);
            }
        }
    }
    map
}

pub async fn run_downloads(input: PathBuf, output: PathBuf, concurrency: Option<usize>, rate_limit: Option<u32>, split: usize, daemon: bool, verify_mode: VerifyMode) -> Result<()> {
    if !output.exists() {
        fs::create_dir_all(&output).await.context("Failed to create output directory")?;
    }

    let file = fs::File::open(&input).await.context(format!("Failed to open input file: {:?}", input))?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    let mut items: Vec<DownloadItem> = vec![];
    while let Some(line) = lines.next_line().await? {
        let raw = line.trim();
        if raw.is_empty() {
            continue;
        }
        let mut parts = raw.splitn(2, '|');
        let url = parts.next().unwrap_or_default().trim().to_string();
        let hash = parts.next().map(|h| h.trim().to_string()).filter(|s| !s.is_empty());
        if !url.is_empty() {
            items.push(DownloadItem { url, hash });
        }
    }
    if matches!(verify_mode, VerifyMode::On) {
        // Require hash for every item
        let missing: Vec<String> = items
            .iter()
            .filter(|i| i.hash.is_none())
            .map(|i| i.url.clone())
            .collect();
        if !missing.is_empty() {
            bail!("校验模式为 on，但以下条目缺少 hash: {:?}", missing);
        }
    }

    let total_files = items.len();

    // Pre-calculate total size
    println!("Calculating total size...");
    let size_map = get_total_size(&items).await;
    let expected_hashes: HashMap<String, String> = if matches!(verify_mode, VerifyMode::Off) {
        HashMap::new()
    } else {
        items
            .iter()
            .filter_map(|i| i.hash.as_ref().map(|h| (i.url.clone(), h.clone())))
            .collect()
    };

    let downloader = Arc::new(Downloader::new(output.clone(), rate_limit, split, total_files, size_map, expected_hashes, verify_mode.clone()));
    let concurrency = concurrency.unwrap_or_else(num_cpus::get);
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut handles = vec![];

    for item in items {
        let downloader_clone = downloader.clone();
        let semaphore_clone = semaphore.clone();
        let download_item = item.clone();
        let url_for_log = download_item.url.clone();

        let handle = tokio::spawn(async move {
            let _permit = semaphore_clone.acquire().await.unwrap();
            if let Err(e) = downloader_clone.download_file(download_item).await {
                eprintln!("Failed to download {}: {}", url_for_log, e);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await?;
    }

    // Clean up PID file if we are the daemon
    #[cfg(unix)]
    if daemon {
        crate::daemon::cleanup_pid_file();
    }

    Ok(())
}
pub async fn run_single_download(
    url: String,
    output: PathBuf,
    concurrency: Option<usize>,
    rate_limit: Option<u32>,
    split: usize,
    verify_mode: VerifyMode,
) -> Result<()> {
    if !output.exists() {
        fs::create_dir_all(&output).await.context("Failed to create output directory")?;
    }

    let items = vec![DownloadItem { url: url.clone(), hash: None }];
    
    // Pre-calculate total size
    println!("Calculating size...");
    let size_map = get_total_size(&items).await;
    let expected_hashes = HashMap::new(); // Single URL download via CLI doesn't support hash verification yet

    let downloader = Arc::new(Downloader::new(
        output.clone(), 
        rate_limit, 
        split, 
        1, 
        size_map, 
        expected_hashes, 
        verify_mode
    ));
    
    // For single file, we don't need complex semaphore logic, but we keep the structure consistent
    // Concurrency here applies to splits if we were downloading multiple files, 
    // but for single file, the splits are handled inside download_file.
    // However, download_file itself spawns tasks.
    
    if let Err(e) = downloader.download_file(items[0].clone()).await {
        eprintln!("Failed to download {}: {}", url, e);
        return Err(e);
    }

    Ok(())
}

pub async fn list_downloads(output: PathBuf, input: PathBuf) -> Result<()> {
    if !output.exists() {
        println!("Output directory '{:?}' does not exist.", output);
        println!("Tip: If you used a custom output directory, please specify it with --output");
        return Ok(());
    }

    // Calculate summary stats
    let mut total_files_count = 0;
    if let Ok(file) = File::open(&input) {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(file);
        total_files_count = reader.lines().count();
    }

    let mut downloaded_files_count = 0;
    let mut active_files_count = 0;
    let mut total_downloaded_bytes: u64 = 0;
    let mut total_known_bytes: u64 = 0;

    // First pass: scan for stats
    if let Ok(mut entries) = fs::read_dir(&output).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let filename = path.file_name().unwrap().to_string_lossy();

            if filename.ends_with(".part.json") {
                if let Ok(content) = fs::read_to_string(&path).await {
                    if let Ok(state) = serde_json::from_str::<DownloadState>(&content) {
                        active_files_count += 1;
                        let downloaded: u64 = state.parts.iter().map(|p| p.current_byte - p.start_byte).sum();
                        total_downloaded_bytes += downloaded;
                        total_known_bytes += state.total_size;
                    }
                }
            } else if !filename.ends_with(".part") && filename != ".DS_Store" {
                 if let Ok(metadata) = entry.metadata().await {
                     if metadata.is_file() {
                         downloaded_files_count += 1;
                         total_downloaded_bytes += metadata.len();
                         total_known_bytes += metadata.len();
                     }
                 }
            }
        }
    }

    println!("Summary: Files: {}/{} | Active: {} | Downloaded: {} / {}",
        downloaded_files_count,
        total_files_count,
        active_files_count,
        HumanBytes(total_downloaded_bytes),
        HumanBytes(total_known_bytes)
    );
    println!();

    println!("{:<50} {:<15} {:<15} {:<15}", "Filename", "Status", "Progress", "Size");
    println!("{:-<50} {:-<15} {:-<15} {:-<15}", "", "", "", "");

    let mut found_any = false;

    if let Ok(mut entries) = fs::read_dir(&output).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "json" {
                    if let Some(stem) = path.file_stem() {
                        if let Some(stem_str) = stem.to_str() {
                            if stem_str.ends_with(".part") {
                                if let Ok(content) = fs::read_to_string(&path).await {
                                    if let Ok(state) = serde_json::from_str::<DownloadState>(&content) {
                                        let filename = stem_str.trim_end_matches(".part");
                                        let downloaded: u64 = state.parts.iter().map(|p| p.current_byte - p.start_byte).sum();
                                        let total = state.total_size;
                                        let progress = if total > 0 {
                                            (downloaded as f64 / total as f64) * 100.0
                                        } else {
                                            0.0
                                        };
                                        
                                        println!("{:<50} {:<15} {:<15} {:<15}",
                                            filename,
                                            "Downloading",
                                            format!("{:.2}%", progress),
                                            format!("{}", HumanBytes(total))
                                        );
                                        found_any = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    if let Ok(mut entries) = fs::read_dir(&output).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_file() {
                let filename = path.file_name().unwrap().to_string_lossy();
                if !filename.ends_with(".part") && !filename.ends_with(".part.json") && filename != ".DS_Store" {
                    if let Ok(metadata) = entry.metadata().await {
                        println!("{:<50} {:<15} {:<15} {:<15}",
                            filename,
                            "Completed",
                            "100.00%",
                            format!("{}", HumanBytes(metadata.len()))
                        );
                        found_any = true;
                    }
                }
            }
        }
    }

    if !found_any {
        println!("No active or completed downloads found in '{:?}'.", output);
    }

    Ok(())
}

pub async fn follow_log(output: PathBuf, input: PathBuf) -> Result<()> {
    loop {
        print!("\x1B[1;1H\x1B[0J");
        list_downloads(output.clone(), input.clone()).await?;
        println!("\n(Press Ctrl+C to exit view)");
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

pub async fn generate_download_list(
    model: &str,
    output_path: PathBuf,
    use_default_input_path: bool,
    revision: String,
    provider: String
) -> Result<PathBuf> {
    let items = providers::fetch_urls(&provider, model, &revision).await?;
    if items.is_empty() {
        bail!("文件列表为空");
    }

    // If user didn't override --input (still using default download.txt),
    // place the generated list under providers/<provider>/<model>/download.txt.
    let final_output = if use_default_input_path {
        PathBuf::from("downloads")
            .join(provider.to_lowercase())
            .join(model)
            .join("download.txt")
    } else {
        output_path
    };

    if let Some(parent) = final_output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .await
                .context("创建下载列表目录失败")?;
        }
    }

    let mut content_lines = Vec::with_capacity(items.len());
    for item in &items {
        let line = match &item.hash {
            Some(hash) => format!("{}|{}", item.url, hash),
            None => item.url.clone(),
        };
        content_lines.push(line);
    }
    let content = content_lines.join("\n") + "\n";
    fs::write(&final_output, content)
        .await
        .context("写入下载列表失败")?;
    println!("已写入 {} 条链接到 {:?}", items.len(), final_output);
    Ok(final_output)
}
