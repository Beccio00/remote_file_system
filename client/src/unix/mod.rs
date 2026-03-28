mod remote_fs;
mod linux;
mod macos;

/// Dispatches startup to the Unix implementation for the current target OS.
pub fn run(cli: &crate::Cli) {
    daemonize_if_requested(cli);

    #[cfg(target_os = "linux")]
    linux::run(cli);

    #[cfg(target_os = "macos")]
    macos::run(cli);
}

fn daemonize_if_requested(cli: &crate::Cli) {
    if !cli.daemon {
        return;
    }

    use daemonize::Daemonize;
    let daemonize = Daemonize::new().working_directory(".").umask(0o022);
    match daemonize.start() {
        Ok(_) => eprintln!("Daemonized successfully (PID {})", std::process::id()),
        Err(e) => {
            eprintln!("Failed to daemonize: {}", e);
            std::process::exit(1);
        }
    }
}
