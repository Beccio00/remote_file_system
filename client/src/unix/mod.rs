mod remote_fs;
mod linux;
mod macos;
use daemonize::Daemonize;

/// Dispatches startup to the Unix implementation for the current target OS.
pub fn run(cli: &crate::cli::Cli) {
    daemonize_if_requested(cli);

    #[cfg(target_os = "linux")]
    linux::run(cli);

    #[cfg(target_os = "macos")]
    macos::run(cli);
}

fn daemonize_if_requested(cli: &crate::cli::Cli) {
    if !cli.daemon {
        return;
    }

    let daemonize = Daemonize::new().working_directory(".").umask(0o022);
    match daemonize.start() {
        Ok(_) => eprintln!("Daemonized successfully (PID {})", std::process::id()),
        Err(e) => {
            eprintln!("Failed to daemonize: {}", e);
            std::process::exit(1);
        }
    }
}
