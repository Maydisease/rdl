use anyhow::Result;
use url::Url;
use std::path::PathBuf;

pub fn get_filename_from_url(url_str: &str) -> Result<String> {
    let url = Url::parse(url_str)?;
    
    if let Some(segments) = url.path_segments() {
        if let Some(filename) = segments.last() {
            if !filename.is_empty() {
                return Ok(filename.to_string());
            }
        }
    }

    // Fallback if no filename found in path
    Ok(format!("download_{}", uuid::Uuid::new_v4()))
}

pub fn sanitize_filename(filename: &str) -> String {
    filename.replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '-' && c != '_', "_")
}

pub fn get_unique_filepath(dir: &PathBuf, filename: &str) -> PathBuf {
    let mut path = dir.join(filename);
    let mut counter = 1;

    while path.exists() {
        let file_stem = path.file_stem().unwrap().to_string_lossy();
        let extension = path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        
        let new_filename = if extension.is_empty() {
            format!("{}_{}", file_stem, counter)
        } else {
            format!("{}_{}.{}", file_stem, counter, extension)
        };
        
        path = dir.join(new_filename);
        counter += 1;
    }
    path
}