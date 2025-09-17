// Common trait between different OS implementations
use std::path::Path;
use async_trait::async_trait;
use std::fmt;

pub mod linux;
pub mod macos;
pub mod windows;

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

#[derive(Debug, Clone)]
pub struct MountOptions {
    pub read_only: bool,
    
    pub allow_other: bool,
    
    pub auto_unmount: bool,
    
    pub fs_name: String,
    
    pub platform_options: Vec<String>,
}

impl Default for MountOptions {
    fn default() -> Self {
        Self {
            read_only: false,
            allow_other: true,
            auto_unmount: true,
            fs_name: "remote-fs".to_string(),
            platform_options: Vec::new(),
        }
    }
}

#[async_trait]
pub trait FuseAdapter {
    fn init() -> Result<Self, FsError> where Self: Sized;
    
    fn new(server_url: String) -> Result<Self, FsError> where Self: Sized;
    
    async fn mount(&self, mountpoint: &str, options: Option<MountOptions>) -> Result<(), FsError>;

    async fn unmount(&self, mountpoint: &str) -> Result<(), FsError>;

    fn is_mounted(&self, mountpoint: &str) -> Result<bool, FsError>;
    
    async fn wait_until_unmount(&self) -> Result<(), FsError>;
}


///??
/// Crea l'adapter appropriato per la piattaforma corrente
pub fn create_adapter(server_url: String) -> Result<Box<dyn FuseAdapter>, FsError> {
    #[cfg(target_os = "linux")]
    {
        use crate::fs::linux::LinuxFuseAdapter;
        Ok(Box::new(LinuxFuseAdapter::new(server_url)?))
    }
    
    #[cfg(target_os = "macos")]
    {
        use crate::fs::macos::MacOSFuseAdapter;
        Ok(Box::new(MacOSFuseAdapter::new(server_url)?))
    }
    
    #[cfg(target_os = "windows")]
    {
        use crate::fs::windows::WindowsFuseAdapter;
        Ok(Box::new(WindowsFuseAdapter::new(server_url)?))
    }
    
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err(FsError::Other("Unsupported platform".to_string()))
    }
}
