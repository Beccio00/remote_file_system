use clap::Parser;

mod cli;
mod remote_client;
mod types;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

fn main() {
    let cli = cli::Cli::parse();

    #[cfg(unix)]
    unix::run(&cli);

    #[cfg(windows)]
    windows::run(&cli);
}
