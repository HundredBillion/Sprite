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
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;
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

const KEY_BYTES: usize = 32;

/// A per-window secret, compared in constant time and wiped when dropped.
pub struct ObservationKey {
    bytes: [u8; KEY_BYTES],
}

impl ObservationKey {
    /// Generates a key from the operating system's cryptographic source.
    ///
    /// Read straight from `/dev/urandom` rather than through a random-number
    /// crate: this is the only randomness Sprite needs, and on Linux the device
    /// is the same CSPRNG such a crate would reach for, so the dependency would
    /// buy nothing and still have to be audited. It must never come from a
    /// seeded or reproducible generator — an observer who can predict the key
    /// can read every pane.
    pub fn generate() -> std::io::Result<Self> {
        let mut bytes = [0_u8; KEY_BYTES];
        let mut source = File::open("/dev/urandom")?;
        source.read_exact(&mut bytes)?;
        Ok(Self { bytes })
    }

    /// The key as lowercase hex, which is the only form that leaves this type.
    pub fn to_hex(&self) -> String {
        let mut hex = String::with_capacity(KEY_BYTES * 2);
        for byte in self.bytes {
            hex.push(nibble(byte >> 4));
            hex.push(nibble(byte & 0x0f));
        }
        hex
    }

    /// Whether `candidate` is this key, compared without leaking where it first
    /// differs.
    ///
    /// A comparison that stops at the first wrong byte tells a caller how much
    /// of a guess was right, which is enough to recover a key one byte at a
    /// time. Every path through this function looks at all 32 bytes.
    pub fn matches(&self, candidate: &str) -> bool {
        let mut guess = [0_u8; KEY_BYTES];
        // A malformed candidate is compared against zeroes rather than
        // returning early, so a wrong length costs the same as a wrong key.
        let well_formed = decode_hex(candidate, &mut guess);
        let mut difference = 0_u8;
        // Every byte, every time: `zip` over the full arrays visits all 32 just
        // as unconditionally as an indexed loop, so no path returns early.
        for (mine, theirs) in self.bytes.iter().zip(guess.iter()) {
            difference |= mine ^ theirs;
        }
        difference == 0 && well_formed
    }
}

impl Drop for ObservationKey {
    fn drop(&mut self) {
        // Closing the window destroys the key. Overwriting it means a later
        // read of freed memory finds zeroes rather than a working secret.
        self.bytes.fill(0);
        // `black_box` so the compiler cannot decide the store above is dead and
        // remove it. Deliberately not `write_volatile`: that would be the only
        // `unsafe` outside the one audited descriptor borrow in `sprite-term`,
        // and adding it here to wipe 32 bytes is a poor trade.
        std::hint::black_box(&self.bytes);
    }
}

fn nibble(value: u8) -> char {
    char::from_digit(u32::from(value), 16).unwrap_or('0')
}

/// Decodes exactly `KEY_BYTES` of hex, reporting whether the input was valid.
///
/// Always fills `out` and always inspects the whole buffer.
fn decode_hex(text: &str, out: &mut [u8; KEY_BYTES]) -> bool {
    let bytes = text.as_bytes();
    let mut valid = bytes.len() == KEY_BYTES * 2;
    for (index, slot) in out.iter_mut().enumerate() {
        let high = bytes.get(index * 2).copied().unwrap_or(b'!');
        let low = bytes.get(index * 2 + 1).copied().unwrap_or(b'!');
        match (hex_value(high), hex_value(low)) {
            (Some(high), Some(low)) => *slot = (high << 4) | low,
            _ => {
                valid = false;
                *slot = 0;
            }
        }
    }
    valid
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// What an authenticated caller asked for.
///
/// The key is already checked and deliberately absent: nothing downstream can
/// re-examine or leak it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    /// Everything after the key, verbatim. Task 6 gives this meaning.
    pub body: String,
}

/// One window's socket and key.
///
/// Dropping or [`close`](Endpoint::close)ing it removes the socket from the
/// filesystem and wipes the key, so a captured key stops working.
pub struct Endpoint {
    socket: PathBuf,
    directory: PathBuf,
    key: Arc<ObservationKey>,
    running: Arc<AtomicBool>,
    listener: Option<JoinHandle<()>>,
}

impl Endpoint {
    /// Opens the endpoint and starts serving requests with `handler`.
    ///
    /// `handler` is called only for requests that presented the right key.
    pub fn open<H>(handler: H) -> std::io::Result<Self>
    where
        H: Fn(Request) -> String + Send + Sync + 'static,
    {
        let directory = runtime_directory()?;
        // 0700: the socket's own mode is a second line of defence, but a
        // directory nobody else may enter is what actually keeps other users
        // off the socket, and it is set before the socket exists rather than
        // after — there is no window in which the path is reachable.
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        // A directory that already existed may have a laxer mode.
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;

        let key = Arc::new(ObservationKey::generate()?);
        // The filename is random, and deliberately *not* derived from the key:
        // a path appears in the environment and in process listings, so a path
        // that encoded the key would publish it.
        let mut name = ObservationKey::generate()?.to_hex();
        name.truncate(24);
        let socket = directory.join(format!("{name}.sock"));
        if socket.as_os_str().len() >= 100 {
            return Err(std::io::Error::other(format!(
                "the observation socket path is too long for a Unix socket: {}",
                socket.display()
            )));
        }

        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;

        let running = Arc::new(AtomicBool::new(true));
        let thread = std::thread::Builder::new()
            .name("sprite-observation".to_owned())
            .spawn({
                let key = Arc::clone(&key);
                let running = Arc::clone(&running);
                let handler = Arc::new(handler);
                move || serve(&listener, &key, &running, &handler)
            })?;

        Ok(Self {
            socket,
            directory,
            key,
            running,
            listener: Some(thread),
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    /// The key, for injection into this window's own children only.
    pub fn key_hex(&self) -> String {
        self.key.to_hex()
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
                OsString::from(self.socket.as_os_str()),
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
        if self.listener.is_none() {
            return;
        }
        self.running.store(false, Ordering::SeqCst);
        // The serving thread is parked in `accept`, so it has to be woken to
        // notice. Connecting to our own socket does that without a second
        // descriptor to poll on.
        let _ = UnixStream::connect(&self.socket);
        if let Some(thread) = self.listener.take() {
            let _ = thread.join();
        }
        // Removed only after the thread has stopped, so nothing can connect to
        // a socket whose server is already gone.
        let _ = fs::remove_file(&self.socket);
        // The directory is shared by this user's windows, so it is removed only
        // when this was the last one; a failure means another window is live.
        let _ = fs::remove_dir(&self.directory);
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.close();
    }
}

fn serve<H>(
    listener: &UnixListener,
    key: &Arc<ObservationKey>,
    running: &Arc<AtomicBool>,
    handler: &Arc<H>,
) where
    H: Fn(Request) -> String + Send + Sync + 'static,
{
    let in_flight = Arc::new(AtomicUsize::new(0));
    for connection in listener.incoming() {
        if !running.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = connection else { continue };
        // Serving on the accepting thread would let one client that connects
        // and says nothing hold the endpoint for the whole client timeout, so
        // every connection gets its own thread.
        if in_flight.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
            // Busy: dropped without being read, and without an answer that
            // would tell a caller anything about the window's state.
            drop(stream);
            continue;
        }
        let _ = stream.set_read_timeout(Some(CLIENT_TIMEOUT));
        let _ = stream.set_write_timeout(Some(CLIENT_TIMEOUT));

        in_flight.fetch_add(1, Ordering::SeqCst);
        let spawned = std::thread::Builder::new()
            .name("sprite-observation-request".to_owned())
            .spawn({
                let key = Arc::clone(key);
                let running = Arc::clone(running);
                let handler = Arc::clone(handler);
                let in_flight = Arc::clone(&in_flight);
                move || {
                    answer(stream, &key, &running, handler.as_ref());
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                }
            });
        if spawned.is_err() {
            in_flight.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn answer<H>(mut stream: UnixStream, key: &ObservationKey, running: &AtomicBool, handler: &H)
where
    H: Fn(Request) -> String,
{
    let mut line = String::new();
    let read = BufReader::new(&stream)
        .take(MAX_REQUEST_BYTES)
        .read_line(&mut line);
    if read.is_err() {
        let _ = writeln!(stream, "{DENIED}");
        return;
    }

    // The key is the first token; everything after it is the request. Splitting
    // before authenticating means the body is never parsed for an unauthorised
    // caller — it is not even looked at.
    let line = line.trim_end_matches(['\r', '\n']);
    let (presented, body) = match line.split_once(' ') {
        Some((presented, body)) => (presented, body),
        None => (line, ""),
    };

    // Closed while this request was in flight: the window is gone, so its key
    // is worthless from this moment rather than whenever the last thread
    // finishes.
    if !running.load(Ordering::SeqCst) || !key.matches(presented) {
        // No detail, and nothing about the request: a caller learns only that
        // it was refused.
        let _ = writeln!(stream, "{DENIED}");
        return;
    }

    let response = handler(Request {
        body: body.to_owned(),
    });
    let _ = writeln!(stream, "{response}");
}

/// The per-user runtime directory this window's socket lives in.
///
/// `XDG_RUNTIME_DIR` is a directory the system already guarantees is private to
/// one user and cleaned up on logout. There is deliberately no fall back to a
/// world-writable temporary directory: an endpoint nobody else can reach is the
/// whole point, so it is better to have no observation surface than one in a
/// place another user can reach.
fn runtime_directory() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
        std::io::Error::other(
            "XDG_RUNTIME_DIR is not set, so there is no private directory for the \
             observation socket; observation is unavailable rather than placed \
             somewhere other users could reach",
        )
    })?;
    Ok(PathBuf::from(base).join("sprite"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufWriter;
    use std::sync::Mutex;

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
        let mut answer = String::new();
        BufReader::new(&stream)
            .read_line(&mut answer)
            .expect("read the answer");
        answer.trim_end().to_owned()
    }

    /// Records what reached the handler, so "the handler was never called" is
    /// an assertion rather than an inference from the response text.
    #[derive(Default)]
    struct Calls(Mutex<Vec<String>>);

    fn endpoint_with_spy() -> (Endpoint, Arc<Calls>) {
        let calls: Arc<Calls> = Arc::default();
        let endpoint = Endpoint::open({
            let calls = Arc::clone(&calls);
            move |request| {
                calls.0.lock().expect("lock").push(request.body.clone());
                format!("ok:{}", request.body)
            }
        })
        .expect("open endpoint");
        (endpoint, calls)
    }

    #[test]
    fn two_windows_never_share_a_key_or_a_socket() {
        let (first, _) = endpoint_with_spy();
        let (second, _) = endpoint_with_spy();

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
            assert_eq!(hex.len(), KEY_BYTES * 2, "32 bytes of key");
            assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(seen.insert(hex), "a key repeated, so it is not random");
        }
    }

    #[test]
    fn the_socket_and_its_directory_are_private() {
        let (endpoint, _) = endpoint_with_spy();

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
        let (endpoint, calls) = endpoint_with_spy();

        let answer = ask(
            endpoint.socket_path(),
            &format!("{} panes snapshot", endpoint.key_hex()),
        );

        assert_eq!(answer, "ok:panes snapshot");
        assert_eq!(*calls.0.lock().expect("lock"), vec!["panes snapshot"]);
    }

    #[test]
    fn a_request_with_no_key_is_refused_without_reaching_the_handler() {
        let (endpoint, calls) = endpoint_with_spy();

        assert_eq!(ask(endpoint.socket_path(), ""), DENIED);
        assert_eq!(ask(endpoint.socket_path(), "panes snapshot"), DENIED);
        assert!(
            calls.0.lock().expect("lock").is_empty(),
            "an unauthorised request never reaches the handler at all"
        );
    }

    #[test]
    fn a_request_with_the_wrong_key_is_refused_without_reaching_the_handler() {
        let (endpoint, calls) = endpoint_with_spy();
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
        let endpoint = Endpoint::open(|_request| DENIED.to_owned()).expect("open endpoint");

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
        let (mut endpoint, _) = endpoint_with_spy();
        let captured_key = endpoint.key_hex();
        let path = endpoint.socket_path().to_path_buf();
        assert_eq!(
            ask(&path, &format!("{captured_key} panes snapshot")),
            "ok:panes snapshot"
        );

        endpoint.close();

        assert!(!path.exists(), "the socket is gone from the filesystem");
        let refused = UnixStream::connect(&path);
        assert!(
            refused.is_err(),
            "a captured key is worthless once the window has closed"
        );
    }

    #[test]
    fn a_session_is_told_the_socket_the_key_and_its_own_identity() {
        let (endpoint, _) = endpoint_with_spy();

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
        let (endpoint, _) = endpoint_with_spy();
        let silent = UnixStream::connect(endpoint.socket_path()).expect("connect");

        let answer = ask(
            endpoint.socket_path(),
            &format!("{} panes snapshot", endpoint.key_hex()),
        );
        assert_eq!(answer, "ok:panes snapshot");
        drop(silent);
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
        assert!(!key.matches(&"0".repeat(KEY_BYTES * 2)));
    }
}
