use clap::Parser;

mod types;
mod remote_client;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod remote_fs;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod mount;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
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

    /// Run as a background daemon
    #[arg(long, default_value = "false")]
    pub daemon: bool,
}

fn main() {
    let cli = Cli::parse();

    // Daemonize works only with unix systems
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

    #[cfg(target_os = "linux")]
    linux::run(&cli);

    #[cfg(target_os = "macos")]
    macos::run(&cli);

    #[cfg(target_os = "windows")]
    windows::run(&cli);
}
