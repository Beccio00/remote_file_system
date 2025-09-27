use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request,
};
use libc::ENOENT;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

#[derive(Debug, Deserialize)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

pub struct RemoteFS {
    client: Client,
    base_url: String,
    inode_counter: u64,
    inode_to_path: Arc<Mutex<HashMap<u64, String>>>,
    path_to_inode: Arc<Mutex<HashMap<String, u64>>>,
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
            // Try to get file info from parent directory listing
            if let Some(parent_path) = file_path.rsplit('/').nth(1) {
                let parent_path = if parent_path.is_empty() {
                    ""
                } else {
                    parent_path
                };
                if let Ok(entries) = self.list_dir(parent_path) {
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

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let i2p = self.inode_to_path.lock().unwrap();
        if let Some(path) = i2p.get(&ino) {
            match self.read_file(path) {
                Ok(data) => {
                    let end = std::cmp::min((offset as usize) + (size as usize), data.len());
                    let slice = &data[(offset as usize)..end];
                    reply.data(slice);
                }
                Err(_) => reply.error(libc::ENOENT),
            }
        } else {
            reply.error(libc::ENOENT);
        }
    }

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

        // Create empty file on server
        let url = format!("{}/files/{}", self.base_url, full_path);
        match self.client.put(&url).body("").send() {
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
                    kind: FileType::RegularFile,
                    perm: 0o644,
                    nlink: 1,
                    uid: 1000,
                    gid: 1000,
                    rdev: 0,
                    blksize: 512,
                    flags: 0,
                };
                reply.created(&ttl, &attr, 0, ino, 0);
            }
            _ => reply.error(libc::EIO),
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: fuser::ReplyWrite,
    ) {
        let i2p = self.inode_to_path.lock().unwrap();
        let path = i2p.get(&ino).cloned();
        drop(i2p);

        if let Some(file_path) = path {
            // For simplicity, we'll do full file replacement for now
            // In a real implementation, you'd want to handle partial writes
            if offset == 0 {
                let url = format!("{}/files/{}", self.base_url, file_path);
                match self.client.put(&url).body(data.to_vec()).send() {
                    Ok(resp) if resp.status().is_success() => {
                        reply.written(data.len() as u32);
                    }
                    _ => reply.error(libc::EIO),
                }
            } else {
                // For offset writes, we'd need to read, modify, write
                // This is a simplified implementation
                reply.error(libc::ENOSYS);
            }
        } else {
            reply.error(libc::ENOENT);
        }
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
