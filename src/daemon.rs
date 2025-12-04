use anyhow::{Context, Result};

#[cfg(unix)]
use daemonize::Daemonize;
#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use nix::sys::signal::{self, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

#[cfg(unix)]
pub fn start_daemon() -> Result<()> {
    // Redirect both stdout and stderr to the same log file for easier following
    let stdout = File::create("rdl.log").context("Failed to create log file")?;
    let stderr = stdout.try_clone().context("Failed to clone log file handle")?;

    let daemonize = Daemonize::new()
        .pid_file("rdl.pid")
        .chown_pid_file(true)
        .working_directory(".")
        .stdout(stdout)
        .stderr(stderr);

    match daemonize.start() {
        Ok(_) => {
            println!("Success, daemonized");
            Ok(())
        },
        Err(e) => Err(anyhow::anyhow!("Error starting daemon: {}", e)),
    }
}

#[cfg(unix)]
pub fn get_daemon_pid() -> Result<Option<i32>> {
    let pid_file = "rdl.pid";
    if !std::path::Path::new(pid_file).exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(pid_file)?;
    let pid = content.trim().parse::<i32>()?;
    Ok(Some(pid))
}

#[cfg(unix)]
pub fn send_signal(pid: i32, signal: Signal) -> Result<()> {
    signal::kill(Pid::from_raw(pid), signal)?;
    Ok(())
}

#[cfg(unix)]
pub fn stop_daemon() -> Result<()> {
    if let Some(pid) = get_daemon_pid()? {
        send_signal(pid, Signal::SIGTERM)?;
        println!("Stopped daemon (PID: {})", pid);
        let _ = std::fs::remove_file("rdl.pid");
    } else {
        println!("Daemon is not running.");
    }
    Ok(())
}

#[cfg(unix)]
pub fn pause_daemon() -> Result<()> {
    if let Some(pid) = get_daemon_pid()? {
        send_signal(pid, Signal::SIGSTOP)?;
        println!("Paused daemon (PID: {})", pid);
    } else {
        println!("Daemon is not running.");
    }
    Ok(())
}

#[cfg(unix)]
pub fn resume_daemon() -> Result<()> {
    if let Some(pid) = get_daemon_pid()? {
        send_signal(pid, Signal::SIGCONT)?;
        println!("Resumed daemon (PID: {})", pid);
    } else {
        println!("Daemon is not running.");
    }
    Ok(())
}

#[cfg(unix)]
pub fn cleanup_pid_file() {
    let _ = std::fs::remove_file("rdl.pid");
}