use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DownloadState {
    pub url: String,
    pub total_size: u64,
    pub parts: Vec<PartState>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PartState {
    pub index: usize,
    pub start_byte: u64,
    pub end_byte: u64,
    pub current_byte: u64,
    pub completed: bool,
}