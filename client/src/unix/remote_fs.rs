use crate::remote_client::{ProgressReader, RemoteClient};
use crate::types::{join_path, parent_of, CacheConfig};
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request,
};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom, Write as IoWrite};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// Filters Finder metadata files that should not be mirrored remotely.
fn is_macos_metadata(name: &OsStr) -> bool {
    let s = name.to_string_lossy();
    s.starts_with("._") || s == ".DS_Store" || s == ".localized"
}

/// Buffered write state associated with an open file handle.
struct WriteBuffer {
    file: std::fs::File,
    path: String,
    dirty: bool,
}

/// Builds FUSE attributes from remote metadata.
fn make_attr(ino: u64, size: u64, kind: FileType) -> FileAttr {
    let now = SystemTime::now();
    FileAttr {
        ino,
        size,
        blocks: (size + 511) / 512,
        atime: now,
        mtime: now,
        ctime: now,
        crtime: now,
        kind,
        perm: if kind == FileType::Directory {
            0o755
        } else {
            0o644
        },
        nlink: if kind == FileType::Directory { 2 } else { 1 },
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
        rdev: 0,
        blksize: 512,
        flags: 0,
    }
}

/// FUSE implementation that maps local VFS operations to the remote HTTP API.
pub struct RemoteFS {
    rc: RemoteClient,
    inode_counter: u64,
    inode_to_path: Arc<Mutex<HashMap<u64, String>>>,
    path_to_inode: Arc<Mutex<HashMap<String, u64>>>,
    write_buffers: HashMap<u64, WriteBuffer>,
    fh_counter: u64,
}

impl RemoteFS {
    pub fn new(base_url: &str, cache_config: CacheConfig) -> Self {
        let mut inode_to_path = HashMap::new();
        let mut path_to_inode = HashMap::new();
        inode_to_path.insert(1, String::new());
        path_to_inode.insert(String::new(), 1);

        Self {
            rc: RemoteClient::new(base_url, cache_config),
            inode_counter: 1,
            inode_to_path: Arc::new(Mutex::new(inode_to_path)),
            path_to_inode: Arc::new(Mutex::new(path_to_inode)),
            write_buffers: HashMap::new(),
            fh_counter: 0,
        }
    }

    fn inode_path(&self, ino: u64) -> Option<String> {
        self.inode_to_path.lock().unwrap().get(&ino).cloned()
    }

    fn child_path(&self, parent: u64, name: &OsStr) -> (String, String) {
        let parent_path = self.inode_path(parent).unwrap_or_default();
        let full = join_path(&parent_path, &name.to_string_lossy());
        (parent_path, full)
    }

    fn alloc_inode(&mut self, path: String) -> u64 {
        let mut p2i = self.path_to_inode.lock().unwrap();
        if let Some(&ino) = p2i.get(&path) {
            return ino;
        }
        self.inode_counter += 1;
        let ino = self.inode_counter;
        p2i.insert(path.clone(), ino);
        drop(p2i);
        self.inode_to_path.lock().unwrap().insert(ino, path);
        ino
    }

    fn remove_inode(&mut self, path: &str) {
        let mut p2i = self.path_to_inode.lock().unwrap();
        if let Some(ino) = p2i.remove(path) {
            drop(p2i);
            self.inode_to_path.lock().unwrap().remove(&ino);
        }
    }

    fn next_fh(&mut self) -> u64 {
        self.fh_counter += 1;
        self.fh_counter
    }
    fn ttl(&self) -> Duration {
        self.rc.cache_config.dir_ttl.max(Duration::from_millis(100))
    }
}

impl Filesystem for RemoteFS {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if is_macos_metadata(name) {
            reply.error(libc::ENOENT);
            return;
        }
        let (parent_path, full_path) = self.child_path(parent, name);
        let name_str = name.to_string_lossy();

        if let Ok(entries) = self.rc.list_dir(&parent_path) {
            if let Some(entry) = entries.iter().find(|e| e.name == *name_str) {
                let ino = self.alloc_inode(full_path);
                let kind = if entry.is_dir {
                    FileType::Directory
                } else {
                    FileType::RegularFile
                };
                reply.entry(&self.ttl(), &make_attr(ino, entry.size, kind), 0);
                return;
            }
        }
        reply.error(libc::ENOENT);
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        if ino == 1 {
            reply.attr(&self.ttl(), &make_attr(1, 0, FileType::Directory));
            return;
        }

        if let Some(path) = self.inode_path(ino) {
            let parent = parent_of(&path);
            let filename = path.split('/').last().unwrap_or("");

            if let Ok(entries) = self.rc.list_dir(&parent) {
                if let Some(entry) = entries.iter().find(|e| e.name == filename) {
                    let kind = if entry.is_dir {
                        FileType::Directory
                    } else {
                        FileType::RegularFile
                    };
                    reply.attr(&self.ttl(), &make_attr(ino, entry.size, kind));
                    return;
                }
            }
        }
        reply.error(libc::ENOENT);
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let parent_path = self.inode_path(ino).unwrap_or_default();

        if offset == 0 {
            let _ = reply.add(ino, 1, FileType::Directory, ".");
            let _ = reply.add(ino, 2, FileType::Directory, "..");

            if let Ok(entries) = self.rc.list_dir(&parent_path) {
                for (i, entry) in entries.iter().enumerate() {
                    let child = join_path(&parent_path, &entry.name);
                    let child_ino = self.alloc_inode(child);
                    let kind = if entry.is_dir {
                        FileType::Directory
                    } else {
                        FileType::RegularFile
                    };
                    if reply.add(child_ino, (i + 3) as i64, kind, &entry.name) {
                        break;
                    }
                }
            }
        }
        reply.ok();
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: fuser::ReplyOpen) {
        let fh = self.next_fh();
        let access = flags & libc::O_ACCMODE;
        let writable = access == libc::O_WRONLY || access == libc::O_RDWR;
        let truncate = (flags & libc::O_TRUNC) != 0;

        if writable || truncate {
            if let Some(path) = self.inode_path(ino) {
                let mut tmp = tempfile::tempfile().unwrap();
                if !truncate {
                    if let Ok(data) = self.rc.fetch_file(&path) {
                        let _ = tmp.write_all(&data);
                        let _ = tmp.seek(SeekFrom::Start(0));
                    }
                }
                self.write_buffers.insert(
                    fh,
                    WriteBuffer {
                        file: tmp,
                        path,
                        dirty: false,
                    },
                );
            }
            reply.opened(fh, 1);
            return;
        } else if self.rc.cache_config.file_ttl.is_zero() {
            if let Some(path) = self.inode_path(ino) {
                let mut tmp = tempfile::tempfile().unwrap();
                if let Ok(data) = self.rc.fetch_file(&path) {
                    let _ = tmp.write_all(&data);
                    let _ = tmp.seek(SeekFrom::Start(0));
                }
                self.write_buffers.insert(
                    fh,
                    WriteBuffer {
                        file: tmp,
                        path,
                        dirty: false,
                    },
                );
            }
        }
        reply.opened(fh, 0);
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
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

        let path = match self.inode_path(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        if let Some(cached) = self.rc.cached_file_data(&path) {
            let start = offset as usize;
            let end = std::cmp::min(start + size as usize, cached.len());
            reply.data(if start >= cached.len() {
                &[]
            } else {
                &cached[start..end]
            });
            return;
        }

        match self.rc.fetch_range(&path, offset as u64, size) {
            Ok(data) => reply.data(&data),
            Err(_) => reply.error(libc::ENOENT),
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
        if is_macos_metadata(name) {
            reply.error(libc::EPERM);
            return;
        }
        let (_, full_path) = self.child_path(parent, name);

        match self.rc.upload(&full_path, Vec::new()) {
            Ok(_) => {
                self.rc.invalidate(&full_path);
                let ino = self.alloc_inode(full_path.clone());
                let fh = self.next_fh();
                let tmp = tempfile::tempfile().unwrap();
                self.write_buffers.insert(
                    fh,
                    WriteBuffer {
                        file: tmp,
                        path: full_path,
                        dirty: false,
                    },
                );
                reply.created(
                    &self.ttl(),
                    &make_attr(ino, 0, FileType::RegularFile),
                    0,
                    fh,
                    0,
                );
            }
            Err(_) => {
                reply.error(libc::EIO);
            }
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _wf: u32,
        _flags: i32,
        _lock: Option<u64>,
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

    fn flush(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _lock: u64,
        reply: fuser::ReplyEmpty,
    ) {
        let upload_info = if let Some(buf) = self.write_buffers.get_mut(&fh) {
            if !buf.dirty {
                reply.ok();
                return;
            }
            if buf.file.seek(SeekFrom::Start(0)).is_err() {
                reply.error(libc::EIO);
                return;
            }
            let size = buf.file.metadata().map(|m| m.len()).unwrap_or(0);
            match buf.file.try_clone() {
                Ok(file) => {
                    buf.dirty = false;
                    Some((buf.path.clone(), file, size))
                }
                Err(_) => {
                    reply.error(libc::EIO);
                    return;
                }
            }
        } else {
            reply.ok();
            return;
        };

        if let Some((path, file, size)) = upload_info {
            let name = path.split('/').last().unwrap_or(&path).to_string();
            let reader = ProgressReader {
                inner: file,
                total: size,
                sent: 0,
                name: name.clone(),
                last_pct: u64::MAX,
            };
            match self.rc.upload_streamed(&path, reader, size) {
                Ok(_) => {
                    self.rc.invalidate(&path);
                    reply.ok();
                }
                Err(_) => {
                    reply.error(libc::EIO);
                }
            }
        }
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock: Option<u64>,
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
        if is_macos_metadata(name) {
            reply.error(libc::EPERM);
            return;
        }
        let (_, full_path) = self.child_path(parent, name);

        match self.rc.mkdir_remote(&full_path) {
            Ok(_) => {
                self.rc.invalidate(&full_path);
                let ino = self.alloc_inode(full_path);
                reply.entry(&self.ttl(), &make_attr(ino, 0, FileType::Directory), 0);
            }
            Err(_) => reply.error(libc::EIO),
        }
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        let (_, full_path) = self.child_path(parent, name);

        match self.rc.delete_remote(&full_path) {
            Ok(_) => {
                self.rc.invalidate(&full_path);
                self.remove_inode(&full_path);
                reply.ok();
            }
            Err(_) => reply.error(libc::EIO),
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
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
        let (_, old_path) = self.child_path(parent, name);
        let (_, new_path) = self.child_path(newparent, newname);

        if old_path.is_empty() || new_path.is_empty() {
            reply.ok();
            return;
        }

        self.rc.invalidate(&old_path);
        self.rc.invalidate(&new_path);

        let parent_path = parent_of(&old_path);
        let entry_name = old_path.split('/').last().unwrap_or("");
        let is_dir = self
            .rc
            .list_dir(&parent_path)
            .ok()
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|e| e.name == entry_name)
                    .map(|e| e.is_dir)
            })
            .unwrap_or(false);

        if is_dir {
            if self.rc.rename_dir_recursive(&old_path, &new_path).is_err() {
                reply.error(libc::EIO);
                return;
            }
            if self.rc.delete_remote(&old_path).is_err() {
                reply.error(libc::EIO);
                return;
            }
            let prefix = format!("{}/", old_path);
            let new_prefix = format!("{}/", new_path);
            let mut p2i = self.path_to_inode.lock().unwrap();
            let to_remap: Vec<(String, u64)> = p2i
                .iter()
                .filter(|(p, _)| *p == &old_path || p.starts_with(&prefix))
                .map(|(p, &ino)| (p.clone(), ino))
                .collect();
            let mut new_entries: Vec<(String, u64)> = Vec::new();
            for (old, _) in &to_remap {
                p2i.remove(old);
            }
            for (old, ino) in &to_remap {
                let new = if old == &old_path {
                    new_path.clone()
                } else {
                    format!("{}{}", new_prefix, &old[prefix.len()..])
                };
                p2i.insert(new.clone(), *ino);
                new_entries.push((new, *ino));
            }
            drop(p2i);
            let mut i2p = self.inode_to_path.lock().unwrap();
            for (new, ino) in new_entries {
                i2p.insert(ino, new);
            }
            drop(i2p);
            self.rc.invalidate(&old_path);
            self.rc.invalidate(&new_path);
            reply.ok();
            return;
        }

        let data = match self.rc.fetch_file(&old_path) {
            Ok(d) => d,
            Err(_) => {
                reply.error(libc::EIO);
                return;
            }
        };

        if let Err(_) = self.rc.upload(&new_path, data) {
            reply.error(libc::EIO);
            return;
        }
        if let Err(_) = self.rc.delete_remote(&old_path) {
            reply.error(libc::EIO);
            return;
        }

        let mut p2i = self.path_to_inode.lock().unwrap();
        if let Some(ino) = p2i.remove(&old_path) {
            p2i.insert(new_path.clone(), ino);
            drop(p2i);
            self.inode_to_path.lock().unwrap().insert(ino, new_path);
        }
        reply.ok();
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
        if let Some(new_size) = size {
            let path = self.inode_path(ino);
            let mut buf_found = false;
            if let Some(ref p) = path {
                for buf in self.write_buffers.values_mut() {
                    if &buf.path == p {
                        let _ = buf.file.set_len(new_size);
                        let _ = buf.file.seek(SeekFrom::End(0));
                        buf.dirty = true;
                        buf_found = true;
                    }
                }
            }
            if buf_found {
                reply.attr(
                    &self.ttl(),
                    &make_attr(ino, new_size, FileType::RegularFile),
                );
                return;
            }
            if new_size == 0 {
                if let Some(p) = path {
                    if self.rc.upload(&p, Vec::new()).is_ok() {
                        self.rc.invalidate(&p);
                        reply.attr(&self.ttl(), &make_attr(ino, 0, FileType::RegularFile));
                        return;
                    }
                }
            }
        }
        self.getattr(_req, ino, None, reply);
    }
}
