use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use governor::{Quota, RateLimiter};
use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use chrono::{DateTime, Local};
use indicatif::{HumanBytes, MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use reqwest::{Client, header};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt, AsyncSeekExt, SeekFrom};
use tokio::sync::Mutex;
use std::time::{Instant, Duration};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::collections::HashMap;

use crate::utils::{get_filename_from_url, sanitize_filename};
use crate::cli::VerifyMode;
use crate::state::{DownloadState, PartState};

pub struct Downloader {
    client: Client,
    output_dir: PathBuf,
    multi_progress: MultiProgress,
    rate_limiter: Option<Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>>,
    split_count: usize,
    total_files: usize,
    downloaded_files: Arc<AtomicUsize>,
    total_downloaded_bytes: Arc<AtomicU64>,
    total_known_bytes: Arc<AtomicU64>,
    header_pb: ProgressBar,
    size_map: HashMap<String, u64>,
    expected_hashes: HashMap<String, String>,
    verify_mode: VerifyMode,
}

impl Downloader {
    pub fn new(
        output_dir: PathBuf,
        rate_limit_bytes_per_sec: Option<u32>,
        split_count: usize,
        total_files: usize,
        size_map: HashMap<String, u64>,
        expected_hashes: HashMap<String, String>,
        verify_mode: VerifyMode,
    ) -> Self {
        let client = Client::builder()
            .user_agent("rdl/0.1.0")
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new());

        let multi_progress = MultiProgress::new();
        // Force draw target to stderr with a reasonable refresh rate (e.g., 5Hz)
        // This ensures progress bars are written even when redirected to a file (daemon mode)
        multi_progress.set_draw_target(ProgressDrawTarget::stderr_with_hz(5));
        
        let header_pb = multi_progress.add(ProgressBar::new(0));
        header_pb.set_style(ProgressStyle::default_bar().template("{msg}").unwrap());
        header_pb.set_message(format!("Summary: Files: 0/{} | Downloaded: 0 B", total_files));

        let rate_limiter = rate_limit_bytes_per_sec.map(|limit| {
            let quota = Quota::per_second(NonZeroU32::new(limit).unwrap());
            Arc::new(RateLimiter::direct(quota))
        });

        let downloaded_files = Arc::new(AtomicUsize::new(0));
        let total_downloaded_bytes = Arc::new(AtomicU64::new(0));
        
        // Initialize total_known_bytes with the sum of pre-calculated sizes
        let initial_total_bytes: u64 = size_map.values().sum();
        let total_known_bytes = Arc::new(AtomicU64::new(initial_total_bytes));

        // Spawn a monitor task to update the header periodically
        let df = downloaded_files.clone();
        let tdb = total_downloaded_bytes.clone();
        let tkb = total_known_bytes.clone();
        let hpb = header_pb.clone();
        tokio::spawn(async move {
            loop {
                let downloaded = df.load(Ordering::Relaxed);
                let bytes = tdb.load(Ordering::Relaxed);
                let known = tkb.load(Ordering::Relaxed);
                hpb.set_message(format!(
                    "Summary: Files: {}/{} | Downloaded: {} / {}", 
                    downloaded, 
                    total_files, 
                    HumanBytes(bytes),
                    HumanBytes(known)
                ));
                hpb.tick(); // Force refresh
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });

        Self {
            client,
            output_dir,
            multi_progress,
            rate_limiter,
            split_count,
            total_files,
            downloaded_files,
            total_downloaded_bytes,
            total_known_bytes,
            header_pb,
            size_map,
            expected_hashes,
            verify_mode,
        }
    }

    pub async fn download_file(&self, item: crate::providers::DownloadItem) -> Result<()> {
        let url = item.url.clone();
        let filename = get_filename_from_url(&url)?;
        let sanitized_filename = sanitize_filename(&filename);
        let filepath = self.output_dir.join(&sanitized_filename);

        if filepath.exists() {
            let metadata = fs::metadata(&filepath).await?;
            let size = metadata.len();
            let created: DateTime<Local> = metadata.created()?.into();
            
            let pb = self.multi_progress.add(ProgressBar::new(0));
            pb.set_style(ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {msg}")
                .unwrap());
            
            // Align with: {bytes:>12}/{total_bytes:<12} {bytes_per_sec:>12} {eta:>4}
            // Total width approx: 25 + 1 + 12 + 1 + 4 = 43 chars
            let size_str = format!("{}", HumanBytes(size));
            let date_str = created.format("%Y-%m-%d %H:%M").to_string();
            
            pb.finish_with_message(format!(
                "{:>25} {:>17} Skipped {}",
                size_str,
                date_str,
                sanitized_filename
            ));
            self.downloaded_files.fetch_add(1, Ordering::Relaxed);
            self.total_downloaded_bytes.fetch_add(size, Ordering::Relaxed);
            
            // If this file was NOT in the size_map (e.g. HEAD failed), we need to add it to known bytes now
            if !self.size_map.contains_key(&url) {
                 self.total_known_bytes.fetch_add(size, Ordering::Relaxed);
            }
            
            return Ok(());
        }

        // Determine partial file path and state file path
        let mut part_filepath = filepath.clone();
        if let Some(extension) = filepath.extension() {
            let mut ext = extension.to_os_string();
            ext.push(".part");
            part_filepath.set_extension(ext);
        } else {
            part_filepath.set_extension("part");
        }
        let state_filepath = part_filepath.with_extension("part.json");

        // Initialize or load state
        let mut state = if state_filepath.exists() {
            let content = fs::read_to_string(&state_filepath).await?;
            match serde_json::from_str(&content) {
                Ok(s) => s,
                Err(_) => self.init_state(&url).await.unwrap_or(DownloadState {
                    url: url.clone(),
                    total_size: 0,
                    parts: vec![],
                }),
            }
        } else {
            self.init_state(&url).await?
        };

        // Update known bytes if not already counted
        if !self.size_map.contains_key(&url) && state.total_size > 0 {
             self.total_known_bytes.fetch_add(state.total_size, Ordering::Relaxed);
        }

        // If total_size is 0 (unknown), fallback to single connection download
        if state.total_size == 0 {
             return self.download_single_connection(url, filepath, part_filepath).await;
        }

        // Create/Open the partial file
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .open(&part_filepath)
            .await
            .context("Failed to open partial file")?;
        
        // Pre-allocate file size if new
        if file.metadata().await?.len() < state.total_size {
            file.set_len(state.total_size).await?;
        }
        
        let file = Arc::new(Mutex::new(file));
        let state_mutex = Arc::new(Mutex::new(state.clone()));

        let pb = self.multi_progress.add(ProgressBar::new(state.total_size));
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes:>12}/{total_bytes:<12} {bytes_per_sec:>12} {eta:>4} {msg}")
            .unwrap()
            .progress_chars("=>-"));
        pb.set_message(format!("Downloading {}", sanitized_filename));
        
        let initial_progress: u64 = state.parts.iter().map(|p| p.current_byte - p.start_byte).sum();
        pb.set_position(initial_progress);

        let mut handles = vec![];

        for part in state.parts.iter_mut() {
            if part.completed {
                continue;
            }

            let client = self.client.clone();
            let url = url.clone();
            let file = file.clone();
            let state_mutex = state_mutex.clone();
            let pb = pb.clone();
            let rate_limiter = self.rate_limiter.clone();
            let part_index = part.index;
            let start = part.current_byte;
            let end = part.end_byte;
            let state_filepath = state_filepath.clone();
            let total_downloaded_bytes = self.total_downloaded_bytes.clone();
            let downloaded_files = self.downloaded_files.clone();
            let header_pb = self.header_pb.clone();
            let total_files = self.total_files;

            let handle = tokio::spawn(async move {
                let range_header = format!("bytes={}-{}", start, end);
                let mut request = client.get(&url).header(header::RANGE, range_header);
                
                let response = request.send().await.context("Failed to send request")?;
                let mut stream = response.bytes_stream();
                let mut current_pos = start;

                while let Some(item) = stream.next().await {
                    let chunk = item.context("Error while downloading chunk")?;
                    let len = chunk.len();

                    if len > 0 {
                        if let Some(limiter) = &rate_limiter {
                            if let Some(nonzero) = NonZeroU32::new(len as u32) {
                                limiter.until_n_ready(nonzero).await.unwrap();
                            }
                        }

                        {
                            let mut f = file.lock().await;
                            f.seek(SeekFrom::Start(current_pos)).await?;
                            f.write_all(&chunk).await?;
                        }

                        current_pos += len as u64;
                        pb.inc(len as u64);
                        
                        // Update global stats
                        total_downloaded_bytes.fetch_add(len as u64, Ordering::Relaxed);
                        {
                            let mut s = state_mutex.lock().await;
                            if let Some(p) = s.parts.get_mut(part_index) {
                                p.current_byte = current_pos;
                                if p.current_byte > p.end_byte {
                                     p.completed = true;
                                } else if p.current_byte == p.end_byte + 1 {
                                     p.completed = true;
                                }
                            }
                            
                            // Save state to file (throttled)
                            let content = serde_json::to_string(&*s)?;
                            fs::write(&state_filepath, content).await?;
                        }
                    }
                }
                
                // Mark part as completed
                {
                    let mut s = state_mutex.lock().await;
                    if let Some(p) = s.parts.get_mut(part_index) {
                        p.completed = true;
                        p.current_byte = p.end_byte + 1; // Ensure it marks as fully done
                    }
                    let content = serde_json::to_string(&*s)?;
                    fs::write(&state_filepath, content).await?;
                }

                Ok::<(), anyhow::Error>(())
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await??;
        }

        // Cleanup
        if state_filepath.exists() {
            fs::remove_file(state_filepath).await?;
        }

        // Hash/verify policy
        let expected = self.expected_hashes.get(&url).cloned();
        if matches!(self.verify_mode, VerifyMode::Off) {
            tokio::fs::rename(&part_filepath, &filepath).await.context("Failed to rename partial file")?;
            pb.finish_with_message(format!("Completed   {}", sanitized_filename));
        } else if let Some(_) = expected {
            pb.set_message(format!("Verifying {}", sanitized_filename));
            let hash = crate::hashing::calculate_hash(&part_filepath).await?;
            self.verify_hash(&url, &hash, &part_filepath)?;
            tokio::fs::rename(&part_filepath, &filepath).await.context("Failed to rename partial file")?;
            pb.finish_with_message(format!("Verified    {} (SHA256: {})", sanitized_filename, hash));
        } else if matches!(self.verify_mode, VerifyMode::On) {
            // Should be prevented earlier; keep a guard.
            return Err(anyhow!("缺少哈希：{}", url));
        } else {
            tokio::fs::rename(&part_filepath, &filepath).await.context("Failed to rename partial file")?;
            pb.finish_with_message(format!("Completed   {}", sanitized_filename));
        }
        
        // Update completed files count
        self.downloaded_files.fetch_add(1, Ordering::Relaxed);
        
        Ok(())
    }

    async fn init_state(&self, url: &str) -> Result<DownloadState> {
        let response = self.client.head(url).send().await?;
        let total_size = response.content_length().unwrap_or(0);

        if total_size == 0 {
            return Ok(DownloadState {
                url: url.to_string(),
                total_size: 0,
                parts: vec![],
            });
        }

        let part_size = total_size / self.split_count as u64;
        let mut parts = vec![];

        for i in 0..self.split_count {
            let start_byte = i as u64 * part_size;
            let end_byte = if i == self.split_count - 1 {
                total_size - 1
            } else {
                (i as u64 + 1) * part_size - 1
            };

            parts.push(PartState {
                index: i,
                start_byte,
                end_byte,
                current_byte: start_byte,
                completed: false,
            });
        }

        Ok(DownloadState {
            url: url.to_string(),
            total_size,
            parts,
        })
    }

    async fn download_single_connection(&self, url: String, filepath: PathBuf, part_filepath: PathBuf) -> Result<()> {
         // Fallback to original single connection logic for files without content-length
         // ... (Simplified version of previous logic)
         
        let mut downloaded_len = 0;
        if part_filepath.exists() {
            downloaded_len = fs::metadata(&part_filepath).await?.len();
        }

        let mut request = self.client.get(&url);
        if downloaded_len > 0 {
            request = request.header(header::RANGE, format!("bytes={}-", downloaded_len));
        }

        let response = request.send().await.context("Failed to send request")?;
        let total_size = response.content_length().unwrap_or(0) + downloaded_len;
        
        // Update known bytes if we discovered size here AND it wasn't in the map
        if total_size > 0 && !self.size_map.contains_key(&url) {
             self.total_known_bytes.fetch_add(total_size, Ordering::Relaxed);
        }

        let pb = self.multi_progress.add(ProgressBar::new(total_size));
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes:>12}/{total_bytes:<12} {bytes_per_sec:>12} {eta:>4} {msg}")
            .unwrap()
            .progress_chars("=>-"));
        pb.set_message(format!("Downloading {}", filepath.file_name().unwrap().to_string_lossy()));
        pb.set_position(downloaded_len);

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(&part_filepath)
            .await
            .context("Failed to open partial file")?;

        let mut stream = response.bytes_stream();

        while let Some(item) = stream.next().await {
            let chunk = item.context("Error while downloading chunk")?;
            let len = chunk.len();

            if len > 0 {
                if let Some(limiter) = &self.rate_limiter {
                    if let Some(nonzero) = NonZeroU32::new(len as u32) {
                        limiter.until_n_ready(nonzero).await.unwrap();
                    }
                }

                file.write_all(&chunk).await.context("Error while writing to file")?;
                pb.inc(len as u64);
                
                // Update global stats for single connection download
                self.total_downloaded_bytes.fetch_add(len as u64, Ordering::Relaxed);
            }
        }

        file.flush().await.context("Failed to flush file")?;
        drop(file);

        let expected = self.expected_hashes.get(&url).cloned();
        if matches!(self.verify_mode, VerifyMode::Off) {
            tokio::fs::rename(&part_filepath, &filepath).await.context("Failed to rename partial file")?;
            pb.finish_with_message(format!("Completed   {}", filepath.file_name().unwrap().to_string_lossy()));
        } else if let Some(_) = expected {
            pb.set_message(format!("Verifying {}", filepath.file_name().unwrap().to_string_lossy()));
            let hash = crate::hashing::calculate_hash(&part_filepath).await?;
            self.verify_hash(&url, &hash, &part_filepath)?;
            tokio::fs::rename(&part_filepath, &filepath).await.context("Failed to rename partial file")?;
            pb.finish_with_message(format!("Verified    {} (SHA256: {})", filepath.file_name().unwrap().to_string_lossy(), hash));
        } else if matches!(self.verify_mode, VerifyMode::On) {
            return Err(anyhow!("缺少哈希：{}", url));
        } else {
            tokio::fs::rename(&part_filepath, &filepath).await.context("Failed to rename partial file")?;
            pb.finish_with_message(format!("Completed   {}", filepath.file_name().unwrap().to_string_lossy()));
        }
        
        // Update completed files count
        self.downloaded_files.fetch_add(1, Ordering::Relaxed);
        
        Ok(())
    }

    fn verify_hash(&self, url: &str, computed: &str, temp_path: &PathBuf) -> Result<()> {
        if let Some(expected) = self.expected_hashes.get(url) {
            if !expected.eq_ignore_ascii_case(computed) {
                // Remove corrupted temp file to avoid confusion
                let _ = std::fs::remove_file(temp_path);
                return Err(anyhow!(
                    "Hash mismatch: expected {}, got {}",
                    expected,
                    computed
                ));
            }
        }
        Ok(())
    }

}
