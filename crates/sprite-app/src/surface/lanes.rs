//! Two lanes into the window for Surface requests.
//!
//! A connection streams Updates, Grid and List operations for as long as it
//! lives and blocks on the stream lane when the window is behind. The
//! one-shot requests — Open, Capabilities, FocusPane, RegisterToken — are
//! answered once and must not queue behind another program's stream, so they
//! have a lane of their own. Order within a connection holds because a
//! connection sends Open, waits for the answer, then streams.

use crate::surface::channel::SurfaceRequest;

/// One wave of streaming programs; a connection past it blocks, which is
/// its own thread's problem and nobody else's.
pub(crate) const STREAM_CAPACITY: usize = 64;

/// Handshakes in flight at once. Past this, a program is refused as busy
/// and reconnects; it never waits.
pub(crate) const ONE_SHOT_CAPACITY: usize = 16;

/// The connection threads' ends.
#[derive(Clone)]
pub struct SurfaceLanes {
    pub(crate) stream: async_channel::Sender<SurfaceRequest>,
    pub(crate) one_shot: async_channel::Sender<SurfaceRequest>,
}

/// The window's ends, each drained by its own task.
///
/// Public, with `surface_lanes`, so a test outside the crate can play the
/// window.
pub struct SurfaceReceivers {
    pub stream: async_channel::Receiver<SurfaceRequest>,
    pub one_shot: async_channel::Receiver<SurfaceRequest>,
}

pub fn surface_lanes() -> (SurfaceLanes, SurfaceReceivers) {
    surface_lanes_with(STREAM_CAPACITY, ONE_SHOT_CAPACITY)
}

pub(crate) fn surface_lanes_with(
    stream: usize,
    one_shot: usize,
) -> (SurfaceLanes, SurfaceReceivers) {
    let (stream_tx, stream_rx) = async_channel::bounded(stream);
    let (one_shot_tx, one_shot_rx) = async_channel::bounded(one_shot);
    (
        SurfaceLanes {
            stream: stream_tx,
            one_shot: one_shot_tx,
        },
        SurfaceReceivers {
            stream: stream_rx,
            one_shot: one_shot_rx,
        },
    )
}
