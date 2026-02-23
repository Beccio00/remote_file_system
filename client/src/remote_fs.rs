use crate::types::{CacheConfig, RemoteEntry, join_path, parent_of};
use fuser::{FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request};
use reqwest::blocking::Client;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom, Write as IoWrite};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

const TTL: Duration = Duration::from_secs(1);

struct CachedDir {
    entries: Vec<RemoteEntry>,
    cached_at: Instant,
}

struct CachedFile {
    data: Vec<u8>,
    cached_at: Instant,
}

struct WriteBuffer {
    file: std::fs::File,
    path: String,
    dirty: bool,
}

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
        perm: if kind == FileType::Directory { 0o755 } else { 0o644 },
        nlink: if kind == FileType::Directory { 2 } else { 1 },
        uid: 1000,
        gid: 1000,
        rdev: 0,
        blksize: 512,
        flags: 0,
    }
}

pub struct RemoteFS {
    client: Client,
    base_url: String,
    inode_counter: u64,
    inode_to_path: Arc<Mutex<HashMap<u64, String>>>,
    path_to_inode: Arc<Mutex<HashMap<String, u64>>>,
    write_buffers: HashMap<u64, WriteBuffer>,
    fh_counter: u64,
    cache_config: CacheConfig,
    dir_cache: HashMap<String, CachedDir>,
    file_cache: HashMap<String, CachedFile>,
    file_cache_size: usize,
}

impl RemoteFS {
    pub fn new(base_url: &str, cache_config: CacheConfig) -> Self {
        let mut inode_to_path = HashMap::new();
        let mut path_to_inode = HashMap::new();
        inode_to_path.insert(1, String::new());
        path_to_inode.insert(String::new(), 1);

        Self {
            client: Client::builder()
                .timeout(None)
                .build()
                .expect("failed to build HTTP client"),
            base_url: base_url.to_string(),
            inode_counter: 1,
            inode_to_path: Arc::new(Mutex::new(inode_to_path)),
            path_to_inode: Arc::new(Mutex::new(path_to_inode)),
            write_buffers: HashMap::new(),
            fh_counter: 0,
            cache_config,
            dir_cache: HashMap::new(),
            file_cache: HashMap::new(),
            file_cache_size: 0,
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

    fn list_dir(&mut self, path: &str) -> Result<Vec<RemoteEntry>, anyhow::Error> {
        if let Some(cached) = self.dir_cache.get(path) {
            if cached.cached_at.elapsed() < self.cache_config.dir_ttl {
                return Ok(cached.entries.clone());
            }
        }

        let url = format!("{}/list/{}", self.base_url, path);
        let entries: Vec<RemoteEntry> = self.client.get(&url).send()?.error_for_status()?.json()?;

        self.dir_cache.insert(path.to_string(), CachedDir {
            entries: entries.clone(),
            cached_at: Instant::now(),
        });
        Ok(entries)
    }

    fn fetch_file(&mut self, path: &str) -> Result<Vec<u8>, anyhow::Error> {
        if let Some(cached) = self.file_cache.get(path) {
            if cached.cached_at.elapsed() < self.cache_config.file_ttl {
                return Ok(cached.data.clone());
            }
        }

        let url = format!("{}/files/{}", self.base_url, path);
        let data = self.client.get(&url).send()?.error_for_status()?.bytes()?.to_vec();

        // Evict oldest entries if over budget
        while self.file_cache_size + data.len() > self.cache_config.max_file_cache_bytes {
            let oldest = self.file_cache.iter()
                .min_by_key(|(_, v)| v.cached_at)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(key) => {
                    if let Some(evicted) = self.file_cache.remove(&key) {
                        self.file_cache_size -= evicted.data.len();
                    }
                }
                None => break,
            }
        }

        self.file_cache_size += data.len();
        self.file_cache.insert(path.to_string(), CachedFile {
            data: data.clone(),
            cached_at: Instant::now(),
        });
        Ok(data)
    }

    fn upload(&self, path: &str, data: Vec<u8>) -> Result<(), anyhow::Error> {
        let url = format!("{}/files/{}", self.base_url, path);
        self.client.put(&url).body(data).send()?.error_for_status()?;
        Ok(())
    }

    fn delete_remote(&self, path: &str) -> Result<(), anyhow::Error> {
        let url = format!("{}/files/{}", self.base_url, path);
        self.client.delete(&url).send()?.error_for_status()?;
        Ok(())
    }

    fn mkdir_remote(&self, path: &str) -> Result<(), anyhow::Error> {
        let url = format!("{}/mkdir/{}", self.base_url, path);
        self.client.post(&url).send()?.error_for_status()?;
        Ok(())
    }

    fn invalidate(&mut self, path: &str) {
        self.dir_cache.remove(&parent_of(path));
        self.dir_cache.remove(path);
        if let Some(evicted) = self.file_cache.remove(path) {
            self.file_cache_size -= evicted.data.len();
        }
    }

    fn fetch_range(&self, path: &str, offset: u64, size: u32) -> Result<Vec<u8>, anyhow::Error> {
        let url = format!("{}/files/{}", self.base_url, path);
        let end = offset + (size as u64) - 1;
        let range_header = format!("bytes={}-{}", offset, end);
        let resp = self.client.get(&url)
            .header("Range", range_header)
            .send()?
            .error_for_status()?;
        Ok(resp.bytes()?.to_vec())
    }
}


impl Filesystem for RemoteFS {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let (parent_path, full_path) = self.child_path(parent, name);
        let name_str = name.to_string_lossy();

        if let Ok(entries) = self.list_dir(&parent_path) {
            if let Some(entry) = entries.iter().find(|e| e.name == *name_str) {
                let ino = self.alloc_inode(full_path);
                let kind = if entry.is_dir { FileType::Directory } else { FileType::RegularFile };
                reply.entry(&TTL, &make_attr(ino, entry.size, kind), 0);
                return;
            }
        }
        reply.error(libc::ENOENT);
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        if ino == 1 {
            reply.attr(&TTL, &make_attr(1, 0, FileType::Directory));
            return;
        }

        if let Some(path) = self.inode_path(ino) {
            let parent = parent_of(&path);
            let filename = path.split('/').last().unwrap_or("");

            if let Ok(entries) = self.list_dir(&parent) {
                if let Some(entry) = entries.iter().find(|e| e.name == filename) {
                    let kind = if entry.is_dir { FileType::Directory } else { FileType::RegularFile };
                    reply.attr(&TTL, &make_attr(ino, entry.size, kind));
                    return;
                }
            }
        }
        reply.error(libc::ENOENT);
    }

    fn readdir(
        &mut self, _req: &Request<'_>, ino: u64, _fh: u64,
        offset: i64, mut reply: ReplyDirectory,
    ) {
        let parent_path = self.inode_path(ino).unwrap_or_default();

        if offset == 0 {
            let _ = reply.add(ino, 1, FileType::Directory, ".");
            let _ = reply.add(ino, 2, FileType::Directory, "..");

            if let Ok(entries) = self.list_dir(&parent_path) {
                for (i, entry) in entries.iter().enumerate() {
                    let child = join_path(&parent_path, &entry.name);
                    let child_ino = self.alloc_inode(child);
                    let kind = if entry.is_dir { FileType::Directory } else { FileType::RegularFile };
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
                    if let Ok(data) = self.fetch_file(&path) {
                        let _ = tmp.write_all(&data);
                        let _ = tmp.seek(SeekFrom::Start(0));
                    }
                }
                self.write_buffers.insert(fh, WriteBuffer { file: tmp, path, dirty: false });
            }
        }
        reply.opened(fh, 0);
    }

    fn read(
        &mut self, _req: &Request<'_>, ino: u64, fh: u64,
        offset: i64, size: u32, _flags: i32, _lock: Option<u64>, reply: ReplyData,
    ) {
        // Prefer the local write buffer if the file is open for writing
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
            None => { reply.error(libc::ENOENT); return; }
        };

        // If the file is in cache, serve from there
        if let Some(cached) = self.file_cache.get(&path) {
            if cached.cached_at.elapsed() < self.cache_config.file_ttl {
                let start = offset as usize;
                let end = std::cmp::min(start + size as usize, cached.data.len());
                reply.data(if start >= cached.data.len() { &[] } else { &cached.data[start..end] });
                return;
            }
        }

        // No cache hit → use Range request (only fetch the bytes we need)
        match self.fetch_range(&path, offset as u64, size) {
            Ok(data) => reply.data(&data),
            Err(_) => reply.error(libc::ENOENT),
        }
    }

    fn create(
        &mut self, _req: &Request<'_>, parent: u64, name: &OsStr,
        _mode: u32, _umask: u32, _flags: i32, reply: fuser::ReplyCreate,
    ) {
        let (_, full_path) = self.child_path(parent, name);

        eprintln!("[create] path={}", full_path);
        match self.upload(&full_path, Vec::new()) {
            Ok(_) => {
                self.invalidate(&full_path);
                let ino = self.alloc_inode(full_path.clone());
                let fh = self.next_fh();
                let tmp = tempfile::tempfile().unwrap();
                self.write_buffers.insert(fh, WriteBuffer { file: tmp, path: full_path, dirty: false });
                eprintln!("[create] ok fh={}", fh);
                reply.created(&TTL, &make_attr(ino, 0, FileType::RegularFile), 0, fh, 0);
            }
            Err(e) => { eprintln!("[create] FAILED: {}", e); reply.error(libc::EIO); }
        }
    }

    fn write(
        &mut self, _req: &Request<'_>, _ino: u64, fh: u64,
        offset: i64, data: &[u8], _wf: u32, _flags: i32, _lock: Option<u64>,
        reply: fuser::ReplyWrite,
    ) {
        if let Some(buf) = self.write_buffers.get_mut(&fh) {
            if buf.file.seek(SeekFrom::Start(offset as u64)).is_err() {
                eprintln!("[write] seek failed fh={} offset={}", fh, offset);
                reply.error(libc::EIO);
                return;
            }
            match buf.file.write_all(data) {
                Ok(_) => {
                    buf.dirty = true;
                    // Log progress every 10MB
                    if offset % (10 * 1024 * 1024) == 0 {
                        eprintln!("[write] fh={} offset={}MB len={}", fh, offset / 1024 / 1024, data.len());
                    }
                    reply.written(data.len() as u32);
                }
                Err(e) => { eprintln!("[write] write_all failed: {}", e); reply.error(libc::EIO); }
            }
        } else {
            eprintln!("[write] no buffer for fh={}", fh);
            reply.error(libc::EBADF);
        }
    }

    fn flush(
        &mut self, _req: &Request<'_>, _ino: u64, fh: u64,
        _lock: u64, reply: fuser::ReplyEmpty,
    ) {
        eprintln!("[flush] called fh={}", fh);
        let upload_data = if let Some(buf) = self.write_buffers.get_mut(&fh) {
            if !buf.dirty {
                eprintln!("[flush] fh={} not dirty, skipping", fh);
                reply.ok();
                return;
            }
            let _ = buf.file.seek(SeekFrom::Start(0));
            let mut data = Vec::new();
            match buf.file.read_to_end(&mut data) {
                Ok(_) => eprintln!("[flush] fh={} read {} bytes from tempfile", fh, data.len()),
                Err(e) => {
                    eprintln!("[flush] fh={} read_to_end FAILED: {}", fh, e);
                    reply.error(libc::EIO);
                    return;
                }
            }
            buf.dirty = false;
            Some((buf.path.clone(), data))
        } else {
            None
        };

        match upload_data {
            Some((path, ref data)) => {
                eprintln!("[flush] uploading path={} size={}MB", path, data.len() / 1024 / 1024);
                match self.upload(&path, data.to_vec()) {
                    Ok(_) => {
                        eprintln!("[flush] upload OK path={}", path);
                        self.invalidate(&path);
                        reply.ok();
                    }
                    Err(e) => {
                        eprintln!("[flush] upload FAILED path={}: {}", path, e);
                        reply.error(libc::EIO);
                    }
                }
            }
            None => reply.ok(),
        }
    }

    fn release(
        &mut self, _req: &Request<'_>, _ino: u64, fh: u64,
        _flags: i32, _lock: Option<u64>, _flush: bool, reply: fuser::ReplyEmpty,
    ) {
        self.write_buffers.remove(&fh);
        reply.ok();
    }

    fn mkdir(
        &mut self, _req: &Request<'_>, parent: u64, name: &OsStr,
        _mode: u32, _umask: u32, reply: ReplyEntry,
    ) {
        let (_, full_path) = self.child_path(parent, name);

        match self.mkdir_remote(&full_path) {
            Ok(_) => {
                self.invalidate(&full_path);
                let ino = self.alloc_inode(full_path);
                reply.entry(&TTL, &make_attr(ino, 0, FileType::Directory), 0);
            }
            Err(_) => reply.error(libc::EIO),
        }
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        let (_, full_path) = self.child_path(parent, name);

        match self.delete_remote(&full_path) {
            Ok(_) => {
                self.invalidate(&full_path);
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
        &mut self, _req: &Request<'_>, parent: u64, name: &OsStr,
        newparent: u64, newname: &OsStr, _flags: u32, reply: fuser::ReplyEmpty,
    ) {
        let (_, old_path) = self.child_path(parent, name);
        let (_, new_path) = self.child_path(newparent, newname);

        self.invalidate(&old_path);
        self.invalidate(&new_path);

        let data = match self.fetch_file(&old_path) {
            Ok(d) => d,
            Err(_) => { reply.error(libc::EIO); return; }
        };

        if self.upload(&new_path, data).is_err() {
            reply.error(libc::EIO);
            return;
        }
        if self.delete_remote(&old_path).is_err() {
            reply.error(libc::EIO);
            return;
        }

        // Update inode mapping
        let mut p2i = self.path_to_inode.lock().unwrap();
        if let Some(ino) = p2i.remove(&old_path) {
            p2i.insert(new_path.clone(), ino);
            drop(p2i);
            self.inode_to_path.lock().unwrap().insert(ino, new_path);
        }
        reply.ok();
    }

    fn setattr(
        &mut self, _req: &Request<'_>, ino: u64,
        _mode: Option<u32>, _uid: Option<u32>, _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>, _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<SystemTime>, _fh: Option<u64>,
        _crtime: Option<SystemTime>, _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>, _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        // Handle truncation to zero
        if let Some(0) = size {
            if let Some(path) = self.inode_path(ino) {
                if self.upload(&path, Vec::new()).is_ok() {
                    self.invalidate(&path);
                    reply.attr(&TTL, &make_attr(ino, 0, FileType::RegularFile));
                    return;
                }
            }
        }
        // Fallback: return current attributes
        self.getattr(_req, ino, reply);
    }
}
