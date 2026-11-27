//! Sprite application shell.
//!
//! Owns the GPUI window, tabs, and Panes, and drives Terminal Sessions through
//! the public `sprite-term` interface. It never reaches past that seam to
//! libghostty or the PTY.

mod block_elements;
mod box_drawing;
mod cli;
mod config;
pub mod control;
#[doc(hidden)]
pub mod diagnostics;
mod graphics_cache;
mod grid;
mod grid_paint;
mod input;
mod local_socket;
mod locked;
mod observation;
#[doc(hidden)]
pub mod paint_benchmark;
mod pane_tree;
mod picture;
mod surface;
mod tabs;
mod terminal_events;
mod terminal_view;
#[cfg(test)]
mod test_blocking_wait;
#[cfg(test)]
mod test_child;
#[cfg(test)]
mod test_event_pressure;
mod timeouts;
mod tokens;
mod workspace;

pub use cli::{Invocation, SurfaceOpenArgs, TokenRegisterArgs, USAGE, WindowArgs, parse_arguments};
pub use config::Settings;
pub use control::client::{run_config_print, run_config_reload};
pub use local_socket::SocketKey;
pub use observation::broker::{
    Failure, FailureKind, PaneAddress, PaneReport, PaneSource, Pending, Report, Withdraw,
    collect as collect_panes,
};
pub use observation::schema::render as render_schema;
pub use pane_tree::{PaneId, Rect};
pub use sprite_term::HistoryLines;
pub use sprite_term::Rgb;
pub use surface::Refusal as SurfaceRefusal;
pub use surface::channel::{
    Position as SurfacePosition, ReturnTarget as SurfaceReturnTarget, Side as SurfaceSide,
    SurfaceEndpoint, SurfaceRequest,
};
pub use surface::client::{run_surface_focus, run_surface_open, run_token_register};
pub use surface::lanes::{SurfaceLanes, SurfaceReceivers, surface_lanes};
pub use tabs::TabId;
pub use workspace::Workspace;

pub use surface::channel::{
    Open as SurfaceOpen, Ownership as SurfaceOwnership, Placement as SurfacePlacement,
};

#[cfg(test)]
mod surface_performance;

#[cfg(any(test, feature = "startup-benchmark"))]
#[doc(hidden)]
pub mod startup_benchmark;

#[cfg(feature = "minimap-benchmark")]
mod minimap_benchmark;

#[doc(hidden)]
pub mod ipc_test_support {
    use std::io::{self, Read, Write};
    use std::path::Path;
    use std::time::{Duration, Instant};

    use crate::local_socket::{self, LocalStream};
    #[cfg(windows)]
    use crate::local_socket::{LocalListener, WakeReceiver, WakeSender, wake_pair};

    pub fn create_private_dir(directory: &Path) -> io::Result<()> {
        local_socket::create_private_dir(directory)
    }

    pub struct TestStream(LocalStream);

    impl TestStream {
        pub fn try_clone(&self) -> io::Result<Self> {
            self.0.try_clone().map(Self)
        }

        pub fn set_read_timeout(&self, duration: Option<Duration>) -> io::Result<()> {
            self.0.set_read_timeout(duration)
        }
    }

    impl Read for TestStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Read for &TestStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            (&self.0).read(buf)
        }
    }

    impl Write for TestStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            (&self.0).flush()
        }
    }

    impl Write for &TestStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            (&self.0).write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            (&self.0).flush()
        }
    }

    pub struct TestListener {
        #[cfg(unix)]
        inner: std::os::unix::net::UnixListener,
        #[cfg(windows)]
        inner: LocalListener,
        #[cfg(windows)]
        wake: WakeSender,
        #[cfg(windows)]
        cancel: WakeReceiver,
    }

    impl TestListener {
        pub fn bind(path: &Path) -> io::Result<Self> {
            #[cfg(unix)]
            {
                Ok(Self {
                    inner: std::os::unix::net::UnixListener::bind(path)?,
                })
            }
            #[cfg(windows)]
            {
                let inner = LocalListener::bind(path)?;
                let (wake, cancel) = wake_pair()?;
                Ok(Self {
                    inner,
                    wake,
                    cancel,
                })
            }
        }

        pub fn accept(&self) -> io::Result<TestStream> {
            #[cfg(unix)]
            {
                self.inner.accept().map(|(stream, _)| TestStream(stream))
            }
            #[cfg(windows)]
            {
                self.inner
                    .accept_cancellable(&self.cancel)?
                    .map(TestStream)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::Interrupted, "listener closed"))
            }
        }

        pub fn accept_within(&self, limit: Duration) -> io::Result<Option<TestStream>> {
            #[cfg(unix)]
            {
                self.inner.set_nonblocking(true)?;
                let deadline = Instant::now() + limit;
                loop {
                    match self.inner.accept() {
                        Ok((stream, _)) => {
                            stream.set_nonblocking(false)?;
                            return Ok(Some(TestStream(stream)));
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            if Instant::now() >= deadline {
                                return Ok(None);
                            }
                            std::thread::park_timeout(Duration::from_millis(10));
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
            #[cfg(windows)]
            {
                let deadline = Instant::now() + limit;
                loop {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Ok(None);
                    }
                    if let Some(stream) = self.inner.accept_timeout(&self.cancel, remaining)? {
                        return Ok(Some(TestStream(stream)));
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    impl Drop for TestListener {
        fn drop(&mut self) {
            self.wake.wake();
        }
    }
}
