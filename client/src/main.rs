use clap::Parser;

mod types;
mod remote_client;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

/// Remote File System — mount a remote filesystem via FUSE
#[derive(Parser, Debug)]
#[command(name = "remote-fs", version, about, long_about = None)]
pub struct Cli {
    /// Local path where the filesystem will be mounted (e.g. /tmp/mnt)
    pub mountpoint: String,

    /// URL of the remote server
    #[arg(long, default_value = "http://127.0.0.1:8000")]
    pub server_url: String,

    /// Directory cache TTL in seconds
    #[arg(long, default_value = "5")]
    pub dir_cache_ttl: u64,

    /// File cache TTL in seconds
    #[arg(long, default_value = "10")]
    pub file_cache_ttl: u64,

    /// Maximum file cache size in MB
    #[arg(long, default_value = "64")]
    pub max_cache_mb: usize,

    /// Disable caching entirely
    #[arg(long, default_value = "false")]
    pub no_cache: bool,

    #[cfg(any(unix, windows))]
    /// Run as a background daemon
    #[arg(long, default_value = "false")]
    pub daemon: bool,

    #[cfg(windows)]
    /// Request clean unmount of an existing daemon mount at <MOUNTPOINT> (e.g. R:)
    #[arg(long, default_value = "false")]
    pub unmount: bool,
}

fn main() {
    let cli = Cli::parse();

    #[cfg(windows)]
    if cli.unmount {
        windows::request_unmount(&cli.mountpoint);
        return;
    }

    // Unix daemonization via daemonize crate.
    #[cfg(unix)]
    if cli.daemon {
        use daemonize::Daemonize;
        let daemonize = Daemonize::new()
            .working_directory(".")
            .umask(0o022);
        match daemonize.start() {
            Ok(_) => eprintln!("Daemonized successfully (PID {})", std::process::id()),
            Err(e) => {
                eprintln!("Failed to daemonize: {}", e);
                std::process::exit(1);
            }
        }
    }

    #[cfg(windows)]
    if cli.daemon {
        use std::fs;
        use std::os::windows::process::CommandExt;
        use std::path::PathBuf;
        use std::process::{Command, Stdio};
        use std::time::{SystemTime, UNIX_EPOCH};

        // Relaunch without --daemon using detached flags, then exit parent.
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let exe = std::env::current_exe().unwrap_or_else(|e| {
            eprintln!("Failed to get executable path: {}", e);
            std::process::exit(1);
        });

        let args: Vec<_> = std::env::args_os()
            .skip(1)
            .filter(|arg| arg != "--daemon")
            .collect();

        // Spawn daemon from a temp copy to avoid locking target/debug/client.exe.
        let mut daemon_exe: PathBuf = std::env::temp_dir();
        daemon_exe.push("remote-fs-daemon");
        if let Err(e) = fs::create_dir_all(&daemon_exe) {
            eprintln!("Failed to prepare daemon temp directory: {}", e);
            std::process::exit(1);
        }

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        daemon_exe.push(format!("client-daemon-{}-{}.exe", std::process::id(), ts));

        if let Err(e) = fs::copy(&exe, &daemon_exe) {
            eprintln!("Failed to stage daemon executable: {}", e);
            std::process::exit(1);
        }

        let mut child = Command::new(&daemon_exe);
        child
            .args(args)
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        match child.spawn() {
            Ok(_) => {
                eprintln!("Daemonized successfully");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("Failed to daemonize on Windows: {}", e);
                std::process::exit(1);
            }
        }
    }

    #[cfg(unix)]
    unix::run(&cli);

    #[cfg(windows)]
    windows::run(&cli);
}
