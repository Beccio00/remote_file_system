//! Windows WinFSP filesystem implementation.
//! Mirrors unix/remote_fs.rs (FUSE) but uses the WinFSP FileSystemContext API.

use crate::remote_client::RemoteClient;
use crate::types::{CacheConfig, RemoteEntry, parent_of};

use std::ffi::c_void;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

use winfsp::filesystem::*;
use winfsp::{U16CStr, U16CString};

// ── Windows file-attribute constants ────────────────────────────
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

// ── NTSTATUS codes used for error mapping ───────────────────────
const STATUS_OBJECT_NAME_NOT_FOUND: i32 = 0xC000_0034_u32 as i32;
const STATUS_UNSUCCESSFUL: i32 = 0xC000_0001_u32 as i32;
const STATUS_INVALID_DEVICE_REQUEST: i32 = 0xC000_0010_u32 as i32;

fn nt(code: i32) -> winfsp::FspError {
    winfsp::FspError::NTSTATUS(code)
}


/// Convert a WinFSP wide path `\foo\bar` to the internal `foo/bar` form.
fn wide_to_path(name: &U16CStr) -> String {
    name.to_string_lossy()
        .trim_start_matches('\\')
        .replace('\\', "/")
}

fn filename_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Current time as a Windows FILETIME value
/// (100-nanosecond intervals since 1601-01-01).
fn filetime_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    EPOCH_DIFF + (dur.as_nanos() / 100) as u64
}

pub(super) fn make_file_info(is_dir: bool, size: u64) -> FileInfo {
    let now = filetime_now();
    FileInfo {
        file_attributes: if is_dir {
            FILE_ATTRIBUTE_DIRECTORY
        } else {
            FILE_ATTRIBUTE_NORMAL
        },
        file_size: size,
        allocation_size: (size + 4095) & !4095,
        creation_time: now,
        last_access_time: now,
        last_write_time: now,
        change_time: now,
        ..Default::default()
    }
}

/// Holds state for a single open file handle.
/// Equivalent to WriteBuffer in unix/remote_fs.rs.
pub struct FileCtx {
    pub path: String,
    pub is_dir: bool,
    /// Temporary file used for buffering writes before upload.
    pub write_buf: Option<std::fs::File>,
}

// ── Filesystem context ───────────────────────────────────────────
/// The WinFSP filesystem implementation.
/// Mirrors RemoteFS (FUSE) but implements FileSystemContext instead.
pub struct RemoteFS {
    rc: Mutex<RemoteClient>,
}

impl RemoteFS {
    pub fn new(base_url: &str, cache: CacheConfig) -> Self {
        Self {
            rc: Mutex::new(RemoteClient::new(base_url, cache)),
        }
    }

    /// Stat a path: returns `None` if the path does not exist on the server.
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
impl FileSystemContext for RemoteFS {
    type FileContext = FileCtx;

    fn get_security_by_name(
        &self,
        file_name: &U16CStr,
        _security_descriptor: Option<&mut [c_void]>,
        resolve: impl FnOnce(&U16CStr) -> Option<FileSecurity>,
    ) -> winfsp::Result<FileSecurity> {
        let path = wide_to_path(file_name);
        let _entry = self
            .stat(&path)
            .ok_or_else(|| nt(STATUS_OBJECT_NAME_NOT_FOUND))?;

            if let Some(fs) = resolve(file_name) {
            return Ok(fs);
        }

        Ok(FileSecurity {
            attributes: FILE_ATTRIBUTE_DIRECTORY,
            reparse: false,
            sz_security_descriptor: 0,
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
        Ok(FileCtx {
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

        let rc = self.rc.lock().unwrap();

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
        Ok(FileCtx { path, is_dir, write_buf })
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
