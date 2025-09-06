pub trait FuseAdapter {
    fn mount(&self, mountpoint: &str) -> Result<(), String>;
}

#[cfg(target_os = "linux")]
pub struct LinuxFuseAdapter;

#[cfg(target_os = "linux")]
impl FuseAdapter for LinuxFuseAdapter {
    fn mount(&self, mountpoint: &str) -> Result<(), String> {
        todo!()
    }
}

#[cfg(target_os = "macos")]
pub struct MacOSFuseAdapter;

#[cfg(target_os = "macos")]
impl FuseAdapter for MacOSFuseAdapter {
    fn mount(&self, mountpoint: &str) -> Result<(), String> {
        todo!()
    }
}

#[cfg(target_os = "windows")]
pub struct WindowsFuseAdapter;

#[cfg(target_os = "windows")]
impl FuseAdapter for WindowsFuseAdapter {
    fn mount(&self, mountpoint: &str) -> Result<(), String> {
        todo!();    
    }
}   


