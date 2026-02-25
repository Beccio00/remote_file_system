//! Windows implementation using WinFSP.

use crate::remote_client::RemoteClient;
use crate::types::{CacheConfig, RemoteEntry, parent_of};
use crate::Cli;

use std::ffi::c_void;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

use winfsp::filesystem::*;
use winfsp::host::*;
use winfsp::{U16CStr, U16CString};

// ── Windows constants ────────────────────────────────────────────
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const STATUS_OBJECT_NAME_NOT_FOUND: i32 = 0xC000_0034_u32 as i32;
const STATUS_UNSUCCESSFUL: i32 = 0xC000_0001_u32 as i32;
const STATUS_INVALID_DEVICE_REQUEST: i32 = 0xC000_0010_u32 as i32;

fn nt(code: i32) -> winfsp::FspError {
    winfsp::FspError::NTSTATUS(code)
}

// ── Helpers ──────────────────────────────────────────────────────

/// Convert WinFSP wide path `\foo\bar` → internal `foo/bar`.
fn wide_to_path(name: &U16CStr) -> String {
    name.to_string_lossy()
        .trim_start_matches('\\')
        .replace('\\', "/")
}

fn filename_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Current time as Windows FILETIME (100-ns intervals since 1601-01-01).
fn filetime_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    let dur = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    EPOCH_DIFF + (dur.as_nanos() / 100) as u64
}

fn make_file_info(is_dir: bool, size: u64) -> FileInfo {
    let now = filetime_now();
    FileInfo {
        file_attributes: if is_dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL },
        file_size: size,
        allocation_size: (size + 4095) & !4095,
        creation_time: now,
        last_access_time: now,
        last_write_time: now,
        change_time: now,
        ..Default::default()
    }
}

/// Minimal self-relative security descriptor granting Everyone full access.
fn default_security_descriptor() -> Vec<u8> {
    vec![
        0x01, 0x00, 0x04, 0x80, // revision, align, control
        0x00, 0x00, 0x00, 0x00, // owner (none)
        0x00, 0x00, 0x00, 0x00, // group (none)
        0x00, 0x00, 0x00, 0x00, // SACL (none)
        0x14, 0x00, 0x00, 0x00, // DACL offset = 20
        // DACL header
        0x02, 0x00, // revision
        0x1C, 0x00, // size = 28
        0x01, 0x00, // ACE count
        0x00, 0x00, // padding
        // ACE: Allow Everyone (S-1-1-0) full access
        0x00, 0x00, // type=ACCESS_ALLOWED, flags
        0x14, 0x00, // size
        0xFF, 0x01, 0x1F, 0x00, // mask
        0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, // SID S-1-1-0
    ]
}

// ── Per-handle file context ──────────────────────────────────────
pub struct FsFileCtx {
    path: String,
    is_dir: bool,
    write_buf: Option<std::fs::File>,
}

// ── Main filesystem context ──────────────────────────────────────
pub struct FsCtx {
    rc: Mutex<RemoteClient>,
}

impl FsCtx {
    fn new(base_url: &str, cache: CacheConfig) -> Self {
        Self {
            rc: Mutex::new(RemoteClient::new(base_url, cache)),
        }
    }

    fn stat(&self, path: &str) -> Option<RemoteEntry> {
        if path.is_empty() {
            return Some(RemoteEntry {
                name: String::new(),
                is_dir: true,
                size: 0,
            });
        }
        let parent = parent_of(path);
        let name = filename_of(path);
        self.rc
            .lock()
            .unwrap()
            .list_dir(&parent)
            .ok()?
            .into_iter()
            .find(|e| e.name == name)
    }
}

// ── FileSystemContext implementation ─────────────────────────────
impl FileSystemContext for FsCtx {
    type FileContext = FsFileCtx;

    fn get_security_by_name(
        &self,
        file_name: &U16CStr,
        security_descriptor: Option<&mut [c_void]>,
        _resolve: impl FnOnce(&U16CStr) -> Option<FileSecurity>,
    ) -> winfsp::Result<FileSecurity> {
        let path = wide_to_path(file_name);
        let entry = self
            .stat(&path)
            .ok_or_else(|| nt(STATUS_OBJECT_NAME_NOT_FOUND))?;

        let sd = default_security_descriptor();
        if let Some(buf) = security_descriptor {
            let bytes: &mut [u8] = unsafe {
                std::slice::from_raw_parts_mut(
                    buf.as_mut_ptr() as *mut u8,
                    buf.len() * std::mem::size_of::<c_void>(),
                )
            };
            let n = sd.len().min(bytes.len());
            bytes[..n].copy_from_slice(&sd[..n]);
        }

        Ok(FileSecurity {
            attributes: if entry.is_dir {
                FILE_ATTRIBUTE_DIRECTORY
            } else {
                FILE_ATTRIBUTE_NORMAL
            },
            reparse: false,
            sz_security_descriptor: sd.len() as u64,
        })
    }

    fn open(
        &self,
        file_name: &U16CStr,
        _create_options: u32,
        _granted_access: winfsp_sys::FILE_ACCESS_RIGHTS,
        file_info: &mut OpenFileInfo,
    ) -> winfsp::Result<Self::FileContext> {
        let path = wide_to_path(file_name);
        let entry = self
            .stat(&path)
            .ok_or_else(|| nt(STATUS_OBJECT_NAME_NOT_FOUND))?;

        *file_info.as_mut() = make_file_info(entry.is_dir, entry.size);
        Ok(FsFileCtx {
            path,
            is_dir: entry.is_dir,
            write_buf: None,
        })
    }

    fn close(&self, _context: Self::FileContext) {}

    fn get_file_info(
        &self,
        context: &Self::FileContext,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        let size = if context.is_dir {
            0
        } else {
            self.stat(&context.path).map(|e| e.size).unwrap_or(0)
        };
        *file_info = make_file_info(context.is_dir, size);
        Ok(())
    }

    fn get_volume_info(&self, out: &mut VolumeInfo) -> winfsp::Result<()> {
        out.total_size = 1024 * 1024 * 1024;
        out.free_size = 512 * 1024 * 1024;
        out.set_volume_label("RemoteFS");
        Ok(())
    }

    fn read_directory(
        &self,
        context: &Self::FileContext,
        _pattern: Option<&U16CStr>,
        marker: DirMarker,
        buffer: &mut [u8],
    ) -> winfsp::Result<u32> {
        let entries = self
            .rc
            .lock()
            .unwrap()
            .list_dir(&context.path)
            .map_err(|_| nt(STATUS_UNSUCCESSFUL))?;

        let mut all: Vec<(String, bool, u64)> = vec![
            (".".into(), true, 0),
            ("..".into(), true, 0),
        ];
        for e in &entries {
            all.push((e.name.clone(), e.is_dir, e.size));
        }

        let mut cursor: u32 = 0;
        let mut past_marker = marker.is_none();

        for (name, is_dir, size) in &all {
            if !past_marker {
                if let Some(m) = marker.inner_as_cstr() {
                    if let Ok(wide) = U16CString::from_str(name) {
                        if m == &*wide {
                            past_marker = true;
                        }
                    }
                }
                continue;
            }

            let mut di = DirInfo::<255>::new();
            *di.file_info_mut() = make_file_info(*is_dir, *size);
            if di.set_name(name.as_str()).is_err() {
                continue;
            }
            if !di.append_to_buffer(buffer, &mut cursor) {
                break;
            }
        }

        DirInfo::<255>::finalize_buffer(buffer, &mut cursor);
        Ok(cursor)
    }

    fn read(
        &self,
        context: &Self::FileContext,
        buffer: &mut [u8],
        offset: u64,
    ) -> winfsp::Result<u32> {
        if let Some(ref wb) = context.write_buf {
            let mut f = wb.try_clone().map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
            f.seek(SeekFrom::Start(offset))
                .map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
            let n = f.read(buffer).map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
            return Ok(n as u32);
        }

        let mut rc = self.rc.lock().unwrap();
        if let Some(cached) = rc.cached_file_data(&context.path) {
            let start = offset as usize;
            if start >= cached.len() {
                return Ok(0);
            }
            let end = (start + buffer.len()).min(cached.len());
            buffer[..end - start].copy_from_slice(&cached[start..end]);
            return Ok((end - start) as u32);
        }

        let data = rc
            .fetch_range(&context.path, offset, buffer.len() as u32)
            .map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
        let n = data.len().min(buffer.len());
        buffer[..n].copy_from_slice(&data[..n]);
        Ok(n as u32)
    }

    fn create(
        &self,
        file_name: &U16CStr,
        _create_options: u32,
        _granted_access: winfsp_sys::FILE_ACCESS_RIGHTS,
        file_attributes: winfsp_sys::FILE_FLAGS_AND_ATTRIBUTES,
        _security_descriptor: Option<&[c_void]>,
        _allocation_size: u64,
        _extra_buffer: Option<&[u8]>,
        _extra_buffer_is_reparse_point: bool,
        file_info: &mut OpenFileInfo,
    ) -> winfsp::Result<Self::FileContext> {
        let path = wide_to_path(file_name);
        let is_dir = (file_attributes & FILE_ATTRIBUTE_DIRECTORY) != 0;

        {
            let mut rc = self.rc.lock().unwrap();
            if is_dir {
                rc.mkdir_remote(&path)
                    .map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
            } else {
                rc.upload(&path, Vec::new())
                    .map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
            }
            rc.invalidate(&path);
        }

        *file_info.as_mut() = make_file_info(is_dir, 0);
        let write_buf = if !is_dir {
            Some(tempfile::tempfile().map_err(|_| nt(STATUS_UNSUCCESSFUL))?)
        } else {
            None
        };
        Ok(FsFileCtx {
            path,
            is_dir,
            write_buf,
        })
    }

    fn write(
        &self,
        context: &Self::FileContext,
        buf: &[u8],
        offset: u64,
        _write_to_eof: bool,
        _constrained_io: bool,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<u32> {
        let wb = context
            .write_buf
            .as_ref()
            .ok_or_else(|| nt(STATUS_INVALID_DEVICE_REQUEST))?;
        let mut f = wb.try_clone().map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
        f.seek(SeekFrom::Start(offset))
            .map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
        f.write_all(buf).map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
        let size = f.metadata().map(|m| m.len()).unwrap_or(0);
        *file_info = make_file_info(false, size);
        Ok(buf.len() as u32)
    }

    fn overwrite(
        &self,
        context: &Self::FileContext,
        _file_attributes: winfsp_sys::FILE_FLAGS_AND_ATTRIBUTES,
        _replace_file_attributes: bool,
        _allocation_size: u64,
        _extra_buffer: Option<&[u8]>,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        if let Some(ref wb) = context.write_buf {
            wb.set_len(0).map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
        }
        let mut rc = self.rc.lock().unwrap();
        rc.upload(&context.path, Vec::new())
            .map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
        rc.invalidate(&context.path);
        *file_info = make_file_info(false, 0);
        Ok(())
    }

    fn cleanup(
        &self,
        context: &Self::FileContext,
        _file_name: Option<&U16CStr>,
        flags: u32,
    ) {
        if flags & 0x01 != 0 {
            let mut rc = self.rc.lock().unwrap();
            let _ = rc.delete_remote(&context.path);
            rc.invalidate(&context.path);
            return;
        }

        if let Some(ref wb) = context.write_buf {
            if let Ok(mut f) = wb.try_clone() {
                if f.seek(SeekFrom::Start(0)).is_ok() {
                    let mut data = Vec::new();
                    if f.read_to_end(&mut data).is_ok() && !data.is_empty() {
                        let mut rc = self.rc.lock().unwrap();
                        let _ = rc.upload(&context.path, data);
                        rc.invalidate(&context.path);
                    }
                }
            }
        }
    }

    fn flush(
        &self,
        context: Option<&Self::FileContext>,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        if let Some(ctx) = context {
            let size = if let Some(ref wb) = ctx.write_buf {
                wb.metadata().map(|m| m.len()).unwrap_or(0)
            } else {
                self.stat(&ctx.path).map(|e| e.size).unwrap_or(0)
            };
            *file_info = make_file_info(ctx.is_dir, size);
        }
        Ok(())
    }

    fn set_basic_info(
        &self,
        context: &Self::FileContext,
        _file_attributes: u32,
        _creation_time: u64,
        _last_access_time: u64,
        _last_write_time: u64,
        _last_change_time: u64,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        self.get_file_info(context, file_info)
    }

    fn set_file_size(
        &self,
        context: &Self::FileContext,
        new_size: u64,
        _set_allocation_size: bool,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        if let Some(ref wb) = context.write_buf {
            wb.set_len(new_size)
                .map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
        }
        *file_info = make_file_info(context.is_dir, new_size);
        Ok(())
    }

    fn rename(
        &self,
        _context: &Self::FileContext,
        file_name: &U16CStr,
        new_file_name: &U16CStr,
        _replace_if_exists: bool,
    ) -> winfsp::Result<()> {
        let old = wide_to_path(file_name);
        let new = wide_to_path(new_file_name);
        let mut rc = self.rc.lock().unwrap();
        let data = rc
            .fetch_file(&old)
            .map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
        rc.upload(&new, data)
            .map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
        rc.delete_remote(&old)
            .map_err(|_| nt(STATUS_UNSUCCESSFUL))?;
        rc.invalidate(&old);
        rc.invalidate(&new);
        Ok(())
    }

    fn set_delete(
        &self,
        _context: &Self::FileContext,
        _file_name: &U16CStr,
        _delete_file: bool,
    ) -> winfsp::Result<()> {
        Ok(())
    }
}

// ── Entry point ──────────────────────────────────────────────────
pub fn run(cli: &Cli) {
    println!("Remote File System — Windows (WinFSP)");
    println!("Server: {}", cli.server_url);
    println!("Mount:  {}", cli.mountpoint);

    if cli.no_cache {
        println!("Cache: disabled");
    } else {
        println!(
            "Cache: dir_ttl={}s, file_ttl={}s, max={}MB",
            cli.dir_cache_ttl, cli.file_cache_ttl, cli.max_cache_mb
        );
    }

    let _init = winfsp::winfsp_init_or_die();

    let cache = CacheConfig::from_cli(
        cli.no_cache,
        cli.dir_cache_ttl,
        cli.file_cache_ttl,
        cli.max_cache_mb,
    );
    let ctx = FsCtx::new(&cli.server_url, cache);

    let mut params = VolumeParams::new();
    params
        .filesystem_name("remote-fs")
        .file_info_timeout(1000)
        .case_sensitive_search(false)
        .case_preserved_names(true)
        .unicode_on_disk(true);

    let mut host =
        FileSystemHost::new(params, ctx).expect("Failed to create WinFSP filesystem host");

    let mp = std::ffi::OsString::from(&cli.mountpoint);
    host.mount(mp).expect("Failed to mount filesystem");
    host.start().expect("Failed to start filesystem dispatcher");

    println!(
        "Filesystem mounted successfully at {}",
        cli.mountpoint
    );
    println!("Press Ctrl+C to unmount and exit.");

    // Block forever; Ctrl+C terminates the process and WinFSP cleans up.
    loop {
        std::thread::park();
    }
}
