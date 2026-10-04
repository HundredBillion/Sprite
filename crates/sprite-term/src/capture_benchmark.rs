//! A synchronous production projection seam, without a PTY or worker thread.

use crate::{SessionError, SnapshotBundle, TerminalSize, ValidTerminalSize, snapshot::Projector};
use libghostty_vt::{Terminal, terminal::Options};

pub struct CaptureBenchmark {
    projector: Projector<'static>,
    terminal: Terminal<'static, 'static>,
    size: ValidTerminalSize,
}

impl CaptureBenchmark {
    pub fn new() -> Result<Self, SessionError> {
        let size = ValidTerminalSize::new(
            TerminalSize {
                rows: 100,
                cols: 100,
                cell_width_px: 8,
                cell_height_px: 16,
            },
            "benchmark",
        )?;
        let mut terminal = Terminal::new(Options {
            rows: size.rows(),
            cols: size.cols(),
            max_scrollback: 1024 * 1024,
        })
        .map_err(|error| SessionError::new("benchmark_terminal", error))?;
        for row in 1..=100 {
            terminal.vt_write(
                format!("\x1b[{row};1HASCII é e\u{301} 界 \x1b[44m    \x1b[0m").as_bytes(),
            );
        }
        Ok(Self {
            projector: Projector::new()?,
            terminal,
            size,
        })
    }

    pub fn capture(&mut self) -> Result<SnapshotBundle, SessionError> {
        self.projector.capture(1, self.size, false, &self.terminal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_capture_does_not_allocate_per_observation_row() {
        let mut driver = CaptureBenchmark::new().unwrap();
        std::hint::black_box(driver.capture().unwrap());
        let (_, sample) = crate::test_allocations::measure(|| {
            std::hint::black_box(driver.capture().unwrap());
        });
        assert!(sample.allocations <= 8, "{sample:?}");
        assert!(sample.bytes <= 4096, "{sample:?}");
    }
}
