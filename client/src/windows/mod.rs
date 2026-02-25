mod remote_fs;
mod mount;

use crate::types::CacheConfig;
use crate::Cli;

pub fn run(cli: &Cli) {
    let cache = CacheConfig::from_cli(
        cli.no_cache,
        cli.dir_cache_ttl,
        cli.file_cache_ttl,
        cli.max_cache_mb,
    );
    mount::run(&cli.mountpoint, &cli.server_url, cache);
}
