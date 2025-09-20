use crate::fs::RemoteFSAdapter;

#[cfg(target_os = "macos")]
pub struct MacOSFuseAdapter;

#[cfg(target_os = "macos")]
impl RemoteFSAdapter for MacOSFuseAdapter {
    fn mount(&self, mountpoint: &str) -> Result<(), String> {
        todo!()
    }
}