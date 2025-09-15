use crate::fs::FuseAdapter;

#[cfg(target_os = "windows")]
pub struct WindowsFuseAdapter;

#[cfg(target_os = "windows")]
impl FuseAdapter for WindowsFuseAdapter {
    fn mount(&self, mountpoint: &str) -> Result<(), String> {
        todo!();    
    }
}   

