//! Private local transport shared by independent protocol adapters.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read};
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const KEY_BYTES: usize = 32;

/// The longest socket path this platform can actually carry.
///
/// A Unix socket address keeps its path in `sockaddr_un.sun_path`, which is 108
/// bytes on Linux and 104 on macOS, NUL terminator included — so the usable
/// length is one less than the array.
///
/// This was a flat 100 on every platform, which is shorter than either, and the
/// difference was not academic. macOS `$TMPDIR` is itself a ~49-byte path under
/// `/var/folders`, so a socket nested one directory deeper than the product
/// nests it exceeded 100 — which is exactly what the endpoint tests do, and why
/// every one of them failed on macOS while passing on Linux. A limit that
/// refuses paths the platform would accept is not a safety margin; it is a bug
/// that only shows up where the base path is long.
///
/// Deliberately not read from `libc`: that would be a new direct dependency,
/// and a third-party notice, for two integers each platform's headers fix.
#[cfg(target_os = "macos")]
pub(crate) const MAX_SOCKET_PATH: usize = 103;
#[cfg(not(target_os = "macos"))]
pub(crate) const MAX_SOCKET_PATH: usize = 107;

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
        // Keep the key wipe observable to the optimizer without unsafe memory writes.
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

#[derive(Clone, Copy)]
pub(crate) struct TransportPolicy {
    pub name: &'static str,
    pub filename_hex: usize,
    pub suffix: &'static str,
    pub max_connections: usize,
    pub max_first_line: usize,
    pub handshake_timeout: Duration,
    pub write_timeout: Duration,
}

pub(crate) struct Authenticated {
    pub stream: UnixStream,
    // A pipelined Surface message may already be buffered after the key line.
    pub reader: BufReader<UnixStream>,
    pub body: String,
    pub reply_connection: ReplyConnection,
}

type Connections = Arc<Mutex<HashMap<usize, UnixStream>>>;

/// Only the authenticated origin may finish a bounded one-shot reply during closure.
#[derive(Clone, Debug)]
pub(crate) struct ReplyConnection {
    connections: Connections,
    id: usize,
}

struct ConnectionSlot {
    connections: Connections,
    id: usize,
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.connections.lock().unwrap().remove(&self.id);
    }
}

pub(crate) struct LocalSocket {
    socket: PathBuf,
    key: Arc<ObservationKey>,
    running: Arc<AtomicBool>,
    connections: Connections,
    listener: Option<JoinHandle<()>>,
}

impl LocalSocket {
    pub fn open_in<H>(
        directory: PathBuf,
        key: Arc<ObservationKey>,
        policy: TransportPolicy,
        refused: fn(&mut UnixStream),
        handler: H,
    ) -> io::Result<Self>
    where
        H: Fn(Authenticated) + Send + Sync + 'static,
    {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        sweep_dead_sockets(&directory);
        let mut name = ObservationKey::generate()?.to_hex();
        name.truncate(policy.filename_hex);
        let socket = directory.join(format!("{name}{}", policy.suffix));
        let listener = bind_private(&socket)?;
        let running = Arc::new(AtomicBool::new(true));
        let connections: Connections = Arc::default();
        let thread = std::thread::Builder::new()
            .name(policy.name.to_owned())
            .spawn({
                let running = Arc::clone(&running);
                let key = Arc::clone(&key);
                let connections = Arc::clone(&connections);
                let handler = Arc::new(handler);
                move || {
                    for (id, connection) in listener.incoming().enumerate() {
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }
                        let Ok(stream) = connection else { continue };
                        let deadline = Instant::now() + policy.handshake_timeout;
                        let mut live = connections.lock().unwrap();
                        if live.len() >= policy.max_connections {
                            continue;
                        }
                        let Ok(cancel) = stream.try_clone() else {
                            continue;
                        };
                        live.insert(id, cancel);
                        drop(live);
                        let slot = ConnectionSlot {
                            connections: Arc::clone(&connections),
                            id,
                        };
                        let running = Arc::clone(&running);
                        let key = Arc::clone(&key);
                        let handler = Arc::clone(&handler);
                        // The slot also drops if spawning fails or an adapter panics.
                        let _ = std::thread::Builder::new()
                            .name(format!("{}-connection", policy.name))
                            .spawn(move || {
                                let _slot = slot;
                                let mut stream = stream;
                                if stream
                                    .set_write_timeout(Some(policy.write_timeout))
                                    .is_err()
                                {
                                    return;
                                }
                                match authenticate(&stream, &key, &running, policy, deadline) {
                                    Ok((reader, body)) => handler(Authenticated {
                                        stream,
                                        reader,
                                        body,
                                        reply_connection: ReplyConnection {
                                            connections: Arc::clone(&_slot.connections),
                                            id,
                                        },
                                    }),
                                    Err(_) => refused(&mut stream),
                                }
                            });
                    }
                }
            });
        let thread = match thread {
            Ok(thread) => thread,
            Err(error) => {
                let _ = fs::remove_file(&socket);
                return Err(error);
            }
        };
        Ok(Self {
            socket,
            key,
            running,
            connections,
            listener: Some(thread),
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }
    pub fn key_hex(&self) -> String {
        self.key.to_hex()
    }

    pub fn close(&mut self) {
        self.close_after_reply(None);
    }

    /// Stop authentication and cancel peers while the initiating one-shot writes its reply.
    /// A registry-scoped identity cannot exempt a connection on a later socket.
    pub fn close_after_reply(&mut self, reply: Option<&ReplyConnection>) {
        if self.listener.is_none() {
            return;
        }
        self.running.store(false, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.socket);
        if let Some(thread) = self.listener.take() {
            let _ = thread.join();
        }
        for (id, stream) in self.connections.lock().unwrap().iter() {
            let finishing_reply = reply.is_some_and(|reply| {
                Arc::ptr_eq(&reply.connections, &self.connections) && reply.id == *id
            });
            let _ = stream.shutdown(if finishing_reply {
                Shutdown::Read
            } else {
                Shutdown::Both
            });
        }
        let _ = fs::remove_file(&self.socket);
        // Other windows may be between directory creation and binding their socket.
        // Leaving the shared directory avoids racing their startup.
    }
}

impl Drop for LocalSocket {
    fn drop(&mut self) {
        self.close();
    }
}

pub(crate) fn bind_private(socket: &Path) -> io::Result<UnixListener> {
    let length = socket.as_os_str().len();
    if length > MAX_SOCKET_PATH {
        return Err(io::Error::other(format!(
            "the local socket path is {length} bytes and this platform's sockaddr_un holds {MAX_SOCKET_PATH}: {}",
            socket.display()
        )));
    }
    let listener = UnixListener::bind(socket)?;
    if let Err(error) = fs::set_permissions(socket, fs::Permissions::from_mode(0o600)) {
        drop(listener);
        let _ = fs::remove_file(socket);
        return Err(error);
    }
    Ok(listener)
}

fn authenticate(
    stream: &UnixStream,
    key: &ObservationKey,
    running: &AtomicBool,
    policy: TransportPolicy,
    deadline: Instant,
) -> io::Result<(BufReader<UnixStream>, String)> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = Vec::new();
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "handshake deadline"))?;
        if !running.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        // Socket timeouts apply per read, so every read uses the remaining total budget.
        stream.set_read_timeout(Some(remaining))?;
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let newline = bytes.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(bytes.len(), |index| index + 1);
        if count > policy.max_first_line - line.len() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        line.extend_from_slice(&bytes[..count]);
        reader.consume(count);
        if newline.is_some() {
            break;
        }
        if line.len() == policy.max_first_line {
            return Err(io::ErrorKind::InvalidData.into());
        }
    }
    let line = std::str::from_utf8(&line).map_err(|_| io::ErrorKind::InvalidData)?;
    let line = line.trim_end_matches(['\r', '\n']);
    let (presented, body) = line.split_once(' ').unwrap_or((line, ""));
    if Instant::now() >= deadline || !running.load(Ordering::SeqCst) || !key.matches(presented) {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    stream.set_read_timeout(None)?;
    Ok((reader, body.to_owned()))
}

/// Removes socket files in `directory` that nothing is listening on.
///
/// A socket with a live window behind it accepts a connection and is left
/// alone; only one that refuses is removed. That is what makes this safe to run
/// while other windows are open.
pub(crate) fn sweep_dead_sockets(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "sock") {
            continue;
        }
        // Connecting is the test, and a refusal is the strongest evidence
        // available rather than proof: a listener that is gone refuses, but so
        // does a live listener whose accept backlog is full — something a
        // window that accepts and drops connections never lets happen in
        // practice. A window that is alive accepts, and is not disturbed by a
        // connection that is immediately dropped. Any other error — a machine
        // out of descriptors, a path the socket layer cannot address — proves
        // nothing, and a live window's socket is worth more than a tidy
        // directory.
        if let Err(error) = UnixStream::connect(&path)
            && error.kind() == std::io::ErrorKind::ConnectionRefused
        {
            let _ = fs::remove_file(&path);
        }
    }
}

/// The per-user runtime directory this window's socket lives in.
///
/// On Linux, `XDG_RUNTIME_DIR`: a directory the system already guarantees is
/// private to one user and cleaned up on logout. There is deliberately no fall
/// back to a world-writable temporary directory: an endpoint nobody else can
/// reach is the whole point, so it is better to have no observation surface
/// than one in a place another user can reach.
#[cfg(not(target_os = "macos"))]
pub(crate) fn runtime_directory() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
        std::io::Error::other(
            "XDG_RUNTIME_DIR is not set, so there is no private directory for the \
             observation socket; observation is unavailable rather than placed \
             somewhere other users could reach",
        )
    })?;
    Ok(PathBuf::from(base).join("sprite"))
}

/// The per-user runtime directory this window's socket lives in.
///
/// macOS has no `XDG_RUNTIME_DIR`, and the equivalent guarantee lives
/// elsewhere: `TMPDIR` there is a per-user directory under `/var/folders`,
/// created by the system and readable only by its owner — the same property
/// `XDG_RUNTIME_DIR` is chosen for on Linux. Without this, observation would
/// simply not exist on macOS, which is not parity.
///
/// `TMPDIR` is honoured because that is where the system publishes the path;
/// the mode is not taken on trust either way, since [`LocalSocket::open_in`]
/// applies 0700 to the directory it is given before the socket exists.
#[cfg(target_os = "macos")]
pub(crate) fn runtime_directory() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("TMPDIR").ok_or_else(|| {
        std::io::Error::other(
            "TMPDIR is not set, so there is no private directory for the \
             observation socket; observation is unavailable rather than placed \
             somewhere other users could reach",
        )
    })?;
    Ok(PathBuf::from(base).join("sprite"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::mpsc;

    const POLICY: TransportPolicy = TransportPolicy {
        name: "sprite-transport-test",
        filename_hex: 24,
        suffix: ".sock",
        max_connections: 2,
        max_first_line: 128,
        handshake_timeout: Duration::from_millis(200),
        write_timeout: Duration::from_millis(200),
    };

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            Self(std::env::temp_dir().join(format!(
                "ls-{:x}-{:x}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            )))
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn rejected(stream: &mut UnixStream) {
        let _ = writeln!(stream, "denied");
        let _ = stream.shutdown(Shutdown::Write);
    }

    fn wait_for_count(socket: &LocalSocket, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while socket.connections.lock().unwrap().len() != count {
            assert!(
                Instant::now() < deadline,
                "connection count did not reach {count}"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn authentication_requires_a_complete_bounded_utf8_line_and_the_exact_key() {
        let key = ObservationKey::generate().unwrap();
        let running = AtomicBool::new(true);
        for line in [
            "\n".to_owned(),
            "wrong hello\n".to_owned(),
            format!("{} hello", key.to_hex()),
            format!("{} {}\n", key.to_hex(), "x".repeat(POLICY.max_first_line)),
            format!("{} hello\n", "a".repeat(65)),
        ] {
            let (mut client, server) = UnixStream::pair().unwrap();
            client.write_all(line.as_bytes()).unwrap();
            client.shutdown(Shutdown::Write).unwrap();
            assert!(
                authenticate(
                    &server,
                    &key,
                    &running,
                    POLICY,
                    Instant::now() + POLICY.handshake_timeout
                )
                .is_err()
            );
        }
        let (mut client, server) = UnixStream::pair().unwrap();
        client.write_all(&[0xff, b'\n']).unwrap();
        assert!(
            authenticate(
                &server,
                &key,
                &running,
                POLICY,
                Instant::now() + POLICY.handshake_timeout
            )
            .is_err()
        );
    }

    #[test]
    fn the_exact_line_limit_is_accepted_and_buffered_followup_bytes_survive() {
        let key = ObservationKey::generate().unwrap();
        let (mut client, server) = UnixStream::pair().unwrap();
        let body = "x".repeat(POLICY.max_first_line - 66);
        write!(client, "{} {body}\nnext\n", key.to_hex()).unwrap();
        let (mut reader, parsed) = authenticate(
            &server,
            &key,
            &AtomicBool::new(true),
            POLICY,
            Instant::now() + POLICY.handshake_timeout,
        )
        .unwrap();
        assert_eq!(parsed, body);
        assert_eq!(reader.get_ref().read_timeout().unwrap(), None);
        let mut next = String::new();
        reader.read_line(&mut next).unwrap();
        assert_eq!(next, "next\n");
    }

    #[test]
    fn a_trickling_client_cannot_restart_the_handshake_deadline() {
        let key = ObservationKey::generate().unwrap();
        let (mut client, server) = UnixStream::pair().unwrap();
        let (stop, stopped) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            while stopped.recv_timeout(Duration::from_millis(20)).is_err() {
                if client.write_all(b"a").is_err() {
                    break;
                }
            }
        });
        let start = Instant::now();
        let result = authenticate(
            &server,
            &key,
            &AtomicBool::new(true),
            POLICY,
            start + POLICY.handshake_timeout,
        );
        let elapsed = start.elapsed();
        stop.send(()).unwrap();
        writer.join().unwrap();
        assert!(result.is_err());
        assert!(elapsed >= POLICY.handshake_timeout);
        assert!(
            elapsed < Duration::from_secs(1),
            "deadline was renewed: {elapsed:?}"
        );
    }

    #[test]
    fn a_silent_client_and_a_cancelled_key_never_authenticate() {
        let key = ObservationKey::generate().unwrap();
        let (_client, server) = UnixStream::pair().unwrap();
        assert!(
            authenticate(
                &server,
                &key,
                &AtomicBool::new(true),
                POLICY,
                Instant::now() + POLICY.handshake_timeout
            )
            .is_err()
        );
        let (mut client, server) = UnixStream::pair().unwrap();
        writeln!(client, "{} hello", key.to_hex()).unwrap();
        assert!(
            authenticate(
                &server,
                &key,
                &AtomicBool::new(false),
                POLICY,
                Instant::now() + POLICY.handshake_timeout
            )
            .is_err()
        );
    }

    #[test]
    fn the_cap_covers_authenticated_connections_and_releases_after_completion() {
        let scratch = Scratch::new();
        let (entered, entry) = mpsc::channel();
        let mut socket = LocalSocket::open_in(
            scratch.0.clone(),
            Arc::new(ObservationKey::generate().unwrap()),
            POLICY,
            rejected,
            move |mut connection| {
                entered.send(()).unwrap();
                let mut done = String::new();
                let _ = connection.reader.read_line(&mut done);
            },
        )
        .unwrap();
        let mut clients = Vec::new();
        for _ in 0..POLICY.max_connections {
            let mut stream = UnixStream::connect(socket.socket_path()).unwrap();
            writeln!(stream, "{} hello", socket.key_hex()).unwrap();
            entry.recv_timeout(Duration::from_secs(2)).unwrap();
            clients.push(stream);
        }
        let mut excess = UnixStream::connect(socket.socket_path()).unwrap();
        excess
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        assert_eq!(excess.read(&mut [0]).unwrap(), 0);
        clients.pop().unwrap().shutdown(Shutdown::Both).unwrap();
        wait_for_count(&socket, 1);
        let mut next = UnixStream::connect(socket.socket_path()).unwrap();
        writeln!(next, "{} hello", socket.key_hex()).unwrap();
        entry.recv_timeout(Duration::from_secs(2)).unwrap();
        socket.close();
        wait_for_count(&socket, 0);
    }

    #[test]
    fn silent_handshakes_are_counted_against_the_connection_cap() {
        let scratch = Scratch::new();
        let policy = TransportPolicy {
            handshake_timeout: Duration::from_secs(30),
            ..POLICY
        };
        let mut socket = LocalSocket::open_in(
            scratch.0.clone(),
            Arc::new(ObservationKey::generate().unwrap()),
            policy,
            rejected,
            |_| panic!("unauthenticated"),
        )
        .unwrap();
        let _clients: Vec<_> = (0..policy.max_connections)
            .map(|_| UnixStream::connect(socket.socket_path()).unwrap())
            .collect();
        wait_for_count(&socket, policy.max_connections);
        let mut excess = UnixStream::connect(socket.socket_path()).unwrap();
        excess
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        assert_eq!(excess.read(&mut [0]).unwrap(), 0);
        socket.close();
        wait_for_count(&socket, 0);
    }

    #[test]
    fn closing_interrupts_silent_handshakes_and_removes_only_the_socket() {
        let scratch = Scratch::new();
        let policy = TransportPolicy {
            handshake_timeout: Duration::from_secs(30),
            ..POLICY
        };
        let mut socket = LocalSocket::open_in(
            scratch.0.clone(),
            Arc::new(ObservationKey::generate().unwrap()),
            policy,
            rejected,
            |_| panic!("unauthenticated"),
        )
        .unwrap();
        let path = socket.socket_path().to_owned();
        let mut client = UnixStream::connect(&path).unwrap();
        // macOS can reject changing the read timeout after the peer has shut down.
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        wait_for_count(&socket, 1);
        socket.close();
        socket.close();
        let mut result = String::new();
        client.read_to_string(&mut result).unwrap();
        assert!(result.is_empty() || result == "denied\n");
        wait_for_count(&socket, 0);
        assert!(!path.exists());
        assert!(scratch.0.exists());
    }

    fn gated_reply_socket(
        directory: &Path,
    ) -> (
        LocalSocket,
        mpsc::Receiver<ReplyConnection>,
        mpsc::Sender<()>,
    ) {
        let (entered, identities) = mpsc::channel();
        let (finish, finished) = mpsc::channel();
        let finished = Mutex::new(finished);
        let socket = LocalSocket::open_in(
            directory.to_owned(),
            Arc::new(ObservationKey::generate().unwrap()),
            POLICY,
            rejected,
            move |mut connection| {
                assert_eq!(
                    connection.stream.write_timeout().unwrap(),
                    Some(POLICY.write_timeout)
                );
                entered.send(connection.reply_connection).unwrap();
                finished
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
                let _ = writeln!(connection.stream, "finished");
            },
        )
        .unwrap();
        (socket, identities, finish)
    }

    fn authenticated_client(socket: &LocalSocket) -> UnixStream {
        let mut client = UnixStream::connect(socket.socket_path()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        writeln!(client, "{} reply", socket.key_hex()).unwrap();
        client
    }

    #[test]
    fn closing_for_one_reply_cancels_other_clients_and_preserves_write_timeout() {
        let scratch = Scratch::new();
        let (entered, identities) = mpsc::channel();
        let (finish, finished) = mpsc::channel();
        let finished = Mutex::new(finished);
        let policy = TransportPolicy {
            max_connections: 3,
            handshake_timeout: Duration::from_secs(30),
            ..POLICY
        };
        let mut socket = LocalSocket::open_in(
            scratch.0.clone(),
            Arc::new(ObservationKey::generate().unwrap()),
            policy,
            rejected,
            move |mut connection| {
                assert_eq!(
                    connection.stream.write_timeout().unwrap(),
                    Some(POLICY.write_timeout)
                );
                entered.send(connection.reply_connection).unwrap();
                if connection.body == "reply" {
                    finished
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(2))
                        .unwrap();
                    writeln!(connection.stream, "finished").unwrap();
                } else {
                    let _ = connection.reader.read_line(&mut String::new());
                }
            },
        )
        .unwrap();
        let mut client = authenticated_client(&socket);
        let origin = identities.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut unrelated = UnixStream::connect(socket.socket_path()).unwrap();
        unrelated
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        writeln!(unrelated, "{} unrelated", socket.key_hex()).unwrap();
        identities.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut partial = UnixStream::connect(socket.socket_path()).unwrap();
        partial
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        write!(partial, "{} unfinished", socket.key_hex()).unwrap();
        wait_for_count(&socket, 3);
        socket.close_after_reply(Some(&origin));
        assert!(!socket.socket_path().exists());
        assert!(UnixStream::connect(socket.socket_path()).is_err());
        let mut answer = String::new();
        unrelated.read_to_string(&mut answer).unwrap();
        assert!(answer.is_empty());
        if let Err(error) = partial.read_to_string(&mut answer) {
            assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
        }
        assert!(answer.is_empty() || answer == "denied\n");
        wait_for_count(&socket, 1);
        finish.send(()).unwrap();
        let mut answer = String::new();
        client.read_to_string(&mut answer).unwrap();
        assert_eq!(answer, "finished\n");
        wait_for_count(&socket, 0);
    }

    #[test]
    fn foreign_and_reopened_reply_identities_cannot_spare_another_connection() {
        let source_dir = Scratch::new();
        let target_dir = Scratch::new();
        let (mut source, identities, finish_source) = gated_reply_socket(&source_dir.0);
        let _source_client = authenticated_client(&source);
        let foreign = identities.recv_timeout(Duration::from_secs(2)).unwrap();
        let (mut target, identities, finish_target) = gated_reply_socket(&target_dir.0);
        let mut target_client = authenticated_client(&target);
        let old_target = identities.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(foreign.id, old_target.id);
        target.close_after_reply(Some(&foreign));
        assert_eq!(target_client.read(&mut [0]).unwrap(), 0);
        finish_target.send(()).unwrap();
        wait_for_count(&target, 0);
        drop(target);
        let (mut reopened, identities, finish_reopened) = gated_reply_socket(&target_dir.0);
        let mut reopened_client = authenticated_client(&reopened);
        let new_target = identities.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(old_target.id, new_target.id);
        reopened.close_after_reply(Some(&old_target));
        assert_eq!(reopened_client.read(&mut [0]).unwrap(), 0);
        finish_reopened.send(()).unwrap();
        wait_for_count(&reopened, 0);
        source.close();
        finish_source.send(()).unwrap();
        wait_for_count(&source, 0);
    }

    #[test]
    fn completed_reply_identity_and_ordinary_close_never_spare_active_clients() {
        let scratch = Scratch::new();
        let (mut socket, identities, finish) = gated_reply_socket(&scratch.0);
        let mut first = authenticated_client(&socket);
        let stale = identities.recv_timeout(Duration::from_secs(2)).unwrap();
        finish.send(()).unwrap();
        first.read_to_string(&mut String::new()).unwrap();
        wait_for_count(&socket, 0);
        let mut second = authenticated_client(&socket);
        let active = identities.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_ne!(stale.id, active.id);
        socket.close_after_reply(Some(&stale));
        assert_eq!(second.read(&mut [0]).unwrap(), 0);
        finish.send(()).unwrap();
        wait_for_count(&socket, 0);

        let (mut socket, identities, finish) = gated_reply_socket(&scratch.0);
        let mut client = authenticated_client(&socket);
        let _active = identities.recv_timeout(Duration::from_secs(2)).unwrap();
        socket.close();
        assert_eq!(client.read(&mut [0]).unwrap(), 0);
        finish.send(()).unwrap();
        wait_for_count(&socket, 0);
    }

    #[test]
    fn panicking_adapters_return_their_connection_slot() {
        let scratch = Scratch::new();
        let (entered, entry) = mpsc::channel();
        let socket = LocalSocket::open_in(
            scratch.0.clone(),
            Arc::new(ObservationKey::generate().unwrap()),
            POLICY,
            rejected,
            move |_| {
                entered.send(()).unwrap();
                panic!("adapter failed");
            },
        )
        .unwrap();
        let mut client = UnixStream::connect(socket.socket_path()).unwrap();
        writeln!(client, "{} hello", socket.key_hex()).unwrap();
        entry.recv_timeout(Duration::from_secs(2)).unwrap();
        wait_for_count(&socket, 0);
    }
}
