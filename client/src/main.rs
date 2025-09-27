#[cfg(any(target_os = "linux", target_os = "macos"))]
mod common;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
mod windows;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <mountpoint>", args[0]);
        std::process::exit(1);
    }

    #[cfg(target_os = "linux")]
    linux::run(&args[1]);

    #[cfg(target_os = "macos")]
    macos::run(&args[1]);

    #[cfg(target_os = "windows")]
    windows::run();
}
