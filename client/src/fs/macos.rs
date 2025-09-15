use crate::fs::FuseAdapter;

#[cfg(target_os = "macos")]
pub struct MacOSFuseAdapter;

#[cfg(target_os = "macos")]
impl FuseAdapter for MacOSFuseAdapter {
    fn mount(&self, mountpoint: &str) -> Result<(), String> {
        todo!()
    }
}