use std::process::Command;
use std::env;
use std::io;
use std::ptr;
use std::fs;
use std::path::{PathBuf};
use std::ffi::{CString};

use log::{debug, LevelFilter};
use env_logger;
use libc;
use tempfile::tempdir;

#[macro_use]
extern crate bitflags;

bitflags! {
    struct MountFlags : u32 {
        const REC        = 0b000001;
        const BIND       = 0b000010;
        const SLAVE      = 0b000100;
        const SHARED     = 0b001000;
        const PRIVATE    = 0b010000;
        const UNBINDABLE = 0b100000;
    }
}

bitflags! {
    struct UmountFlags : u32 {
        const NOFOLLOW  = 0b000001;
        const DETACH    = 0b000010;
    }
}

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

fn pivot_root(new_root : &str, put_old : &str) -> io::Result<()> {
    debug!("pivot root to {} old at {}", new_root, put_old);
    // TODO pass args
    let c_new_root = CString::new(new_root).expect("new root must not contain null bytes");
    let c_put_old = CString::new(put_old).expect("put old must not contain null bytes");
    match libc_result(unsafe{ libc::syscall(libc::SYS_pivot_root, c_new_root.as_ptr(), c_put_old.as_ptr())}, 0) {
        Ok(_) => Ok(()),
        Err(err) => Err(err)
    }
}

fn mount(src : &str, target : &str, fstype: &str, maybe_flags : Option<MountFlags>) -> io::Result<()> {
    let mut mnt_flags : libc::c_ulong = 0;

    if let Some(flags) = maybe_flags {
        let extra_bits = flags.bits() & !MountFlags::all().bits();
        if extra_bits != 0 {
            return Err(io::Error::new(io::ErrorKind::Other, format!("unexpected flags 0b{:b}", extra_bits)))
        }
        for flag in &[MountFlags::REC, MountFlags::BIND, MountFlags::SLAVE,
                      MountFlags::SHARED, MountFlags::PRIVATE,
                      MountFlags::UNBINDABLE] {
            if !flags.contains(*flag) {
                continue
            }
            match *flag {
                MountFlags::BIND => mnt_flags = mnt_flags | libc::MS_BIND,
                MountFlags::REC => mnt_flags = mnt_flags | libc::MS_REC,
                MountFlags::SLAVE => mnt_flags = mnt_flags | libc::MS_SLAVE,
                MountFlags::SHARED => mnt_flags = mnt_flags | libc::MS_SHARED,
                MountFlags::PRIVATE => mnt_flags = mnt_flags | libc::MS_PRIVATE,
                // there's no libc::MS_UNBINDABLE
                // sys/mount.h: MS_UNBINDABLE = 1 << 17
                MountFlags::UNBINDABLE => mnt_flags = mnt_flags | (1<<17),
                _ => return Err(io::Error::new(io::ErrorKind::Other, format!("unexpected flag {:x}", *flag))),
            }
        }
    }
    debug!("mount {} -> {} fs: {} flags: 0x{:b}", src, target, fstype,  mnt_flags);
    let c_src = CString::new(src).expect("source must not contain null bytes");
    let c_target = CString::new(target).expect("target must not contain null bytes");
    let c_fstype = CString::new(fstype).expect("fs type must not contain null bytes");
    match libc_result(unsafe{ libc::mount(c_src.as_ptr(), c_target.as_ptr(),
                                          c_fstype.as_ptr(), mnt_flags, ptr::null())},
                      0) {
        Ok(_) => Ok(()),
        Err(err) => Err(err)
    }
}

fn umount(target : &str, maybe_flags : Option<UmountFlags>) -> io::Result<()> {
    let mut umnt_flags : libc::c_int = 0;
    if let Some(flags) = maybe_flags {
        if flags.contains(UmountFlags::DETACH) {
            umnt_flags = umnt_flags | libc::MNT_DETACH;
        }
    }
    // no const for UMOUNT_NOFOLLOW
    // if flags & UmountFlags::UMOUNT_NOFOLLOW {
    //     umnt_flags = umnt_flags | libc::UMOUNT_NOFOLLOW;
    // }
    let c_target = CString::new(target).expect("target must not contain null bytes");
    match libc_result(unsafe {libc::umount2(c_target.as_ptr(), umnt_flags)}, 0) {
        Ok(_) => Ok(()),
        Err(err) => Err(err)
    }
}

fn cmd_from_args(program_args : &[String]) -> Command {
    debug!("command: {}", program_args[0]);
    let mut cmd = Command::new(program_args[0].as_str());
    for arg in program_args.iter().skip(1) {
        debug!("argument: \"{}\"", arg);
        cmd.arg(arg.as_str());
    }
    return cmd
}

fn main() {
    init_log();

    let args : Vec<String> = env::args().skip(1).collect();
    if args.len() < 2 {
        panic!("rootfs or command not provided");
    }
    let rootfs_unresolved = &args[0];
    debug!("new rootfs: {}", rootfs_unresolved);
    let rootfs = PathBuf::from(rootfs_unresolved).canonicalize()
        .expect(&format!("cannot resolve path: {}", rootfs_unresolved));
    let rootfs_str = rootfs.to_string_lossy();

    let mut cmd = cmd_from_args(&args[1..]);
    cmd.env_clear();

    let tmp = tempdir().expect("cannot create a temporary directory");
    let scratch_dir = tmp.path();
    debug!("scratch dir: {}", scratch_dir.to_string_lossy());

    debug!("unsharing mount ns");
    unshare_mnt().expect("failed to unshare mount namespace");

    let scratch_dir_str = &scratch_dir.to_string_lossy();

    // only needed if / isn't shared already
    mount("none", "/", "", Some(MountFlags::REC | MountFlags::SHARED))
        .expect("cannot make / recursively shared");

    mount(scratch_dir_str, scratch_dir_str, "", Some(MountFlags::BIND))
        .expect(&format!("cannot make {} a mount point", scratch_dir_str));
    mount("none", scratch_dir_str, "", Some(MountFlags::UNBINDABLE))
        .expect(&format!("cannot make {} unbindable", scratch_dir_str));

    debug!("mounting rootfs from {} to {}", rootfs_str, scratch_dir_str);

    mount(&rootfs_str, scratch_dir_str, "", Some(MountFlags::REC | MountFlags::BIND))
        .expect(&format!("cannot bind mount rootfs from {} to {}", rootfs_str, scratch_dir_str));

    // stop propagation of changes to the host
    mount("none", scratch_dir_str, "", Some(MountFlags::REC |MountFlags::SLAVE))
        .expect(&format!("cannot make rootfs at {} rslave", scratch_dir_str));

    let from_host = [
        "/dev",
        "/sys",
        "/proc",
    ];
    for loc in from_host.iter() {
        // join with absolute path replaces the path, so drop the leading /
        let target_path = scratch_dir.join(&loc[1..]);
        let target = &target_path.to_string_lossy();
        debug!("rbind mounting {} to {}", loc, target);
        // recursive bind
        mount(loc, &target, "", Some(MountFlags::REC | MountFlags::BIND))
            .expect(&format!("cannot bind mount {} to {}", loc, target));
        // stop propagation of changes to the host
        mount("none", &target, "", Some(MountFlags::REC | MountFlags::SLAVE))
              .expect(&format!("cannot make {} rslave", target));
    }

    // setup tmpfs
    mount("none", &scratch_dir.join("tmp").to_string_lossy(), "tmpfs", None)
        .expect("cannot mount a new tmpfs");

    // old rootfs in after pivot world
    let old_root = PathBuf::from("/tmp/old-root");
    let old_root_str = &old_root.to_string_lossy();
    // this it where old root will be put in the before pivot world
    let put_old = scratch_dir.join("tmp/old-root");
    let put_old_str = &put_old.to_string_lossy();
    // this is where the scratch dir is in after pivot world
    let scratch_in_old = old_root.join(&scratch_dir.to_string_lossy()[1..]);
    debug!("scratch dir after pivot root: {}", &scratch_in_old.to_string_lossy());

    fs::create_dir(&put_old).expect("cannot create temporary directory for old root");

    mount(put_old_str, put_old_str, "", Some(MountFlags::BIND))
        .expect("cannot make put old a mount point");
    mount("none", put_old_str, "", Some(MountFlags::PRIVATE))
        .expect("cannot set private propagation on put old");
    // switch root
    pivot_root(scratch_dir_str, put_old_str)
        .expect(&format!("cannot pivot root to {}", scratch_dir_str));

    // umount first so that rmdir works
    umount(&scratch_in_old.to_string_lossy(), None)
        .expect("cannot unmount scratch directory");
    debug!("remove scratch directory in old root");
    fs::remove_dir(scratch_in_old).expect("cannot remove old scratch location");

    // make old root slave, otherwise we would unmount the host root
    mount("none", old_root_str, "", Some(MountFlags::REC | MountFlags::SLAVE))
        .expect("cannot switch old root to slave");
    umount(&old_root.to_string_lossy(), Some(UmountFlags::DETACH))
        .expect("cannot unmount old root");
    debug!("remove old root at {} after pivot", &old_root.to_string_lossy());
    //fs::remove_dir(old_root).expect("cannot remove old root");


    // XXX run the command
    cmd.status()
        .expect("failed to execute process");
}
