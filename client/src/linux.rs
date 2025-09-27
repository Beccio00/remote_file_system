use crate::common::{run_linux_macos, RemoteFS};
use fuser::MountOption;
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request,
};
use libc::ENOENT;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};


pub fn run(mountpoint: &str) {
    println!("Starting Remote File System on Linux...");

    run_linux_macos(mountpoint);
}

