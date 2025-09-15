// Common trait between different OS implementations

pub mod linux;
pub mod macos;
pub mod windows;

pub trait FuseAdapter {
    fn mount(&self, mountpoint: &str) -> Result<(), String>;
}
