//! The window's observation endpoint: a private socket and an unguessable key.
//!
//! **Threat model.** Anything that can reach this socket *and* present the key
//! can read every pane in the window. So the key is unguessable, per-window,
//! injected only into sessions this window launches, and destroyed with the
//! window; the socket lives in a directory only its owner can enter; and a
//! request that fails authentication is answered with one fixed refusal that
//! says nothing about why.
//!
//! Only Unix-domain sockets are used. No TCP port is ever opened — a listening
//! port would be reachable by anything on the machine that can talk to
//! loopback, which is a much larger set of things than "processes this window
//! started". A test asserts the running process holds no TCP socket at all.

use std::ffi::OsString;
#[cfg(test)]
use std::fs;
use std::io::Write;
#[cfg(test)]
use std::io::{BufReader, Read};
#[cfg(test)]
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::pane_tree::PaneId;
use crate::tabs::TabId;

/// The one answer to every request that is not allowed to proceed.
///
/// Identical for a missing key, a wrong key, and a pane the caller may not see,
/// because telling those apart would let a caller probe for which panes exist
/// by watching how the refusal changes.
pub const DENIED: &str = "denied";

/// A client sending more than this before a newline is not making a request.
const MAX_REQUEST_BYTES: u64 = 8 * 1024;

/// A connected client that never speaks must not hold its own thread for long.
///
/// A local client connects and writes immediately, so this only ever expires
/// for something that is not making a request.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(2);

/// How many requests may be in flight at once.
///
/// Each connection is served on its own thread so that one silent client
/// cannot stall the window's endpoint — but unbounded threads would simply
/// move the denial of service rather than remove it, so past this many the
/// endpoint drops new connections without reading them.
const MAX_CONNECTIONS: usize = 16;

pub use crate::local_socket::ObservationKey;
#[cfg(test)]
use crate::local_socket::bind_private;
use crate::local_socket::{Authenticated, LocalSocket, TransportPolicy};
#[allow(unused_imports)]
pub(crate) use crate::local_socket::{MAX_SOCKET_PATH, runtime_directory, sweep_dead_sockets};

/// What an authenticated caller asked for.
///
/// The key is already checked and deliberately absent: nothing downstream can
/// re-examine or leak it.
#[derive(Clone, Debug)]
pub struct Request {
    /// Everything after the key, verbatim. Task 6 gives this meaning.
    pub body: String,
    pub(crate) reply_connection: crate::local_socket::ReplyConnection,
}

/// One window's socket and key.
///
/// Dropping or [`close`](Endpoint::close)ing it removes the socket from the
/// filesystem and wipes the key, so a captured key stops working.
pub struct Endpoint {
    transport: LocalSocket,
}

impl Endpoint {
    /// Opens the endpoint and starts serving requests with `handler`.
    ///
    /// `handler` is called only for requests that presented the right key.
    pub fn open<H>(handler: H) -> std::io::Result<Self>
    where
        H: Fn(Request) -> String + Send + Sync + 'static,
    {
        Self::open_in(runtime_directory()?, handler)
    }

    /// Opens the endpoint inside a named directory.
    ///
    /// Split out from [`open`](Endpoint::open) so the tests do not depend on
    /// the ambient environment. They used to, and it cost this project a red
    /// CI on both platforms for three checkpoints: a container has no
    /// `XDG_RUNTIME_DIR` and macOS has none at all, so twelve tests failed
    /// everywhere except a logged-in Linux desktop. A test that only passes on
    /// the machine it was written on is not a gate.
    ///
    /// The privacy rules are applied here, to whatever directory is named, so a
    /// caller cannot obtain a laxer endpoint by choosing a different one.
    pub fn open_in<H>(directory: PathBuf, handler: H) -> std::io::Result<Self>
    where
        H: Fn(Request) -> String + Send + Sync + 'static,
    {
        let policy = TransportPolicy {
            name: "sprite-observation",
            filename_hex: 24,
            suffix: ".sock",
            max_connections: MAX_CONNECTIONS,
            max_first_line: MAX_REQUEST_BYTES as usize,
            handshake_timeout: CLIENT_TIMEOUT,
            write_timeout: CLIENT_TIMEOUT,
        };
        let transport = LocalSocket::open_in(
            directory,
            Arc::new(ObservationKey::generate()?),
            policy,
            refuse,
            move |connection| answer(connection, &handler),
        )?;
        Ok(Self { transport })
    }

    pub fn socket_path(&self) -> &Path {
        self.transport.socket_path()
    }

    /// The key, for injection into this window's own children only.
    pub fn key_hex(&self) -> String {
        self.transport.key_hex()
    }

    /// What one pane's session needs to talk to this endpoint.
    ///
    /// A session learns the socket, the key, and **its own** identity. It is
    /// told who it is so a request can default to the caller's own tab without
    /// the caller having to name a pane it might not be allowed to see.
    pub fn environment(&self, tab: TabId, pane: PaneId) -> Vec<(OsString, OsString)> {
        vec![
            (
                OsString::from("SPRITE_OBSERVATION_SOCKET"),
                OsString::from(self.socket_path().as_os_str()),
            ),
            (
                OsString::from("SPRITE_OBSERVATION_KEY"),
                OsString::from(self.key_hex()),
            ),
            (
                OsString::from("SPRITE_TAB"),
                OsString::from(tab.0.to_string()),
            ),
            (
                OsString::from("SPRITE_PANE"),
                OsString::from(pane.0.to_string()),
            ),
        ]
    }

    /// Destroys the socket and stops serving. The key is wiped when the last
    /// reference to it drops.
    pub fn close(&mut self) {
        self.transport.close();
    }

    pub(crate) fn close_after_reply(
        &mut self,
        reply: Option<&crate::local_socket::ReplyConnection>,
    ) {
        self.transport.close_after_reply(reply);
    }
}

fn answer<H>(connection: Authenticated, handler: &H)
where
    H: Fn(Request) -> String,
{
    let Authenticated {
        mut stream,
        body,
        reply_connection,
        ..
    } = connection;
    let response = handler(Request {
        body,
        reply_connection,
    });
    let _ = writeln!(stream, "{response}");
    let _ = stream.shutdown(std::net::Shutdown::Write);
}

fn refuse(stream: &mut UnixStream) {
    let _ = writeln!(stream, "{DENIED}");
    let _ = stream.shutdown(std::net::Shutdown::Write);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufWriter;
    use std::sync::Mutex;

    fn assert_denied_without_dispatch(body: &str) {
        let (_scratch, endpoint, calls) = endpoint_with_spy();
        let mut stream = UnixStream::connect(endpoint.socket_path()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(stream, "{} {body}", endpoint.key_hex()).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        assert_eq!(answer.trim(), DENIED);
        assert!(calls.0.lock().unwrap().is_empty());
    }

    #[test]
    fn incomplete_authenticated_lines_are_denied_before_dispatch() {
        assert_denied_without_dispatch("hello");
    }

    #[test]
    fn oversized_authenticated_lines_are_denied_before_dispatch() {
        assert_denied_without_dispatch(&format!("{}\n", "x".repeat(MAX_REQUEST_BYTES as usize)));
    }

    /// A private directory of this test's own, removed when it is dropped.
    ///
    /// Tests take one rather than reading `XDG_RUNTIME_DIR`, so the suite
    /// passes in a container, over ssh, and on macOS — none of which have one.
    struct Scratch(PathBuf);

    /// The name of one test's scratch directory.
    ///
    /// A free function so its *width* can be asserted at the widest inputs it
    /// could ever be given, rather than at whatever this run's pid happens to
    /// be. Kept short on purpose: this sits inside `$TMPDIR`, which on macOS is
    /// already a ~48-byte path under `/var/folders`, and what is left has to
    /// hold a socket name.
    fn scratch_name(pid: u32, ordinal: u64) -> String {
        format!("sp-{pid:x}-{ordinal:x}")
    }

    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let ordinal = NEXT.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(scratch_name(std::process::id(), ordinal));
            let _ = fs::remove_dir_all(&path);
            Self(path)
        }

        fn path(&self) -> PathBuf {
            self.0.clone()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Sends one request and returns the answer.
    fn ask(socket: &Path, line: &str) -> String {
        let stream = UnixStream::connect(socket).expect("connect to the endpoint");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        {
            let mut writer = BufWriter::new(&stream);
            writeln!(writer, "{line}").expect("send the request");
            writer.flush().expect("flush");
        }
        // Read to end of stream, which is how a client frames a response that
        // may be laid out over several lines.
        let mut answer = String::new();
        BufReader::new(&stream)
            .read_to_string(&mut answer)
            .expect("read the answer");
        answer.trim_end().to_owned()
    }

    /// Records what reached the handler, so "the handler was never called" is
    /// an assertion rather than an inference from the response text.
    #[derive(Default)]
    struct Calls(Mutex<Vec<String>>);

    /// One endpoint in a directory of the test's choosing.
    ///
    /// The directory is a parameter because two of these tests are *about* two
    /// endpoints sharing one: sweeping a dead socket, and two windows in the
    /// same runtime directory keeping their keys apart.
    fn endpoint_in(scratch: &Scratch) -> (Endpoint, Arc<Calls>) {
        let calls: Arc<Calls> = Arc::default();
        let endpoint = Endpoint::open_in(scratch.path(), {
            let calls = Arc::clone(&calls);
            move |request| {
                calls.0.lock().expect("lock").push(request.body.clone());
                format!("ok:{}", request.body)
            }
        })
        .expect("open endpoint");
        (endpoint, calls)
    }

    /// The common case: one endpoint, in a directory nobody else uses.
    ///
    /// **The scratch comes first, and the order is load-bearing.** Bindings are
    /// dropped in reverse, so a scratch returned last would delete the socket
    /// before the endpoint shut down — and shutting down means connecting to
    /// that socket to wake the thread parked in `accept`. With the file gone
    /// the connection fails, the thread never wakes, and the join hangs
    /// forever. This suite hung exactly that way once.
    fn endpoint_with_spy() -> (Scratch, Endpoint, Arc<Calls>) {
        let scratch = Scratch::new();
        let (endpoint, calls) = endpoint_in(&scratch);
        (scratch, endpoint, calls)
    }

    #[test]
    fn two_windows_never_share_a_key_or_a_socket() {
        // The same directory, which is the case that matters: two windows on
        // one machine share a runtime directory and must still be separate.
        let scratch = Scratch::new();
        let (first, _) = endpoint_in(&scratch);
        let (second, _) = endpoint_in(&scratch);

        assert_ne!(first.key_hex(), second.key_hex());
        assert_ne!(first.socket_path(), second.socket_path());
    }

    /// A weak key is the whole attack. Many draws, no repeats, full length.
    #[test]
    fn keys_are_long_and_do_not_repeat() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let key = ObservationKey::generate().expect("generate");
            let hex = key.to_hex();
            assert_eq!(hex.len(), 64, "32 bytes of key");
            assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(seen.insert(hex), "a key repeated, so it is not random");
        }
    }

    /// What `open_in` appends to the directory it is given: a separator, 24 hex
    /// characters, and `.sock`.
    const SOCKET_NAME_BYTES: usize = 1 + 24 + ".sock".len();

    /// A directory long enough that the socket inside it is exactly `target`.
    fn directory_of_socket_length(scratch: &Scratch, target: usize) -> PathBuf {
        let base = scratch.path();
        let wanted = target - SOCKET_NAME_BYTES;
        let base_len = base.as_os_str().len();
        assert!(
            base_len + 1 < wanted,
            "the scratch base is already {base_len} bytes, too long to pad up to {wanted}"
        );
        // One padded component, so no component approaches the 255-byte limit.
        let directory = base.join("p".repeat(wanted - base_len - 1));
        assert_eq!(
            directory.as_os_str().len() + SOCKET_NAME_BYTES,
            target,
            "the padding arithmetic is what this test rests on"
        );
        directory
    }

    /// This suite's own paths must fit the *tightest* platform, not whichever
    /// one it happens to be running on.
    ///
    /// This is the test that would have caught the original failure from Linux.
    /// The scratch directory used to be `sprite-endpoint-<pid>-<ordinal>`, and
    /// nested inside a macOS `$TMPDIR` its sockets came to ~102 bytes — fine
    /// against Linux's 107, refused by the flat 100 the guard then applied, and
    /// over macOS's 103 even once the guard was corrected. A suite that only
    /// fits on the platform it was written on is not a gate, so the budget is
    /// asserted here rather than discovered on a Mac.
    #[test]
    fn the_scratch_directory_leaves_room_for_a_macos_tmpdir() {
        /// `sun_path` on macOS, less its NUL terminator.
        const MACOS_SUN_PATH: usize = 103;
        /// `/var/folders/<2>/<~30>/T`, as `temp_dir` reports it — no trailing
        /// separator, because that is the form the length is measured in.
        const MACOS_TMPDIR: usize = 48;

        // The widest name this can realistically produce, not the one today's
        // pid gives: a test whose margin depends on how many digits the pid
        // happened to have passes or fails by luck. The pid is taken at its
        // true maximum; the ordinal is a counter of scratch directories within
        // one test process, and 65,535 is generous for a suite of ~300 tests.
        let widest = scratch_name(u32::MAX, 0xFFFF);
        let on_macos = MACOS_TMPDIR + 1 + widest.len() + SOCKET_NAME_BYTES;
        assert!(
            on_macos <= MACOS_SUN_PATH,
            "the widest scratch name ({widest}) gives a {on_macos}-byte socket path \
             inside a macOS $TMPDIR, over its {MACOS_SUN_PATH}-byte limit"
        );
    }

    /// A socket path the platform can carry is accepted, even past 100 bytes.
    ///
    /// The guard used to be a flat 100 on every platform. `sun_path` holds 108
    /// bytes on Linux and 104 on macOS, so 100 refused paths both platforms
    /// accept — and on macOS, where `$TMPDIR` alone is ~49 bytes, it refused
    /// this suite's own scratch directories and made every endpoint test fail
    /// there. This pins the limit to the platform rather than to a number.
    #[test]
    fn a_socket_path_the_platform_can_carry_is_accepted() {
        let scratch = Scratch::new();
        // Both platforms' limits are past the old flat 100, so on either one
        // this path is one the guard used to refuse.
        let directory = directory_of_socket_length(&scratch, MAX_SOCKET_PATH);

        let endpoint = Endpoint::open_in(directory, |_request| "ok".to_owned())
            .expect("a path the platform can carry is accepted");
        assert_eq!(
            endpoint.socket_path().as_os_str().len(),
            MAX_SOCKET_PATH,
            "the longest path this platform allows"
        );
        assert_eq!(
            ask(
                endpoint.socket_path(),
                &format!("{} hello", endpoint.key_hex())
            ),
            "ok",
            "and it is a working endpoint, not merely a path that was allowed"
        );
    }

    /// One byte past what the platform can carry is refused, and says so.
    ///
    /// The guard is not being removed, only corrected: a path `bind` could not
    /// hold must still fail with an explanation rather than an `EINVAL` from
    /// the kernel.
    #[test]
    fn a_socket_path_the_platform_cannot_carry_is_refused() {
        let scratch = Scratch::new();
        let directory = directory_of_socket_length(&scratch, MAX_SOCKET_PATH + 1);

        // Matched rather than `expect_err`, which would need `Debug` on
        // `Endpoint` — and a `Debug` on a type that owns a key is a way for the
        // key to reach a log.
        let message = match Endpoint::open_in(directory, |_request| "ok".to_owned()) {
            Ok(_) => panic!("one byte past the platform's limit was accepted"),
            Err(error) => error.to_string(),
        };
        assert!(
            message.contains(&MAX_SOCKET_PATH.to_string()),
            "the refusal names the limit it applied: {message}"
        );
    }

    #[test]
    fn the_socket_and_its_directory_are_private() {
        let (_scratch, endpoint, _) = endpoint_with_spy();

        let socket = fs::metadata(endpoint.socket_path()).expect("socket exists");
        assert_eq!(
            socket.permissions().mode() & 0o777,
            0o600,
            "only the owner may use the socket"
        );
        let directory = fs::metadata(endpoint.socket_path().parent().expect("parent"))
            .expect("directory exists");
        assert_eq!(
            directory.permissions().mode() & 0o777,
            0o700,
            "only the owner may enter the directory"
        );
    }

    #[test]
    fn a_request_with_the_right_key_reaches_the_handler() {
        let (_scratch, endpoint, calls) = endpoint_with_spy();

        let answer = ask(
            endpoint.socket_path(),
            &format!("{} panes snapshot", endpoint.key_hex()),
        );

        assert_eq!(answer, "ok:panes snapshot");
        assert_eq!(*calls.0.lock().expect("lock"), vec!["panes snapshot"]);
    }

    #[test]
    fn a_request_with_no_key_is_refused_without_reaching_the_handler() {
        let (_scratch, endpoint, calls) = endpoint_with_spy();

        assert_eq!(ask(endpoint.socket_path(), ""), DENIED);
        assert_eq!(ask(endpoint.socket_path(), "panes snapshot"), DENIED);
        assert!(
            calls.0.lock().expect("lock").is_empty(),
            "an unauthorised request never reaches the handler at all"
        );
    }

    #[test]
    fn a_request_with_the_wrong_key_is_refused_without_reaching_the_handler() {
        let (_scratch, endpoint, calls) = endpoint_with_spy();
        let mut wrong = endpoint.key_hex();
        // One byte different, so this also covers a near-miss rather than only
        // an obviously bogus key.
        wrong.replace_range(0..1, if wrong.starts_with('a') { "b" } else { "a" });

        assert_eq!(
            ask(endpoint.socket_path(), &format!("{wrong} panes snapshot")),
            DENIED
        );
        assert_eq!(
            ask(endpoint.socket_path(), "not-even-hex panes snapshot"),
            DENIED
        );
        assert!(calls.0.lock().expect("lock").is_empty());
    }

    /// The refusal must not say what was wrong, or a caller could tell "your
    /// key is bad" from "that pane is not yours" and map the window.
    #[test]
    fn every_refusal_is_the_same_answer() {
        // The handler refuses a pane the caller may not see, using the same
        // constant the endpoint uses for a bad key.
        let scratch = Scratch::new();
        let endpoint =
            Endpoint::open_in(scratch.path(), |_request| DENIED.to_owned()).expect("open endpoint");

        let bad_key = ask(endpoint.socket_path(), "0123 panes snapshot --pane 4");
        let missing_key = ask(endpoint.socket_path(), "panes snapshot");
        let forbidden_pane = ask(
            endpoint.socket_path(),
            &format!("{} panes snapshot --pane 999", endpoint.key_hex()),
        );

        assert_eq!(bad_key, DENIED);
        assert_eq!(missing_key, DENIED);
        assert_eq!(
            forbidden_pane, bad_key,
            "a refused pane and a refused key are indistinguishable"
        );
    }

    #[test]
    fn closing_the_window_destroys_the_socket_and_the_key_stops_working() {
        let (_scratch, mut endpoint, _) = endpoint_with_spy();
        let captured_key = endpoint.key_hex();
        let path = endpoint.socket_path().to_path_buf();
        assert_eq!(
            ask(&path, &format!("{captured_key} panes snapshot")),
            "ok:panes snapshot"
        );

        endpoint.close();

        assert!(!path.exists(), "the socket is gone from the filesystem");
        assert!(
            path.parent().expect("a directory").exists(),
            "but the directory other windows share is left alone"
        );
        let refused = UnixStream::connect(&path);
        assert!(
            refused.is_err(),
            "a captured key is worthless once the window has closed"
        );
    }

    #[test]
    fn a_session_is_told_the_socket_the_key_and_its_own_identity() {
        let (_scratch, endpoint, _) = endpoint_with_spy();

        let environment = endpoint.environment(TabId(3), PaneId(7));
        let lookup = |name: &str| {
            environment
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.to_string_lossy().into_owned())
                .expect("variable is present")
        };

        assert_eq!(
            lookup("SPRITE_OBSERVATION_SOCKET"),
            endpoint.socket_path().to_string_lossy()
        );
        assert_eq!(lookup("SPRITE_OBSERVATION_KEY"), endpoint.key_hex());
        assert_eq!(lookup("SPRITE_TAB"), "3");
        assert_eq!(lookup("SPRITE_PANE"), "7");
    }

    /// A client that connects and says nothing must not hold the endpoint: the
    /// next caller still gets served.
    #[test]
    fn a_silent_client_does_not_wedge_the_endpoint() {
        let (_scratch, endpoint, _) = endpoint_with_spy();
        let silent = UnixStream::connect(endpoint.socket_path()).expect("connect");

        let answer = ask(
            endpoint.socket_path(),
            &format!("{} panes snapshot", endpoint.key_hex()),
        );
        assert_eq!(answer, "ok:panes snapshot");
        drop(silent);
    }

    /// Turning observation off and on again must not revive the old endpoint.
    /// A key captured while it was enabled would otherwise start working again
    /// the moment someone re-enabled it.
    #[test]
    fn re_enabling_creates_a_new_endpoint_and_the_old_key_stays_dead() {
        let scratch = Scratch::new();
        let (mut first, _) = endpoint_in(&scratch);
        let captured_key = first.key_hex();
        let captured_path = first.socket_path().to_path_buf();
        assert_eq!(
            ask(&captured_path, &format!("{captured_key} panes snapshot")),
            "ok:panes snapshot"
        );

        // Disabled: the socket leaves the filesystem, not merely stops
        // answering.
        first.close();
        assert!(!captured_path.exists());

        // Re-enabled: a new endpoint, with a new key at a new path.
        let (second, calls) = endpoint_in(&scratch);
        assert_ne!(second.key_hex(), captured_key, "a fresh key");
        assert_ne!(second.socket_path(), captured_path, "at a fresh path");

        // The captured key is worthless against the new endpoint.
        assert_eq!(
            ask(
                second.socket_path(),
                &format!("{captured_key} panes snapshot")
            ),
            DENIED
        );
        assert!(
            calls.0.lock().expect("lock").is_empty(),
            "and it never reached the handler"
        );

        // Nothing about the old socket came back either.
        assert!(UnixStream::connect(&captured_path).is_err());
    }

    /// Sweeps until the socket at `expected_gone` is gone, or the bound is hit.
    ///
    /// One sweep is not always enough on macOS under load: a connect to a
    /// listener dropped a moment earlier can still *succeed*, before the
    /// listening socket is fully torn down, returning a socket with no peer
    /// that reads end-of-stream at once. The sweep then honestly finds nothing
    /// that refused and leaves the file alone. A connect a millisecond later is
    /// refused, so sweeping again clears it.
    ///
    /// The bound stays well under the 128-deep listen backlog on purpose: a
    /// live listener whose backlog is full also refuses, and a sweep that saw
    /// that would delete a socket still in use.
    fn sweep_until_gone(directory: &Path, expected_gone: &Path) {
        for _ in 0..32 {
            if !expected_gone.exists() {
                return;
            }
            sweep_dead_sockets(directory);
        }
    }

    /// A window killed rather than closed leaves its socket behind. The next
    /// window clears it — without disturbing a window that is still running.
    #[test]
    fn a_new_endpoint_clears_dead_sockets_but_not_live_ones() {
        // One directory for both, because a sweep only ever clears its own.
        let scratch = Scratch::new();
        let (live, _) = endpoint_in(&scratch);
        let live_path = live.socket_path().to_path_buf();
        let directory = live_path.parent().expect("a directory").to_path_buf();

        // Stands in for what a killed window leaves: a socket file with nothing
        // listening on it.
        let abandoned = directory.join("abandoned-by-a-killed-window.sock");
        {
            let listener = bind_private(&abandoned).expect("bind");
            drop(listener);
        }
        assert!(abandoned.exists());

        let (next, _) = endpoint_in(&scratch);
        sweep_until_gone(&directory, &abandoned);

        assert!(!abandoned.exists(), "the dead socket was cleared");
        assert!(live_path.exists(), "the live one was left alone");
        assert_eq!(
            ask(&live_path, &format!("{} panes snapshot", live.key_hex())),
            "ok:panes snapshot",
            "and still works"
        );
        assert!(next.socket_path().exists());
    }

    /// Only a refused connection proves nobody is listening. Any other error a
    /// loaded machine can return leaves the file alone: a stale file costs
    /// nothing, a removed live socket costs a window its observation.
    #[test]
    fn the_sweep_keeps_a_socket_it_could_not_prove_dead() {
        let scratch = Scratch::new();
        let directory = scratch.path().join("sockets");
        fs::create_dir_all(&directory).expect("dir");
        // A `.sock` name too long for a socket address: connecting fails
        // before any syscall with "path must be shorter than SUN_LEN", so it
        // is not a refusal and the sweep must not touch it. A name is used
        // rather than a regular file because "not a socket" is not a portable
        // non-refusal — Linux's connect reports refused for a regular file,
        // macOS reports not-a-socket — whereas the length check is Rust's own,
        // ahead of the kernel, and identical on both.
        let odd = directory.join(format!("{}.sock", "x".repeat(120)));
        fs::write(&odd, b"").expect("write");
        // A socket nobody listens on any more: refused, so removed.
        let dead = directory.join("dead.sock");
        drop(bind_private(&dead).expect("bind"));

        sweep_until_gone(&directory, &dead);

        assert!(odd.exists(), "a file that did not refuse is left alone");
        assert!(!dead.exists(), "a socket that refused is removed");
    }

    #[test]
    fn a_key_matches_only_itself() {
        let key = ObservationKey::generate().expect("generate");
        let hex = key.to_hex();

        assert!(key.matches(&hex));
        assert!(key.matches(&hex.to_uppercase()), "hex case is not a secret");
        assert!(!key.matches(""));
        assert!(
            !key.matches(&hex[..hex.len() - 1]),
            "a truncated key is not"
        );
        assert!(!key.matches(&format!("{hex}0")), "nor an extended one");
        assert!(!key.matches(&"0".repeat(64)));
    }
}
