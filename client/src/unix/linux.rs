use crate::Cli;
use crate::types::CacheConfig;

/// Linux entry point that resolves cache settings and starts mounting.
pub fn run(cli: &Cli) {
    let cache = CacheConfig::from_cli(
        cli.no_cache, cli.dir_cache_ttl, cli.file_cache_ttl, cli.max_cache_mb,
    );
    super::mount::run(&cli.mountpoint, &cli.server_url, cache);
}
