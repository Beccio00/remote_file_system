
use fuser::{
    FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request
};
use libc::ENOENT;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom, Write as IoWrite};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

#[derive(Debug, Deserialize)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Buffer locale per file aperti in scrittura.
/// I dati vengono accumulati in un file temporaneo su disco
/// e inviati al server al flush/release.
pub struct WriteBuffer {
    pub file: std::fs::File,
    pub path: String,
    pub dirty: bool,
}

pub struct RemoteFS {
    client: Client,
    base_url: String,
    inode_counter: u64,
    inode_to_path: Arc<Mutex<HashMap<u64, String>>>,
    path_to_inode: Arc<Mutex<HashMap<String, u64>>>,
    write_buffers: HashMap<u64, WriteBuffer>,
    fh_counter: u64,
}

impl RemoteFS {
    pub fn new(base_url: &str) -> Self {
        let mut inode_to_path = HashMap::new();
        let mut path_to_inode = HashMap::new();

        // root
        inode_to_path.insert(1, "".to_string());
        path_to_inode.insert("".to_string(), 1);

        RemoteFS {
            client: Client::new(),
            base_url: base_url.to_string(),
            inode_counter: 1,
            inode_to_path: Arc::new(Mutex::new(inode_to_path)),
            path_to_inode: Arc::new(Mutex::new(path_to_inode)),
            write_buffers: HashMap::new(),
            fh_counter: 0,
        }
    }

    fn list_dir(&self, path: &str) -> Result<Vec<RemoteEntry>, anyhow::Error> {
        let url = format!("{}/list/{}", self.base_url, path);
        let resp = self.client.get(&url).send()?.error_for_status()?;
        Ok(resp.json::<Vec<RemoteEntry>>()?)
    }

    fn read_file(&self, path: &str) -> Result<Vec<u8>, anyhow::Error> {
        let url = format!("{}/files/{}", self.base_url, path);
        let resp = self.client.get(&url).send()?.error_for_status()?;
        Ok(resp.bytes()?.to_vec())
    }

    fn alloc_inode(&mut self, path: String) -> u64 {
        let mut p2i = self.path_to_inode.lock().unwrap();
        if let Some(&ino) = p2i.get(&path) {
            return ino;
        }
        self.inode_counter += 1;
        let ino = self.inode_counter;
        p2i.insert(path.clone(), ino);
        self.inode_to_path.lock().unwrap().insert(ino, path);
        ino
    }
}

impl Filesystem for RemoteFS {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let i2p = self.inode_to_path.lock().unwrap();
        let parent_path = i2p.get(&parent).cloned().unwrap_or_default();
        drop(i2p);

        let full_path = if parent_path.is_empty() {
            name.to_string_lossy().to_string()
        } else {
            format!("{}/{}", parent_path, name.to_string_lossy())
        };

        // Try to find the entry by listing the parent directory
        if let Ok(entries) = self.list_dir(&parent_path) {
            for entry in entries {
                if entry.name == name.to_string_lossy() {
                    // Allocate inode for this entry
                    let child_ino = self.alloc_inode(full_path);

                    let ttl = Duration::from_secs(1);
                    let kind = if entry.is_dir {
                        FileType::Directory
                    } else {
                        FileType::RegularFile
                    };

                    let attr = FileAttr {
                        ino: child_ino,
                        size: entry.size,
                        blocks: (entry.size + 511) / 512,
                        atime: SystemTime::now(),
                        mtime: SystemTime::now(),
                        ctime: SystemTime::now(),
                        crtime: SystemTime::now(),
                        kind,
                        perm: if entry.is_dir { 0o755 } else { 0o644 },
                        nlink: if entry.is_dir { 2 } else { 1 },
                        uid: 1000,
                        gid: 1000,
                        rdev: 0,
                        blksize: 512,
                        flags: 0,
                    };
                    reply.entry(&ttl, &attr, 0);
                    return;
                }
            }
        }

        reply.error(ENOENT);
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        let i2p = self.inode_to_path.lock().unwrap();
        let path = i2p.get(&ino).cloned();
        drop(i2p);

        let ttl = Duration::from_secs(1);

        if ino == 1 {
            // Root directory
            let attr = FileAttr {
                ino,
                size: 0,
                blocks: 0,
                atime: SystemTime::now(),
                mtime: SystemTime::now(),
                ctime: SystemTime::now(),
                crtime: SystemTime::now(),
                kind: FileType::Directory,
                perm: 0o755,
                nlink: 2,
                uid: 1000,
                gid: 1000,
                rdev: 0,
                blksize: 512,
                flags: 0,
            };
            reply.attr(&ttl, &attr);
            return;
        }

       if let Some(file_path) = path {
        let parent_path = if file_path.contains('/') {
            let parts: Vec<&str> = file_path.split('/').collect();
            parts[..parts.len()-1].join("/")
        } else {
            String::new()
        };

        if let Ok(entries) = self.list_dir(&parent_path) {
            let filename = file_path.split('/').last().unwrap_or("");
            for entry in entries {
                if entry.name == filename {
                    let kind = if entry.is_dir {
                        FileType::Directory
                    } else {
                        FileType::RegularFile
                    };

                    let attr = FileAttr {
                        ino,
                        size: entry.size,
                        blocks: (entry.size + 511) / 512,
                        atime: SystemTime::now(),
                        mtime: SystemTime::now(),
                        ctime: SystemTime::now(),
                        crtime: SystemTime::now(),
                        kind,
                        perm: if entry.is_dir { 0o755 } else { 0o644 },
                        nlink: if entry.is_dir { 2 } else { 1 },
                        uid: 1000,
                        gid: 1000,
                        rdev: 0,
                        blksize: 512,
                        flags: 0,
                    };
                    reply.attr(&ttl, &attr);
                    return;
                }
            }
        }
    }

        reply.error(ENOENT);
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let i2p = self.inode_to_path.lock().unwrap();
        let parent_path = i2p.get(&ino).unwrap_or(&"".to_string()).clone();
        drop(i2p);

        if offset == 0 {
            reply.add(ino, 1, FileType::Directory, ".");
            reply.add(ino, 2, FileType::Directory, "..");

            if let Ok(entries) = self.list_dir(&parent_path) {
                let mut idx = 3;
                for entry in entries {
                    let child_path = if parent_path.is_empty() {
                        entry.name.clone()
                    } else {
                        format!("{}/{}", parent_path, entry.name)
                    };
                    let child_ino = self.alloc_inode(child_path);
                    let kind = if entry.is_dir {
                        FileType::Directory
                    } else {
                        FileType::RegularFile
                    };
                    reply.add(child_ino, idx, kind, entry.name);
                    idx += 1;
                }
            }
        }
        reply.ok();
    }

    /// Apre un file. Se in modalità scrittura, scarica il contenuto
    /// attuale in un file temporaneo locale che funge da buffer.
    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: fuser::ReplyOpen) {
        self.fh_counter += 1;
        let fh = self.fh_counter;

        let access_mode = flags & libc::O_ACCMODE;
        let write_mode = access_mode == libc::O_WRONLY || access_mode == libc::O_RDWR;
        let truncate = (flags & libc::O_TRUNC) != 0;

        if write_mode || truncate {
            let i2p = self.inode_to_path.lock().unwrap();
            let path = i2p.get(&ino).cloned();
            drop(i2p);

            if let Some(file_path) = path {
                let mut tmp = tempfile::tempfile().unwrap();

                // Pre-fill con il contenuto esistente (a meno che non sia O_TRUNC)
                if !truncate {
                    if let Ok(data) = self.read_file(&file_path) {
                        let _ = tmp.write_all(&data);
                        let _ = tmp.seek(SeekFrom::Start(0));
                    }
                }

                self.write_buffers.insert(fh, WriteBuffer {
                    file: tmp,
                    path: file_path,
                    dirty: false,
                });
            }
        }

        reply.opened(fh, 0);
    }

    /// Legge un file. Se c'è un buffer attivo (file aperto in scrittura),
    /// legge dal buffer locale; altrimenti scarica dal server.
    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        // Se c'è un buffer di scrittura aperto, leggi da lì
        if let Some(buf) = self.write_buffers.get_mut(&fh) {
            if buf.file.seek(SeekFrom::Start(offset as u64)).is_err() {
                reply.error(libc::EIO);
                return;
            }
            let mut data = vec![0u8; size as usize];
            match buf.file.read(&mut data) {
                Ok(n) => reply.data(&data[..n]),
                Err(_) => reply.error(libc::EIO),
            }
            return;
        }

        // Altrimenti, scarica dal server
        let i2p = self.inode_to_path.lock().unwrap();
        let path = i2p.get(&ino).cloned();
        drop(i2p);

        if let Some(file_path) = path {
            match self.read_file(&file_path) {
                Ok(data) => {
                    let start = offset as usize;
                    if start >= data.len() {
                        reply.data(&[]);
                    } else {
                        let end = std::cmp::min(start + size as usize, data.len());
                        reply.data(&data[start..end]);
                    }
                }
                Err(_) => reply.error(libc::ENOENT),
            }
        } else {
            reply.error(libc::ENOENT);
        }
    }

    /// Crea un nuovo file. Crea il file vuoto sul server e prepara
    /// un buffer locale per le successive write.
    fn create(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: fuser::ReplyCreate,
    ) {
        let i2p = self.inode_to_path.lock().unwrap();
        let parent_path = i2p.get(&parent).cloned().unwrap_or_default();
        drop(i2p);

        let full_path = if parent_path.is_empty() {
            name.to_string_lossy().to_string()
        } else {
            format!("{}/{}", parent_path, name.to_string_lossy())
        };

        // Crea file vuoto sul server
        let url = format!("{}/files/{}", self.base_url, full_path);
        match self.client.put(&url).body("").send() {
            Ok(resp) if resp.status().is_success() => {
                let ino = self.alloc_inode(full_path.clone());

                // Prepara buffer locale per le write successive
                self.fh_counter += 1;
                let fh = self.fh_counter;
                let tmp = tempfile::tempfile().unwrap();
                self.write_buffers.insert(fh, WriteBuffer {
                    file: tmp,
                    path: full_path,
                    dirty: false,
                });

                let ttl = Duration::from_secs(1);
                let attr = FileAttr {
                    ino,
                    size: 0,
                    blocks: 0,
                    atime: SystemTime::now(),
                    mtime: SystemTime::now(),
                    ctime: SystemTime::now(),
                    crtime: SystemTime::now(),
                    kind: FileType::RegularFile,
                    perm: 0o644,
                    nlink: 1,
                    uid: 1000,
                    gid: 1000,
                    rdev: 0,
                    blksize: 512,
                    flags: 0,
                };
                reply.created(&ttl, &attr, 0, fh, 0);
            }
            _ => reply.error(libc::EIO),
        }
    }

    /// Scrive dati nel buffer locale alla posizione (offset) indicata.
    /// I dati verranno caricati sul server al flush.
    fn write(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: fuser::ReplyWrite,
    ) {
        if let Some(buf) = self.write_buffers.get_mut(&fh) {
            if buf.file.seek(SeekFrom::Start(offset as u64)).is_err() {
                reply.error(libc::EIO);
                return;
            }
            match buf.file.write_all(data) {
                Ok(_) => {
                    buf.dirty = true;
                    reply.written(data.len() as u32);
                }
                Err(_) => reply.error(libc::EIO),
            }
        } else {
            reply.error(libc::EBADF);
        }
    }

    /// Flush: legge tutto il buffer locale e lo carica sul server con PUT.
    fn flush(&mut self, _req: &Request<'_>, _ino: u64, fh: u64, _lock_owner: u64, reply: fuser::ReplyEmpty) {
        if let Some(buf) = self.write_buffers.get_mut(&fh) {
            if buf.dirty {
                let _ = buf.file.seek(SeekFrom::Start(0));
                let mut data = Vec::new();
                if buf.file.read_to_end(&mut data).is_err() {
                    reply.error(libc::EIO);
                    return;
                }

                let url = format!("{}/files/{}", self.base_url, buf.path);
                match self.client.put(&url).body(data).send() {
                    Ok(resp) if resp.status().is_success() => {
                        buf.dirty = false;
                        reply.ok();
                    }
                    _ => reply.error(libc::EIO),
                }
            } else {
                reply.ok();
            }
        } else {
            reply.ok();
        }
    }

    /// Release: chiude il file e rimuove il buffer locale.
    /// Il file temporaneo viene eliminato automaticamente.
    fn release(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        self.write_buffers.remove(&fh);
        reply.ok();
    }

    fn mkdir(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let i2p = self.inode_to_path.lock().unwrap();
        let parent_path = i2p.get(&parent).cloned().unwrap_or_default();
        drop(i2p);

        let full_path = if parent_path.is_empty() {
            name.to_string_lossy().to_string()
        } else {
            format!("{}/{}", parent_path, name.to_string_lossy())
        };

        let url = format!("{}/mkdir/{}", self.base_url, full_path);
        match self.client.post(&url).send() {
            Ok(resp) if resp.status().is_success() => {
                let ino = self.alloc_inode(full_path);
                let ttl = Duration::from_secs(1);
                let attr = FileAttr {
                    ino,
                    size: 0,
                    blocks: 0,
                    atime: SystemTime::now(),
                    mtime: SystemTime::now(),
                    ctime: SystemTime::now(),
                    crtime: SystemTime::now(),
                    kind: FileType::Directory,
                    perm: 0o755,
                    nlink: 2,
                    uid: 1000,
                    gid: 1000,
                    rdev: 0,
                    blksize: 512,
                    flags: 0,
                };
                reply.entry(&ttl, &attr, 0);
            }
            _ => reply.error(libc::EIO),
        }
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        let i2p = self.inode_to_path.lock().unwrap();
        let parent_path = i2p.get(&parent).cloned().unwrap_or_default();
        drop(i2p);

        let full_path = if parent_path.is_empty() {
            name.to_string_lossy().to_string()
        } else {
            format!("{}/{}", parent_path, name.to_string_lossy())
        };

        let url = format!("{}/files/{}", self.base_url, full_path);
        match self.client.delete(&url).send() {
            Ok(resp) if resp.status().is_success() => {
                // Remove from our cache
                let mut p2i = self.path_to_inode.lock().unwrap();
                if let Some(ino) = p2i.remove(&full_path) {
                    self.inode_to_path.lock().unwrap().remove(&ino);
                }
                reply.ok();
            }
            _ => reply.error(libc::EIO),
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        // Same as unlink for our simple implementation
        self.unlink(_req, parent, name, reply);
    }

    fn rename(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: fuser::ReplyEmpty,
    ) {
        let i2p = self.inode_to_path.lock().unwrap();
        let parent_path = i2p.get(&parent).cloned().unwrap_or_default();
        let newparent_path = i2p.get(&newparent).cloned().unwrap_or_default();
        drop(i2p);

        let old_path = if parent_path.is_empty() {
            name.to_string_lossy().to_string()
        } else {
            format!("{}/{}", parent_path, name.to_string_lossy())
        };

        let new_path = if newparent_path.is_empty() {
            newname.to_string_lossy().to_string()
        } else {
            format!("{}/{}", newparent_path, newname.to_string_lossy())
        };

        // Simple implementation: read old file, write new file, delete old
        match self.read_file(&old_path) {
            Ok(data) => {
                let write_url = format!("{}/files/{}", self.base_url, new_path);
                let delete_url = format!("{}/files/{}", self.base_url, old_path);

                if let Ok(resp) = self.client.put(&write_url).body(data).send() {
                    if resp.status().is_success() {
                        if let Ok(resp) = self.client.delete(&delete_url).send() {
                            if resp.status().is_success() {
                                // Update our cache
                                let mut p2i = self.path_to_inode.lock().unwrap();
                                if let Some(ino) = p2i.remove(&old_path) {
                                    p2i.insert(new_path.clone(), ino);
                                    self.inode_to_path.lock().unwrap().insert(ino, new_path);
                                }
                                reply.ok();
                                return;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        reply.error(libc::EIO);
    }

    fn setattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>,
        _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        // Handle file truncation
        if let Some(new_size) = size {
            let i2p = self.inode_to_path.lock().unwrap();
            if let Some(path) = i2p.get(&ino).cloned() {
                drop(i2p);

                if new_size == 0 {
                    // Truncate to zero
                    let url = format!("{}/files/{}", self.base_url, path);
                    if let Ok(resp) = self.client.put(&url).body("").send() {
                        if resp.status().is_success() {
                            let ttl = Duration::from_secs(1);
                            let attr = FileAttr {
                                ino,
                                size: 0,
                                blocks: 0,
                                atime: SystemTime::now(),
                                mtime: SystemTime::now(),
                                ctime: SystemTime::now(),
                                crtime: SystemTime::now(),
                                kind: FileType::RegularFile,
                                perm: 0o644,
                                nlink: 1,
                                uid: 1000,
                                gid: 1000,
                                rdev: 0,
                                blksize: 512,
                                flags: 0,
                            };
                            reply.attr(&ttl, &attr);
                            return;
                        }
                    }
                }
            }
        }

        // For other attributes, just return current attributes
        self.getattr(_req, ino, reply);
    }
}

pub fn run_linux_macos(mountpoint: &str) {
    println!("Mounting at: {}", mountpoint);
    println!("Press Ctrl-C to unmount and exit.");

    let fs = RemoteFS::new("http://127.0.0.1:8000");

    let options = vec![
        MountOption::FSName("remote-fs".to_string()),
        MountOption::Subtype("remote-fs".to_string()),
        MountOption::DefaultPermissions,
        MountOption::AllowOther,
        MountOption::AutoUnmount
    ];


    match fuser::mount2(fs, mountpoint, &options) {
        Ok(()) => {
            println!("File system mounted successfully at {}", mountpoint);
        }
        Err(e) => {
            eprintln!("Failed to mount file system: {}", e);
            eprintln!("Make sure:");
            eprintln!("1. macFUSE is properly installed and enabled");
            eprintln!("2. The mount point exists and is empty");
            eprintln!("3. You have the necessary permissions");
            std::process::exit(1);
        }
    }
}
