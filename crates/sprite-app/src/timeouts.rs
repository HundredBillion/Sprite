//! Every duration that stands in a relation to another, with the relation
//! checked when the crate compiles.
//!
//! Clients once gave up at 15 s while the window's "still applying" answer
//! could arrive at 5 s + 10 s measured from a later moment, so the client
//! never saw it. A client now always outlives the server's two waits.

use std::time::Duration;

/// How long a connection waits for the window to answer a request.
pub(crate) const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// How much longer an asker waits once the window has claimed its request.
///
/// Bounded, because a wedged GPUI thread must not pin an endpoint thread and
/// its connection slot forever. Generous, because by then the work is under
/// way and the only honest early answer is that it still is.
pub(crate) const AFTER_CLAIM: Duration = Duration::from_secs(10);

/// A client that connects and sends no first line for this long has its
/// thread taken back. Short under test so handshake tests finish quickly.
#[cfg(not(test))]
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(200);

/// A client that will not accept an event for this long is treated as gone.
/// Short under test so the dead-write test finishes quickly.
#[cfg(not(test))]
pub(crate) const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
pub(crate) const WRITE_TIMEOUT: Duration = Duration::from_millis(200);

/// The slowest a program may read its Surface events, in bytes per second.
/// A write timeout only catches a reader that stops completely. A reader
/// that takes a few bytes at a time, each sooner than the timeout, would
/// otherwise keep a writer thread and its connection slot for as long as it
/// kept reading. Fast under test, so the drain-floor test's one-megabyte
/// batch is due in a second.
#[cfg(not(test))]
pub(crate) const MIN_DRAIN_RATE: u64 = 64 * 1024;
#[cfg(test)]
pub(crate) const MIN_DRAIN_RATE: u64 = 1280 * 1024;

/// How long a Surface writer may spend on a batch of `bytes`: the write
/// timeout, plus the time the batch takes at `MIN_DRAIN_RATE`. A full 4 MiB
/// queue gets about 66 s.
pub(crate) const fn drain_deadline(bytes: usize) -> Duration {
    let millis = (bytes as u64).saturating_mul(1000) / MIN_DRAIN_RATE;
    WRITE_TIMEOUT.saturating_add(Duration::from_millis(millis))
}

/// How long the observation broker waits for every pane to answer.
pub(crate) const BROKER_DEADLINE: Duration = Duration::from_millis(500);

/// How long the observation endpoint tolerates a connected client that
/// sends nothing. Not a client's patience: the server's.
pub(crate) const SILENT_CLIENT_TIMEOUT: Duration = Duration::from_secs(2);

/// A client's patience for one exchange: the window's reply wait, its
/// after-claim wait, and a second for the socket.
pub(crate) const CLIENT_TIMEOUT: Duration = REPLY_TIMEOUT
    .saturating_add(AFTER_CLAIM)
    .saturating_add(Duration::from_secs(1));

/// Gives a one-exchange client socket `CLIENT_TIMEOUT` on every read and write.
pub(crate) fn bound_client(stream: &std::os::unix::net::UnixStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(CLIENT_TIMEOUT))?;
    stream.set_write_timeout(Some(CLIENT_TIMEOUT))
}

/// The read and write timeouts a client's `open` gives a stream it connects
/// to a fresh listening socket, which is removed before this returns.
///
/// The directory is named the way `local_socket`'s test scratch is, by pid
/// and a per-process counter in hex, so the socket path stays far below
/// macOS's 103-byte limit for a socket path.
#[cfg(test)]
pub(crate) fn exchange_timeouts(
    open: impl FnOnce(&std::path::Path) -> std::io::Result<std::os::unix::net::UnixStream>,
) -> std::io::Result<(Option<Duration>, Option<Duration>)> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let directory = std::env::temp_dir().join(format!(
        "sx-{:x}-{:x}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir(&directory)?;
    let socket = directory.join("w.sock");
    let timeouts = std::os::unix::net::UnixListener::bind(&socket).and_then(|_listener| {
        let stream = open(&socket)?;
        Ok((stream.read_timeout()?, stream.write_timeout()?))
    });
    std::fs::remove_dir_all(&directory)?;
    timeouts
}

const fn longer(a: Duration, b: Duration) -> bool {
    a.as_millis() > b.as_millis()
}

const _: () = assert!(
    longer(CLIENT_TIMEOUT, REPLY_TIMEOUT.saturating_add(AFTER_CLAIM)),
    "a client must outlive the window's reply and after-claim waits"
);
const _: () = assert!(longer(CLIENT_TIMEOUT, HANDSHAKE_TIMEOUT));
const _: () = assert!(longer(CLIENT_TIMEOUT, WRITE_TIMEOUT));
const _: () = assert!(longer(REPLY_TIMEOUT, BROKER_DEADLINE));
const _: () = assert!(longer(REPLY_TIMEOUT, SILENT_CLIENT_TIMEOUT));
const _: () = assert!(
    MIN_DRAIN_RATE > 0,
    "the drain deadline divides by the floor"
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Both clients bound their sockets through `bound_client`, so this checks
    /// the bound a client actually gets, not only the constants.
    #[test]
    fn a_client_outlives_the_windows_reply_and_after_claim_waits() -> std::io::Result<()> {
        let (stream, _peer) = std::os::unix::net::UnixStream::pair()?;
        bound_client(&stream)?;
        assert_eq!(stream.read_timeout()?, Some(CLIENT_TIMEOUT));
        assert_eq!(stream.write_timeout()?, Some(CLIENT_TIMEOUT));
        assert!(CLIENT_TIMEOUT > REPLY_TIMEOUT + AFTER_CLAIM);
        Ok(())
    }

    /// An empty batch gets only the write timeout. Each further floor's
    /// worth of bytes adds one second.
    #[test]
    fn a_batch_gets_the_write_timeout_plus_its_time_at_the_drain_floor() {
        assert_eq!(drain_deadline(0), WRITE_TIMEOUT);
        let one_second = usize::try_from(MIN_DRAIN_RATE).unwrap();
        assert_eq!(
            drain_deadline(3 * one_second),
            WRITE_TIMEOUT + Duration::from_secs(3)
        );
    }
}
