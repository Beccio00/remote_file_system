mod fs;

#[cfg(target_os = "linux")]
use crate::fs::linux::LinuxFuseAdapter;

#[cfg(target_os = "macos")]
use crate::fs::macos::MacOSFuseAdapter;

#[cfg(target_os = "windows")]
use crate::fs::windows::WindowsFuseAdapter;

fn main() {
    
    #[cfg(target_os = "linux")]
    let adapter = LinuxFuseAdapter;

    #[cfg(target_os = "macos")]
    let adapter = fs::MacOSFuseAdapter;

    #[cfg(target_os = "windows")]
    let adapter = fs::WindowsFuseAdapter;

}