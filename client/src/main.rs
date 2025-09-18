use std::path::Path;
mod fs;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>>  {
    env_logger::init();
    let adapter = fs::create_adapter()?;
    let mountpoint = "/tmp/remote_mount";
    if !Path::new(mountpoint).exists() {
        std::fs::create_dir_all(mountpoint)?;
    }
    println!("Mounting at {}...", mountpoint);
    adapter.mount(mountpoint, None).await?;
    println!("Mounted. Press Ctrl-C to unmount.");
    tokio::signal::ctrl_c().await?;
    println!("Unmounting {}...", mountpoint);
    adapter.unmount(mountpoint).await?;
    adapter.wait_until_unmount().await?;
    println!("Unmounted.");
    Ok(())
}