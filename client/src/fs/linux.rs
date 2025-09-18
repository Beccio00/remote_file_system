use crate::fs::{FuseAdapter, FsError, MountOptions};
use async_trait::async_trait;
use fuser::{Filesystem, MountOption};
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::collections::HashMap;
use std::time::Duration;
use std::thread;
use std::fs;
use log::{info, debug, warn, error};
// reqwest client removed for now (we use simple blocking calls when needed)

/// Semplice implementazione Linux usando `fuser` (sincrono callbacks)
pub struct LinuxFuseAdapter {
    server_url: String,
    is_mounted: Arc<RwLock<bool>>,
    current_mountpoint: Arc<RwLock<Option<String>>>,
    // stato minimo per il filesystem
    inode_map: Arc<Mutex<HashMap<String, u64>>>,
}

impl LinuxFuseAdapter {
    pub fn new_internal(server_url: String) -> Self {
        Self {
            server_url,
            is_mounted: Arc::new(RwLock::new(false)),
            current_mountpoint: Arc::new(RwLock::new(None)),
            inode_map: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

struct RemoteLinuxFs {
    adapter: Arc<LinuxFuseAdapter>,
}

impl RemoteLinuxFs {
    fn new(adapter: Arc<LinuxFuseAdapter>) -> Self {
        Self { adapter }
    }
}

impl Filesystem for RemoteLinuxFs {
    // Implement only minimal callbacks so mount works; real logic can call adapter.client (blocking)
    fn init(&mut self, _req: &fuser::Request, _cfg: &mut fuser::KernelConfig) -> Result<(), libc::c_int> {
        debug!("RemoteFs init");
        Ok(())
    }

    fn destroy(&mut self) {
        debug!("RemoteFs destroy");
    }

    fn readdir(&mut self, _req: &fuser::Request, ino: u64, _fh: u64, offset: i64, mut reply: fuser::ReplyDirectory) {
        debug!("readdir ino={} offset={}", ino, offset);

        if ino != 1 {
            reply.error(libc::ENOTDIR);
            return;
        }

        if offset == 0 {
            let _ = reply.add(1, 1, fuser::FileType::Directory, ".");
            let _ = reply.add(1, 2, fuser::FileType::Directory, "..");
        }

        reply.ok();
    }

    fn getattr(&mut self, _req: &fuser::Request, ino: u64, _fh: Option<u64>, reply: fuser::ReplyAttr) {
        debug!("getattr ino={}", ino);
        if ino == 1 {
            let ttl = Duration::new(1, 0);
            let attr = fuser::FileAttr {
                ino: 1,
                size: 0,
                blocks: 0,
                atime: std::time::SystemTime::now(),
                mtime: std::time::SystemTime::now(),
                ctime: std::time::SystemTime::now(),
                crtime: std::time::SystemTime::now(),
                kind: fuser::FileType::Directory,
                perm: 0o755,
                nlink: 2,
                uid: unsafe { libc::getuid() },
                gid: unsafe { libc::getgid() },
                rdev: 0,
                flags: 0,
                blksize: 512,
            };
            reply.attr(&ttl, &attr);
            return;
        }
        reply.error(libc::ENOENT);
    }

    fn read(&mut self, _req: &fuser::Request, ino: u64, _fh: u64, offset: i64, size: u32, _flags: i32, _lock_owner: Option<u64>, reply: fuser::ReplyData) {
        debug!("read ino={} offset={} size={}", ino, offset, size);
        reply.error(libc::ENOENT);
    }
}

#[async_trait]
impl FuseAdapter for LinuxFuseAdapter {
    fn init() -> Result<Self, FsError> where Self: Sized {
        Ok(LinuxFuseAdapter::new_internal("http://localhost:8000".to_string()))
    }

    fn new(server_url: String) -> Result<Self, FsError> where Self: Sized {
        Ok(LinuxFuseAdapter::new_internal(server_url))
    }

    async fn mount(&self, mountpoint: &str, options: Option<MountOptions>) -> Result<(), FsError> {
        // validate mountpoint
        let path = Path::new(mountpoint);
        if !path.exists() {
            fs::create_dir_all(path).map_err(FsError::IoError)?;
        }

        // spawn a blocking thread to run fuser::mount
        let adapter = Arc::new(self.clone());
        let mount_str = mountpoint.to_string();

        // simple mount options
        let mut mountopts = vec![MountOption::FSName("remote-fs".to_string())];
        if let Some(opts) = options {
            if opts.read_only { mountopts.push(MountOption::RO); }
            if opts.auto_unmount { mountopts.push(MountOption::AutoUnmount); }
        }

        let thread_handle = thread::spawn(move || {
            let fs = RemoteLinuxFs::new(adapter);
            // mount in foreground so thread blocks until unmounted
            match fuser::mount2(fs, &mount_str, &mountopts) {
                Ok(_) => info!("mounted {}", mount_str),
                Err(e) => error!("mount error {}: {}", mount_str, e),
            }
        });

        // mark mounted
    *self.is_mounted.write().unwrap() = true;
    *self.current_mountpoint.write().unwrap() = Some(mountpoint.to_string());

    // drop the handle to let the thread run independently
    drop(thread_handle);

        Ok(())
    }

    async fn unmount(&self, mountpoint: &str) -> Result<(), FsError> {
        if !self.is_mounted(mountpoint)? {
            return Err(FsError::Other("not mounted".to_string()));
        }

        // call fusermount -u
        let output = std::process::Command::new("fusermount")
            .arg("-u")
            .arg(mountpoint)
            .output();

        match output {
            Ok(o) if o.status.success() => info!("unmounted {}", mountpoint),
            Ok(o) => warn!("fusermount returned {}", o.status),
            Err(e) => warn!("failed to run fusermount: {}", e),
        }

        *self.is_mounted.write().unwrap() = false;
        *self.current_mountpoint.write().unwrap() = None;
        Ok(())
    }

    fn is_mounted(&self, mountpoint: &str) -> Result<bool, FsError> {
        let mounted = *self.is_mounted.read().unwrap();
        let current = self.current_mountpoint.read().unwrap().clone();
        Ok(mounted && current.as_deref() == Some(mountpoint))
    }

    async fn wait_until_unmount(&self) -> Result<(), FsError> {
        while self.is_mounted(&self.current_mountpoint.read().unwrap().clone().unwrap_or_default())? {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(())
    }
}

impl Clone for LinuxFuseAdapter {
    fn clone(&self) -> Self {
        Self { 
            server_url: self.server_url.clone(),
            is_mounted: Arc::clone(&self.is_mounted),
            current_mountpoint: Arc::clone(&self.current_mountpoint),
            inode_map: Arc::clone(&self.inode_map),
        }
    }
}