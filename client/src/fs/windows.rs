use crate::fs::RemoteFSAdapter;

#[cfg(target_os = "windows")]
pub struct WindowsFuseAdapter;

#[cfg(target_os = "windows")]
impl RemoteFSAdapter for WindowsFuseAdapter {
    fn mount(&self, mountpoint: &str) -> Result<(), String> {
        todo!();    
    }
}   

