use crate::fs::{FuseAdapter, FsError, MountOptions};
use async_trait::async_trait;
use fuser::{Filesystem, MountOption};
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::collections::HashMap;
use serde_json::Value as JsonValue;
use crossbeam_channel::{bounded, unbounded, Sender as CBSender, RecvTimeoutError};
use reqwest::Client as AsyncClient;
use std::time::Duration;
use std::thread;
use std::fs;
use log::{info, debug, warn, error};
// reqwest client removed for now (we use simple blocking calls when needed)

/// Semplice implementazione Linux usando `fuser` (sincrono callbacks)
pub struct LinuxFuseAdapter {
    server_url: String,
    // request channel to dispatcher runtime
    req_tx: CBSender<Request>,
    is_mounted: Arc<RwLock<bool>>,
    current_mountpoint: Arc<RwLock<Option<String>>>,
    // stato minimo per il filesystem
    inode_map: Arc<Mutex<HashMap<String, u64>>>,
}

impl LinuxFuseAdapter {
    pub fn new_internal(server_url: String) -> Self {
        // create crossbeam channel and spawn a background tokio runtime thread
        let (tx, rx) = unbounded::<Request>();
        let base = server_url.clone();
        // spawn background thread with a tokio runtime that processes requests async
        thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
                .expect("failed to build tokio runtime");
            let client = AsyncClient::new();
            // blocking recv loop - this runs in its own thread
            loop {
                match rx.recv() {
                    Ok(req) => {
                        let client = client.clone();
                        let base = base.clone();
                        // process request synchronously on this thread via runtime
                        let _ = rt.block_on(async move {
                            match req.kind {
                                RequestKind::List(path) => {
                                    let url = format!("{}/list{}", base, path);
                                    let res = client.get(&url).send().await;
                                    match res {
                                        Ok(r) if r.status().is_success() => {
                                            match r.bytes().await {
                                                Ok(b) => { let _ = req.resp.send(Ok(b.to_vec())); }
                                                Err(_) => { let _ = req.resp.send(Err(())); }
                                            }
                                        }
                                        _ => { let _ = req.resp.send(Err(())); }
                                    }
                                }
                                RequestKind::GetFile(path) => {
                                    let url = format!("{}/files{}", base, path);
                                    let res = client.get(&url).send().await;
                                    match res {
                                        Ok(r) if r.status().is_success() => {
                                            match r.bytes().await {
                                                Ok(b) => { let _ = req.resp.send(Ok(b.to_vec())); }
                                                Err(_) => { let _ = req.resp.send(Err(())); }
                                            }
                                        }
                                        _ => { let _ = req.resp.send(Err(())); }
                                    }
                                }
                            }
                        });
                    }
                    Err(_) => break, // channel closed, exit
                }
            }
        });

        Self {
            server_url: server_url.clone(),
            req_tx: tx,
            is_mounted: Arc::new(RwLock::new(false)),
            current_mountpoint: Arc::new(RwLock::new(None)),
            inode_map: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

// request/response types for dispatcher
enum RequestKind { List(String), GetFile(String) }
struct Request { kind: RequestKind, resp: CBSender<Result<Vec<u8>, ()>> }

impl LinuxFuseAdapter {
    // simple inode allocator: starts at 2 (1 is root)
    fn ino_for_path(&self, path: &str) -> u64 {
        let mut map = self.inode_map.lock().unwrap();
        if let Some(&ino) = map.get(path) {
            ino
        } else {
            let next = (map.len() as u64) + 2;
            map.insert(path.to_string(), next);
            next
        }
    }

    fn path_for_ino(&self, ino: u64) -> Option<String> {
        if ino == 1 { return Some("/".to_string()); }
        let map = self.inode_map.lock().unwrap();
        for (p, &i) in map.iter() {
            if i == ino { return Some(p.clone()); }
        }
        None
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

    fn lookup(&mut self, _req: &fuser::Request, parent: u64, name: &std::ffi::OsStr, reply: fuser::ReplyEntry) {
        let name_str = match name.to_str() { Some(s) => s, None => { reply.error(libc::ENOENT); return; } };
        debug!("lookup parent={} name={}", parent, name_str);

        let parent_path = match self.adapter.path_for_ino(parent) { Some(p) => p, None => { reply.error(libc::ENOENT); return; } };

        // ask dispatcher for entries under parent
        let (r_tx, r_rx) = bounded(1);
        let _ = self.adapter.req_tx.send(Request { kind: RequestKind::List(parent_path.clone()), resp: r_tx });
        match r_rx.recv_timeout(Duration::from_millis(400)) {
            Ok(Ok(bytes)) => {
                if let Ok(entries) = serde_json::from_slice::<Vec<JsonValue>>(&bytes) {
                    for ent in entries.into_iter() {
                        if ent.get("name").and_then(|v| v.as_str()) == Some(name_str) {
                            let is_dir = ent.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false);
                            let child_path = if parent_path == "/" { format!("/{}", name_str) } else { format!("{}/{}", parent_path.trim_end_matches('/'), name_str) };
                            let ino = self.adapter.ino_for_path(&child_path);
                            let ttl = Duration::new(1, 0);
                            let attr = if is_dir {
                                fuser::FileAttr {
                                    ino,
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
                                }
                            } else {
                                fuser::FileAttr {
                                    ino,
                                    size: ent.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                                    blocks: 0,
                                    atime: std::time::SystemTime::now(),
                                    mtime: std::time::SystemTime::now(),
                                    ctime: std::time::SystemTime::now(),
                                    crtime: std::time::SystemTime::now(),
                                    kind: fuser::FileType::RegularFile,
                                    perm: 0o644,
                                    nlink: 1,
                                    uid: unsafe { libc::getuid() },
                                    gid: unsafe { libc::getgid() },
                                    rdev: 0,
                                    flags: 0,
                                    blksize: 512,
                                }
                            };
                            reply.entry(&ttl, &attr, 0);
                            return;
                        }
                    }
                }
                reply.error(libc::ENOENT);
                return;
            }
            Ok(Err(_)) => { reply.error(libc::EIO); return; }
            Err(_) => { reply.error(libc::ETIMEDOUT); return; }
        }
    }

    fn access(&mut self, _req: &fuser::Request, ino: u64, _mask: i32, reply: fuser::ReplyEmpty) {
        debug!("access ino={}", ino);
        // naive: always allow access if inode exists
        if self.adapter.path_for_ino(ino).is_some() || ino == 1 {
            reply.ok();
        } else {
            reply.error(libc::ENOENT);
        }
    }

    fn opendir(&mut self, _req: &fuser::Request, ino: u64, _flags: i32, reply: fuser::ReplyOpen) {
        debug!("opendir ino={}", ino);
        reply.opened(0, 0);
    }

    fn open(&mut self, _req: &fuser::Request, ino: u64, _flags: i32, reply: fuser::ReplyOpen) {
        debug!("open ino={}", ino);
        reply.opened(0, 0);
    }

    fn readdir(&mut self, _req: &fuser::Request, ino: u64, _fh: u64, offset: i64, mut reply: fuser::ReplyDirectory) {
        debug!("readdir ino={} offset={}", ino, offset);

        let path = match self.adapter.path_for_ino(ino) {
            Some(p) => p,
            None => { reply.error(libc::ENOENT); return; }
        };

        // send request to dispatcher and wait
        let (r_tx, r_rx) = bounded(1);
        let _ = self.adapter.req_tx.send(Request { kind: RequestKind::List(path.clone()), resp: r_tx });
        match r_rx.recv_timeout(Duration::from_millis(400)) {
            Ok(Ok(bytes)) => {
                match serde_json::from_slice::<Vec<JsonValue>>(&bytes) {
                    Ok(entries) => {
                        if offset == 0 {
                            let _ = reply.add(ino, 1, fuser::FileType::Directory, ".");
                            let _ = reply.add(ino, 2, fuser::FileType::Directory, "..");
                        }
                        for (i, ent) in entries.into_iter().enumerate().skip(offset as usize) {
                            let name = ent.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                            let is_dir = ent.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false);
                            let child_path = if path == "/" { format!("/{}", name) } else { format!("{}/{}", path.trim_end_matches('/'), name) };
                            let ino = self.adapter.ino_for_path(&child_path);
                            let ftype = if is_dir { fuser::FileType::Directory } else { fuser::FileType::RegularFile };
                            let _ = reply.add(ino, (i + 3) as i64, ftype, name);
                        }
                        reply.ok();
                        return;
                    }
                    Err(_) => { reply.error(libc::EIO); return; }
                }
            }
            Ok(Err(_)) => { reply.error(libc::EIO); return; }
            Err(RecvTimeoutError::Timeout) => { reply.error(libc::ETIMEDOUT); return; }
            Err(_) => { reply.error(libc::EIO); return; }
        }
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

        // try to resolve path and query backend for metadata
        if let Some(path) = self.adapter.path_for_ino(ino) {
            // request file bytes via dispatcher
            let (r_tx, r_rx) = bounded(1);
            let _ = self.adapter.req_tx.send(Request { kind: RequestKind::GetFile(path.clone()), resp: r_tx });
            match r_rx.recv_timeout(Duration::from_millis(400)) {
                Ok(Ok(bytes)) => {
                    let ttl = Duration::new(1, 0);
                    let size = bytes.len() as u64;
                    let attr = fuser::FileAttr {
                        ino,
                        size,
                        blocks: 0,
                        atime: std::time::SystemTime::now(),
                        mtime: std::time::SystemTime::now(),
                        ctime: std::time::SystemTime::now(),
                        crtime: std::time::SystemTime::now(),
                        kind: fuser::FileType::RegularFile,
                        perm: 0o644,
                        nlink: 1,
                        uid: unsafe { libc::getuid() },
                        gid: unsafe { libc::getgid() },
                        rdev: 0,
                        flags: 0,
                        blksize: 512,
                    };
                    reply.attr(&ttl, &attr);
                    return;
                }
                Ok(Err(_)) => { reply.error(libc::EIO); return; }
                Err(RecvTimeoutError::Timeout) => { reply.error(libc::ETIMEDOUT); return; }
                Err(_) => { reply.error(libc::EIO); return; }
            }
        }
        reply.error(libc::ENOENT);
    }

    fn read(&mut self, _req: &fuser::Request, ino: u64, _fh: u64, offset: i64, size: u32, _flags: i32, _lock_owner: Option<u64>, reply: fuser::ReplyData) {
        debug!("read ino={} offset={} size={}", ino, offset, size);
        let path = match self.adapter.path_for_ino(ino) {
            Some(p) => p,
            None => { reply.error(libc::ENOENT); return; }
        };

        // use dispatcher
        let (r_tx, r_rx) = bounded(1);
        let _ = self.adapter.req_tx.send(Request { kind: RequestKind::GetFile(path.clone()), resp: r_tx });
        match r_rx.recv_timeout(Duration::from_millis(400)) {
            Ok(Ok(bytes)) => {
                let start = offset as usize;
                if start >= bytes.len() { reply.data(&[]); return; }
                let end = std::cmp::min(start + size as usize, bytes.len());
                reply.data(&bytes[start..end]);
                return;
            }
            Ok(Err(_)) => { reply.error(libc::EIO); return; }
            Err(RecvTimeoutError::Timeout) => { reply.error(libc::ETIMEDOUT); return; }
            Err(_) => { reply.error(libc::EIO); return; }
        }
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
            req_tx: self.req_tx.clone(),
            is_mounted: Arc::clone(&self.is_mounted),
            current_mountpoint: Arc::clone(&self.current_mountpoint),
            inode_map: Arc::clone(&self.inode_map),
        }
    }
}