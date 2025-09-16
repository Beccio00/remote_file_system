// Common trait between different OS implementations
use async_trait::async_trait;

pub mod linux;
pub mod macos;
pub mod windows;

#[async_trait]
pub trait FuseAdapter {
    fn init() -> Result<Self, String> where Self: Sized;
    
    async fn mount(&self, mountpoint: &str) -> Result<(), String>;

    async fn unmount(&self, mountpoint: &str) -> Result<(), String>;

    fn is_mounted(&self, mountpoint: &str) -> Result<bool, String>;

}
