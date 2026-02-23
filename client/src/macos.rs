use crate::Cli;
use crate::types::CacheConfig;

pub fn run(cli: &Cli) {
    if !std::path::Path::new("/Library/Frameworks/macFUSE.framework").exists() {
        eprintln!("macFUSE is not installed.");
        eprintln!("Install with: brew install --cask macfuse");
        std::process::exit(1);
    }

    let cache = CacheConfig::from_cli(
        cli.no_cache, cli.dir_cache_ttl, cli.file_cache_ttl, cli.max_cache_mb,
    );
    crate::mount::run(&cli.mountpoint, &cli.server_url, cache);
}
