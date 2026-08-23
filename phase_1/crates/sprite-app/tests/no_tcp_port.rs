//! The observation surface opens no TCP port.
//!
//! A listening port is reachable by anything on the machine that can talk to
//! loopback, which is a far larger set of things than "processes this window
//! started". The forbidden-state scan asserts this by inspection; this asserts
//! it by measurement, against a process that actually has the endpoint open.

use std::collections::HashSet;
use std::fs;

use sprite_app::Endpoint;

/// Socket inodes this process holds, read from its own descriptor table.
fn socket_inodes() -> HashSet<u64> {
    let mut inodes = HashSet::new();
    let entries = fs::read_dir("/proc/self/fd").expect("this test needs /proc");
    for entry in entries.flatten() {
        // A descriptor can close between listing and reading, which is not a
        // failure — it simply is not open any more.
        let Ok(target) = fs::read_link(entry.path()) else {
            continue;
        };
        let target = target.to_string_lossy();
        if let Some(inode) = target
            .strip_prefix("socket:[")
            .and_then(|rest| rest.strip_suffix(']'))
            .and_then(|number| number.parse().ok())
        {
            inodes.insert(inode);
        }
    }
    inodes
}

/// Every TCP socket inode on the machine, from both address families.
fn tcp_inodes() -> HashSet<u64> {
    let mut inodes = HashSet::new();
    for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(contents) = fs::read_to_string(table) else {
            continue;
        };
        // Columns are fixed: the inode is the tenth field of each entry row.
        for line in contents.lines().skip(1) {
            if let Some(inode) = line.split_whitespace().nth(9).and_then(|f| f.parse().ok()) {
                inodes.insert(inode);
            }
        }
    }
    inodes
}

#[test]
fn an_open_endpoint_holds_no_tcp_socket() {
    let endpoint = Endpoint::open(|_request| "unused".to_owned())
        .expect("open an endpoint; this test needs XDG_RUNTIME_DIR");

    // The endpoint is listening, so if it were ever going to open a port it
    // would have one now.
    assert!(
        endpoint.socket_path().exists(),
        "the endpoint is actually open, so this test is measuring something"
    );

    let ours = socket_inodes();
    assert!(
        !ours.is_empty(),
        "the Unix socket is visible in our descriptor table, so the method works"
    );

    let tcp = tcp_inodes();
    let overlap: Vec<u64> = ours.intersection(&tcp).copied().collect();
    assert!(
        overlap.is_empty(),
        "the process holds TCP socket inodes {overlap:?}; observation must be \
         reachable only through the private Unix socket"
    );
}
