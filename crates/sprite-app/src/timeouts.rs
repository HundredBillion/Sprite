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
#[cfg(test)]
pub(crate) fn exchange_timeouts(
    open: impl FnOnce(&std::path::Path) -> std::io::Result<std::os::unix::net::UnixStream>,
) -> std::io::Result<(Option<Duration>, Option<Duration>)> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let directory =
        std::env::temp_dir().join(format!("sprite-exchange-{}-{nanos}", std::process::id()));
    std::fs::create_dir(&directory)?;
    let socket = directory.join("window.sock");
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
}
