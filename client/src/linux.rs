use crate::common::RemoteFS;
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
    println!("Mounting at: {}", mountpoint);
    println!("Press Ctrl+C to unmount and exit.");

    let fs = RemoteFS::new("http://127.0.0.1:8000");

    let options = vec![
        MountOption::FSName("remote-fs".to_string()),
        MountOption::DefaultPermissions,
        MountOption::AutoUnmount,  
        MountOption::AllowOther,   
    ];


    match fuser::mount2(fs, mountpoint, &options) {
        Ok(()) => {
            println!("File system mounted successfully at {}", mountpoint);
        }
        Err(e) => {
            eprintln!("Failed to mount file system: {}", e);
            eprintln!("Make sure:");
            eprintln!("1. FUSE is installed (apt install fuse)");
            eprintln!("2. The mount point exists and is empty");
            eprintln!("3. You have the necessary permissions");
            std::process::exit(1);
        }
    }

    println!("Unmounting file system...");


}

