use anyhow::Result;
use sha2::{Sha256, Digest};
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

pub async fn calculate_hash(filepath: &PathBuf) -> Result<String> {
    let mut file = File::open(filepath).await?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];

    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Ok(hex::encode(hasher.finalize()))
}