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

/// Linux only, because the method is `/proc` and macOS has none.
///
/// This measures the property by reading `/proc/self/fd` and `/proc/net/tcp`.
/// There is no macOS equivalent that does not mean shelling out to `lsof`, so
/// the test is ignored there rather than rewritten around a second mechanism.
///
/// **macOS is not left uncovered.** The forbidden-state scan asserts the same
/// property by inspection — no TCP type is named anywhere in the crate — and
/// although that scan runs in the Linux job only, it greps source that both
/// platforms compile, so it covers the macOS build too. What macOS loses is the
/// measurement, not the guarantee.
///
/// This surfaced when the socket-path guard was corrected: before that, every
/// endpoint test failed on macOS long before this one ran, so a `/proc` test on
/// a platform without `/proc` was never reached.
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "reads /proc, which only Linux has; the forbidden-state scan covers this platform"
)]
#[test]
fn an_open_endpoint_holds_no_tcp_socket() {
    // A directory of this test's own rather than `XDG_RUNTIME_DIR`, which a
    // container does not set and macOS does not have.
    let directory = std::env::temp_dir().join(format!("sprite-no-tcp-{}", std::process::id()));
    let endpoint = Endpoint::open_in(directory.clone(), |_request| "unused".to_owned())
        .expect("open an endpoint");

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
