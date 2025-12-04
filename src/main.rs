mod commands;
mod daemon;
mod downloader;
mod state;
mod hashing;
mod utils;
mod providers;
mod cli;

use anyhow::Result;
use clap::Parser;
use crate::cli::VerifyMode;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Single URL to download (optional, if provided, tasks-file is ignored)
    #[arg(index = 1)]
    url: Option<String>,

    /// Path to the file containing URLs (one per line)
    #[arg(short = 't', long = "tasks-file", default_value = "download.txt")]
    tasks_file: PathBuf,

    /// Directory to save downloaded files
    #[arg(short = 'd', long = "download-dir", default_value = "downloads")]
    download_dir: PathBuf,

    /// Maximum number of concurrent downloads (defaults to number of logical CPUs)
    #[arg(short = 'c', long)]
    concurrency: Option<usize>,

    /// Global rate limit in bytes per second (e.g., 1048576 for 1MB/s)
    #[arg(short = 'r', long)]
    rate_limit: Option<u32>,

    /// Number of splits per file (segmented download)
    #[arg(short = 's', long, default_value_t = 8)]
    split: usize,

    /// Run in background (daemon mode) [Unix only]
    #[arg(short = 'd', long)]
    daemon: bool,

    /// Follow the log file of the daemon process (like tail -f) [Unix only]
    #[arg(short = 'f', long)]
    follow: bool,

    /// Stop the daemon process [Unix only]
    #[arg(short = 'x', long)]
    stop: bool,

    /// Pause the daemon process [Unix only]
    #[arg(short = 'p', long)]
    pause: bool,

    /// Resume the daemon process [Unix only]
    #[arg(short = 'u', long)]
    resume: bool,

    /// List all downloads and their status
    #[arg(short = 'l', long)]
    list: bool,

    /// Fetch ModelScope file list (org/model) and write to --input path, then exit
    #[arg(short = 'f', long = "fetch-list")]
    fetch_list: Option<String>,

    /// Revision/branch used when generating resolve URLs (default: master)
    #[arg(short = 'b', long = "branch", default_value = "master")]
    branch: String,

    /// Provider name (e.g., modelscope, huggingface)
    #[arg(short = 'P', long, default_value = "modelscope")]
    provider: String,

    /// Hash verification: auto (only when hash provided), on (require hash), off (skip)
    #[arg(long = "verify-hash", value_enum, default_value = "auto")]
    verify_hash: VerifyMode,
}

fn main() -> Result<()> {
    let mut args = Args::parse();
    let input_is_default = args.tasks_file == PathBuf::from("download.txt");
    let output_is_default = args.download_dir == PathBuf::from("downloads");

    // Resolve paths to absolute before daemonizing to avoid issues with working directory
    // Only if we are NOT in single URL mode (because in single URL mode, tasks_file might be default but unused)
    if args.url.is_none() {
        if let Ok(abs_input) = std::fs::canonicalize(&args.tasks_file) {
            args.tasks_file = abs_input;
        }
    }
    // Output dir might not exist yet, so we resolve it relative to current dir
    if args.download_dir.is_relative() {
        if let Ok(cwd) = std::env::current_dir() {
            args.download_dir = cwd.join(&args.download_dir);
        }
    }

    // Handle synchronous commands (list, stop, pause, resume, follow) BEFORE starting runtime
    if args.list || args.stop || args.pause || args.resume || args.follow {
        let rt = tokio::runtime::Runtime::new()?;
        return rt.block_on(async {
            handle_sync_commands(&args).await
        });
    }

    if let Some(model) = &args.fetch_list {
        let rt = tokio::runtime::Runtime::new()?;
        let generated_input = rt.block_on(async {
            crate::commands::generate_download_list(
                model,
                args.tasks_file.clone(),
                input_is_default,
                args.branch.clone(),
                args.provider.clone()
            ).await
        })?;

        // Use the generated list as the new input (absolute if possible)
        if let Ok(abs_input) = std::fs::canonicalize(&generated_input) {
            args.tasks_file = abs_input;
        } else {
            args.tasks_file = generated_input;
        }

        // If user didn't override --output, store downloads alongside the generated list
        if output_is_default {
            if let Some(parent) = args.tasks_file.parent() {
                args.download_dir = parent.to_path_buf();
            }
        }
    }

    #[cfg(unix)]
    if args.daemon {
        crate::daemon::start_daemon()?;
    }

    // Now start the runtime for the actual download task
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        if let Some(url) = args.url {
            crate::commands::run_single_download(
                url,
                args.download_dir,
                args.concurrency,
                args.rate_limit,
                args.split,
                args.verify_hash,
            ).await
        } else {
            crate::commands::run_downloads(
                args.tasks_file,
                args.download_dir,
                args.concurrency,
                args.rate_limit,
                args.split,
                args.daemon,
                args.verify_hash,
            ).await
        }
    })
}

async fn handle_sync_commands(args: &Args) -> Result<()> {
    if args.list {
        return crate::commands::list_downloads(args.download_dir.clone(), args.tasks_file.clone()).await;
    }

    #[cfg(unix)]
    {
        if args.stop {
            return crate::daemon::stop_daemon();
        }

        if args.pause {
            return crate::daemon::pause_daemon();
        }

        if args.resume {
            return crate::daemon::resume_daemon();
        }

        if args.follow {
            return crate::commands::follow_log(args.download_dir.clone(), args.tasks_file.clone()).await;
        }
    }
    Ok(())
}
