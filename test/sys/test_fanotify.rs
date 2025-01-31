use crate::*;
use nix::sys::fanotify::{
    EventFFlags, Fanotify, FanotifyResponse, InitFlags, MarkFlags, MaskFlags,
    Response,
};
use std::fs::{read_link, File, OpenOptions};
use std::io::ErrorKind;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::thread;

#[test]
/// Run fanotify tests sequentially to avoid tmp files races
pub fn test_fanotify() {
    require_capability!("test_fanotify", CAP_SYS_ADMIN);

    test_fanotify_notifications();
    test_fanotify_responses();
}

fn test_fanotify_notifications() {
    let group =
        Fanotify::init(InitFlags::FAN_CLASS_NOTIF, EventFFlags::O_RDONLY)
            .unwrap();
    let tempdir = tempfile::tempdir().unwrap();

    group
        .mark(
            MarkFlags::FAN_MARK_ADD | MarkFlags::FAN_MARK_MOUNT,
            MaskFlags::FAN_OPEN | MaskFlags::FAN_MODIFY | MaskFlags::FAN_CLOSE,
            None,
            Some(tempdir.path()),
        )
        .unwrap();

    let tempfile = tempdir.path().join("test");

    // create test file
    File::create(&tempfile).unwrap();

    let [event] = &group.read_events().unwrap()[..] else {
        panic!("should have read exactly one event");
    };
    assert!(event.check_version());
    assert_eq!(
        event.mask(),
        MaskFlags::FAN_OPEN | MaskFlags::FAN_CLOSE_WRITE
    );
    let fd_opt = event.fd();
    let fd = fd_opt.as_ref().unwrap();
    let path = read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).unwrap();
    assert_eq!(path, tempfile);

    // modify test file
    {
        let mut f = OpenOptions::new().write(true).open(&tempfile).unwrap();
        f.write_all(b"hello").unwrap();
    }

    let [event] = &group.read_events().unwrap()[..] else {
        panic!("should have read exactly one event");
    };
    assert!(event.check_version());
    assert_eq!(
        event.mask(),
        MaskFlags::FAN_OPEN
            | MaskFlags::FAN_MODIFY
            | MaskFlags::FAN_CLOSE_WRITE
    );
    let fd_opt = event.fd();
    let fd = fd_opt.as_ref().unwrap();
    let path = read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).unwrap();
    assert_eq!(path, tempfile);

    // read test file
    {
        let mut f = File::open(&tempfile).unwrap();
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
    }

    let [event] = &group.read_events().unwrap()[..] else {
        panic!("should have read exactly one event");
    };
    assert!(event.check_version());
    assert_eq!(
        event.mask(),
        MaskFlags::FAN_OPEN | MaskFlags::FAN_CLOSE_NOWRITE
    );
    let fd_opt = event.fd();
    let fd = fd_opt.as_ref().unwrap();
    let path = read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).unwrap();
    assert_eq!(path, tempfile);
}

fn test_fanotify_responses() {
    let group =
        Fanotify::init(InitFlags::FAN_CLASS_CONTENT, EventFFlags::O_RDONLY)
            .unwrap();
    let tempdir = tempfile::tempdir().unwrap();

    group
        .mark(
            MarkFlags::FAN_MARK_ADD | MarkFlags::FAN_MARK_MOUNT,
            MaskFlags::FAN_OPEN_PERM,
            None,
            Some(tempdir.path()),
        )
        .unwrap();

    let tempfile = tempdir.path().join("test");
    let tempfname = tempfile.clone();

    let file_thread = thread::spawn(move || {
        // first try, should fail
        let Err(err) = File::create(&tempfile) else {
            panic!("first open is denied, should return error");
        };
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);

        // second try, should succeed
        File::create(&tempfile).unwrap();
    });

    // Deny the first open try
    let [event] = &group.read_events().unwrap()[..] else {
        panic!("should have read exactly one event");
    };
    assert!(event.check_version());
    assert_eq!(event.mask(), MaskFlags::FAN_OPEN_PERM);
    let fd_opt = event.fd();
    let fd = fd_opt.as_ref().unwrap();
    let path = read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).unwrap();
    assert_eq!(path, tempfname);
    group
        .write_response(FanotifyResponse::new(*fd, Response::FAN_DENY))
        .unwrap();

    //// Allow the second open try
    let [event] = &group.read_events().unwrap()[..] else {
        panic!("should have read exactly one event");
    };
    assert!(event.check_version());
    assert_eq!(event.mask(), MaskFlags::FAN_OPEN_PERM);
    let fd_opt = event.fd();
    let fd = fd_opt.as_ref().unwrap();
    let path = read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).unwrap();
    assert_eq!(path, tempfname);
    group
        .write_response(FanotifyResponse::new(*fd, Response::FAN_ALLOW))
        .unwrap();

    file_thread.join().unwrap();
}
