//! Windows WinFSP mount setup.
//! Mirrors mount.rs (FUSE) but uses the WinFSP FileSystemHost API.

use crate::remote_win_fs::RemoteWinFS;
use crate::types::CacheConfig;
use winfsp::host::{FileSystemHost, VolumeParams};

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

    let ctx = RemoteWinFS::new(server_url, cache);

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
    println!("Press Ctrl+C to unmount and exit.");

    loop {
        std::thread::park();
    }
}
