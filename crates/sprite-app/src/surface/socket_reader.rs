//! Reads from a Surface's sole socket reader while another clone writes events.

use std::io::{self, Read};
use std::os::unix::net::UnixStream;

pub(super) struct SocketReader(pub(super) UnixStream);

/// How much unread data the kernel holds for an open Surface, and how much
/// of it one read takes. A Unix socket's default is a few kilobytes, so a
/// megabyte of list rows arrived in over a hundred reads, the sender waking
/// for each; with room for the whole message it arrives in a handful.
pub(super) const RECEIVE_BUFFER: usize = 1024 * 1024;
pub(super) const READ_BUFFER: usize = 64 * 1024;

/// Asks for `RECEIVE_BUFFER` on `stream`. The system may grant less or
/// refuse; the Surface works either way, only slower for large messages.
pub(super) fn widen_receive_buffer(stream: &UnixStream) {
    use std::os::fd::AsRawFd;
    let size = RECEIVE_BUFFER as libc::c_int;
    // SAFETY: `stream` owns an open descriptor for the length of this call,
    // and the option value is a `c_int` that outlives it, passed with its size.
    unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            std::ptr::from_ref(&size).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
}

impl Read for SocketReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        #[cfg(target_os = "macos")]
        {
            use nix::errno::Errno;
            use nix::poll::{PollFd, PollFlags, poll};
            use std::os::fd::AsFd;

            // Darwin can leave a blocking recv asleep after the peer half-closes.
            // Poll readiness first; the finite wait rechecks without expiring an idle Surface.
            loop {
                let mut descriptors = [PollFd::new(self.0.as_fd(), PollFlags::POLLIN)];
                match poll(&mut descriptors, 1000_u16) {
                    Ok(0) | Err(Errno::EINTR) => continue,
                    Ok(_) => break,
                    Err(error) => return Err(error.into()),
                }
            }
        }
        self.0.read(buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::Shutdown;
    use std::sync::mpsc;

    #[test]
    fn a_half_close_drains_buffered_lines_and_keeps_the_reply_direction_open() {
        let (socket, mut peer) = UnixStream::pair().unwrap();
        let mut writer = socket.try_clone().unwrap();
        let (done, finished) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let lines = BufReader::new(SocketReader(socket))
                .lines()
                .collect::<io::Result<Vec<_>>>()
                .unwrap();
            writer.write_all(b"closed\n").unwrap();
            done.send(lines).unwrap();
        });
        peer.write_all(b"first\nsecond\n").unwrap();
        peer.shutdown(Shutdown::Write).unwrap();
        assert_eq!(
            finished
                .recv_timeout(crate::test_blocking_wait::watchdog())
                .unwrap(),
            ["first", "second"]
        );
        let mut answer = String::new();
        peer.read_to_string(&mut answer).unwrap();
        assert_eq!(answer, "closed\n");
        thread.join().unwrap();
    }

    #[test]
    fn an_idle_surface_can_receive_data_after_a_poll_interval() {
        let (socket, mut peer) = UnixStream::pair().unwrap();
        let (started, ready) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut reader = SocketReader(socket);
            started.send(()).unwrap();
            let mut bytes = Vec::new();
            let result = reader.read_to_end(&mut bytes);
            done.send((result, bytes)).unwrap();
        });
        ready
            .recv_timeout(crate::test_blocking_wait::watchdog())
            .unwrap();
        assert!(matches!(
            finished.recv_timeout(std::time::Duration::from_millis(1100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        peer.write_all(b"still open").unwrap();
        peer.shutdown(Shutdown::Write).unwrap();
        let (result, bytes) = finished
            .recv_timeout(crate::test_blocking_wait::watchdog())
            .unwrap();
        assert_eq!(result.unwrap(), 10);
        assert_eq!(bytes, b"still open");
        thread.join().unwrap();
    }

    #[test]
    fn an_empty_read_does_not_wait_for_the_peer() {
        let (socket, _peer) = UnixStream::pair().unwrap();
        assert_eq!(SocketReader(socket).read(&mut []).unwrap(), 0);
    }
}
