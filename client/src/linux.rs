use crate::common::RemoteFS;
use fuser::MountOption;
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
struct RemoteEntry {
    name: String,
    is_dir: bool,
    size: u64,
}

pub fn run(mountpoint: &str) {
    println!("Starting Remote File System on Linux...");
    println!("Mounting at: {}", mountpoint);

    let fs = RemoteFS::new("http://127.0.0.1:8000");

    let options = vec![
        MountOption::FSName("remote-fs".to_string()),
        MountOption::DefaultPermissions,
    ];

    match fuser::mount2(fs, mountpoint, &options) {
        Ok(()) => {
            println!("File system mounted successfully at {}", mountpoint);
        }
        Err(e) => {
            eprintln!("Failed to mount file system: {}", e);
            eprintln!("Make sure:");
            eprintln!("1. FUSE is installed (apt install fuse)");
            eprintln!("2. The mount point exists and is empty");
            eprintln!("3. You have the necessary permissions");
            std::process::exit(1);
        }
    }
}

struct RemoteFS {
    client: Client,
    base_url: String,
    inode_counter: u64,
    inode_to_path: Arc<Mutex<HashMap<u64, String>>>,
    path_to_inode: Arc<Mutex<HashMap<String, u64>>>,
}

impl RemoteFS {
    fn new(base_url: &str) -> Self {
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
        let p2i = self.path_to_inode.lock().unwrap();
        let i2p = self.inode_to_path.lock().unwrap();

        if let Some(parent_path) = i2p.get(&parent) {
            let full_path = if parent_path.is_empty() {
                name.to_string_lossy().to_string()
            } else {
                format!("{}/{}", parent_path, name.to_string_lossy())
            };

            if let Some(&ino) = p2i.get(&full_path) {
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
                reply.entry(&ttl, &attr, 0);
                return;
            }
        }

        reply.error(ENOENT);
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        let ttl = Duration::from_secs(1);
        let kind = if ino == 1 {
            FileType::Directory
        } else {
            FileType::RegularFile
        };

        let attr = FileAttr {
            ino,
            size: 0,
            blocks: 0,
            atime: SystemTime::now(),
            mtime: SystemTime::now(),
            ctime: SystemTime::now(),
            crtime: SystemTime::now(),
            kind,
            perm: 0o755,
            nlink: 2,
            uid: 1000,
            gid: 1000,
            rdev: 0,
            blksize: 512,
            flags: 0,
        };
        reply.attr(&ttl, &attr);
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
}
