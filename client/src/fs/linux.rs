use crate::fs::FuseAdapter;

#[cfg(target_os = "linux")]
pub struct LinuxFuseAdapter;

#[cfg(target_os = "linux")]
impl FuseAdapter for LinuxFuseAdapter {
    fn mount(&self, mountpoint: &str) -> Result<(), String> {
        todo!()
    }
}