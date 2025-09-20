// Common trait between different OS implementations
use std::path::Path;
use std::fmt;
use crate::http_client::HttpClient;

pub mod linux;
pub mod macos;
pub mod windows;

// Maybe can not be needed
#[derive(Debug)]
pub enum FsError {
    MountPointNotFound(String),
    
    PermissionDenied(String),
    
    FuseNotAvailable,
    
    IoError(std::io::Error),
    
    ConfigError(String),
    
    RemoteError(String),
    
    Other(String),
}


impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FsError::MountPointNotFound(path) => write!(f, "Mount point not found: {}", path),
            FsError::PermissionDenied(msg) => write!(f, "Permission denied: {}", msg),
            FsError::FuseNotAvailable => write!(f, "FUSE is not available on this system"),
            FsError::IoError(e) => write!(f, "I/O error: {}", e),
            FsError::ConfigError(msg) => write!(f, "Configuration error: {}", msg),
            FsError::RemoteError(msg) => write!(f, "Remote server error: {}", msg),
            FsError::Other(msg) => write!(f, "Error: {}", msg),
        }
    }
}

impl std::error::Error for FsError {}

impl From<std::io::Error> for FsError {
    fn from(error: std::io::Error) -> Self {
        FsError::IoError(error)
    }
}

impl From<String> for FsError {
    fn from(error: String) -> Self {
        FsError::Other(error)
    }
}

pub trait RemoteFSAdapter {
    fn new(client: HttpClient) -> Self;

    fn mount(self, mountpoint: &str) -> Result<(), FsError> where Self: Sized;

    fn unmount(&self, mountpoint: &str) -> Result<(), ()>;
}
