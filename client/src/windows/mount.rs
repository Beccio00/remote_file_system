use super::remote_fs::RemoteFS;
use crate::types::CacheConfig;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use winfsp::host::{FileSystemHost, VolumeParams};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    CreateEventW, EVENT_MODIFY_STATE, OpenEventW, SetEvent, WaitForSingleObject,
};

/// Canonicalizes mountpoints so daemon and unmount commands share the same key.
fn normalize_mountpoint(mountpoint: &str) -> String {
    mountpoint
        .trim()
        .trim_end_matches('\\')
        .to_ascii_uppercase()
}

/// Builds the named event identifier used to request daemon shutdown.
fn event_name_for_mount(mountpoint: &str) -> String {
    format!("Local\\remote-fs-unmount-{}", normalize_mountpoint(mountpoint))
}

/// Converts UTF-8 text to a null-terminated UTF-16 string for Win32 APIs.
fn to_wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// Creates a per-mount event used by external unmount requests.
fn create_shutdown_event(mountpoint: &str) -> Result<HANDLE, String> {
    let name = event_name_for_mount(mountpoint);
    let wide = to_wide_null(&name);
    let handle = unsafe { CreateEventW(std::ptr::null(), 1, 0, wide.as_ptr()) };
    if handle.is_null() {
        return Err("CreateEventW failed".to_string());
    }
    Ok(handle)
}

/// Signals an active mount daemon to stop and unmount.
pub fn request_unmount(mountpoint: &str) -> Result<bool, String> {
    let name = event_name_for_mount(mountpoint);
    let wide = to_wide_null(&name);

    let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, wide.as_ptr()) };
    if handle.is_null() {
        return Ok(false);
    }

    let ok = unsafe { SetEvent(handle) } != 0;
    unsafe {
        CloseHandle(handle);
    }

    if ok {
        Ok(true)
    } else {
        Err("SetEvent failed".to_string())
    }
}

/// Starts the WinFSP dispatcher and keeps it alive until shutdown is requested.
pub fn run(mountpoint: &str, server_url: &str, cache: CacheConfig) {
    println!("Mounting at: {}", mountpoint);
    println!("Server: {}", server_url);
    println!(
        "Cache: dir_ttl={}s, file_ttl={}s, max={}MB",
        cache.dir_ttl.as_secs(),
        cache.file_ttl.as_secs(),
        cache.max_file_cache_bytes / 1024 / 1024,
    );

    let _init = winfsp::winfsp_init_or_die();

    let ctx = RemoteFS::new(server_url, cache);

    let mut params = VolumeParams::new();
    params
        .filesystem_name("remote-fs")
        .file_info_timeout(1000)
        .case_sensitive_search(false)
        .case_preserved_names(true)
        .unicode_on_disk(true);

    let mut host =
        FileSystemHost::new(params, ctx).expect("Failed to create WinFSP filesystem host");

    let mp = std::ffi::OsString::from(mountpoint);
    host.mount(mp).expect("Failed to mount filesystem");
    host.start().expect("Failed to start filesystem dispatcher");

    println!("Filesystem mounted successfully at {}", mountpoint);
    println!("Press Ctrl+C for a clean unmount and exit.");

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_handler = Arc::clone(&shutdown);
    let shutdown_event = create_shutdown_event(mountpoint).ok();

    if let Err(e) = ctrlc::set_handler(move || {
        shutdown_handler.store(true, Ordering::SeqCst);
    }) {
        eprintln!("Warning: failed to install Ctrl+C handler: {}", e);
    }

    while !shutdown.load(Ordering::SeqCst) {
        if let Some(event) = shutdown_event {
            let wait = unsafe { WaitForSingleObject(event, 250) };
            if wait == WAIT_OBJECT_0 {
                shutdown.store(true, Ordering::SeqCst);
                break;
            }
            if wait != WAIT_TIMEOUT {
                shutdown.store(true, Ordering::SeqCst);
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    println!("Shutdown requested. Unmounting filesystem...");
    host.unmount();
    host.stop();
    if let Some(event) = shutdown_event {
        unsafe {
            CloseHandle(event);
        }
    }
    println!("Filesystem unmounted.");
}
