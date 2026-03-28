mod remote_fs;
mod mount;

use crate::cli::Cli;

/// Builds cache settings from CLI and starts the Windows filesystem backend.
/// Handles unmount requests if the --unmount flag is present.
pub fn run(cli: &Cli) {
    if cli.unmount {
        request_unmount(&cli.mountpoint);
        return;
    }

    daemonize_if_requested(cli);

    let cache = cli.cache_config();
    mount::run(&cli.mountpoint, &cli.server_url, cache);
}

/// Sends an unmount request to a running Windows daemon instance.
fn request_unmount(mountpoint: &str) {
    match mount::request_unmount(mountpoint) {
        Ok(true) => println!("Unmount requested for {}", mountpoint),
        Ok(false) => {
            eprintln!("No active daemon mount found for {}", mountpoint);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Failed to request unmount for {}: {}", mountpoint, e);
            std::process::exit(1);
        }
    }
}

fn daemonize_if_requested(cli: &Cli) {
    if !cli.daemon {
        return;
    }

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
