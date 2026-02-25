mod remote_fs;
mod mount;
mod linux;
mod macos;

pub fn run(cli: &crate::Cli) {
    #[cfg(target_os = "linux")]
    linux::run(cli);

    #[cfg(target_os = "macos")]
    macos::run(cli);
}
