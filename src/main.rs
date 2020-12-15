use std::process::Command;
use std::env;
use std::io;
use std::ptr;
use std::ffi::{CString};

use log::{debug, LevelFilter};
use env_logger;
use libc;
use tempfile::tempdir;

fn init_log() {
    env_logger::Builder::new()
        .filter_level(LevelFilter::Debug)
        .init();
}

fn libc_result<T : Ord>(res : T, happy: T) -> io::Result<T> {
    if res != happy  {
        Err(io::Error::last_os_error())
    } else {
        Ok(res)
    }
}

fn unshare_mnt() -> io::Result<()> {
    match libc_result(unsafe { libc::unshare(libc::CLONE_NEWNS) }, 0) {
        Ok(_) => Ok(()),
        Err(err) => Err(err)
    }
}

fn mount(src : &str, target : &str, fstype: &str, maybe_options : Option<&Vec<&str>>) -> io::Result<()> {
    let mut flags : libc::c_ulong = 0;
    if let Some(options) = maybe_options {
        for opt in options {
            match *opt {
                "bind" => flags = flags | libc::MS_BIND,
                "rbind" => flags = flags | libc::MS_BIND | libc::MS_REC,
                "slave" => flags = flags | libc::MS_SLAVE,
                "shared" => flags = flags | libc::MS_SHARED,
                "private" => flags = flags | libc::MS_PRIVATE,
                _ => return Err(io::Error::new(io::ErrorKind::Other, format!("unexpected option {}", opt))),
            }
        }
    }
    let c_src = CString::new(src).expect("source must not contain null bytes");
    let c_target = CString::new(target).expect("target must not contain null bytes");
    let c_fstype = CString::new(fstype).expect("fs type must not contain null bytes");
    match libc_result(unsafe{ libc::mount(c_src.as_ptr(), c_target.as_ptr(),
                                          c_fstype.as_ptr(), flags, ptr::null())},
                      0) {
        Ok(_) => Ok(()),
        Err(err) => Err(err)
    }
}

fn main() {
    init_log();

    let args : Vec<String> = env::args().skip(1).collect();
    if args.len() == 0 {
        return ();
    }
    debug!("command: {}", args[0]);
    let mut cmd = Command::new(args[0].as_str());
    for arg in args.iter().skip(1) {
        debug!("argument: \"{}\"", arg);
        cmd.arg(arg.as_str());
    }
    cmd.env_clear();
    // cmd.status()
    //     .expect("failed to execute process");
    let tmp = tempdir().expect("cannot create a temporary directory");
    let root_tmp = tmp.path();
    debug!("root tmp: {}", root_tmp.to_string_lossy());

    unshare_mnt().expect("failed to unshare mount namespace");

    mount("", &root_tmp.to_string_lossy(), "tmpfs", None)
        .expect("cannot mount tmpfs");
}
