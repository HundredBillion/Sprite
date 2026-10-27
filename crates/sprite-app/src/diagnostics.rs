use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const EVENT_LIMIT: usize = 4096;
const QUEUE_LIMIT: usize = 512;

pub struct Guard {
    running: Arc<AtomicBool>,
    stopped: mpsc::Receiver<()>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        self.stopped.recv_timeout(Duration::from_millis(200)).ok();
    }
}

#[derive(Clone)]
struct Writer {
    sender: mpsc::SyncSender<Vec<u8>>,
    dropped: Arc<AtomicUsize>,
}

struct Event {
    writer: Writer,
    bytes: Vec<u8>,
}

impl Write for Event {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let keep = bytes
            .len()
            .min((EVENT_LIMIT - 1).saturating_sub(self.bytes.len()));
        self.bytes.extend_from_slice(&bytes[..keep]);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        if let Err(error) = std::str::from_utf8(&self.bytes) {
            self.bytes.truncate(error.valid_up_to());
        }
        if !self.bytes.is_empty() {
            self.bytes.push(b'\n');
        }
        if !self.bytes.is_empty()
            && self
                .writer
                .sender
                .try_send(std::mem::take(&mut self.bytes))
                .is_err()
        {
            self.writer.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

struct DiagnosticLayer(Writer);

struct Fields<'a>(&'a mut Event);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        write!(self.0, " {}={:?}", field.name(), value).ok();
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for DiagnosticLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let mut output = Event {
            writer: self.0.clone(),
            bytes: Vec::new(),
        };
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let metadata = event.metadata();
        write!(
            output,
            "unix_ms={stamp} level={} subsystem={} pid={}",
            metadata.level(),
            metadata.target(),
            std::process::id()
        )
        .ok();
        event.record(&mut Fields(&mut output));
    }
}

const SEGMENT_LIMIT: u64 = 1024 * 1024;
const SEGMENTS: usize = 4;
const CLOSED_RUNS: usize = 10;

struct Disk {
    run: PathBuf,
    file: std::fs::File,
    size: u64,
    _marker: std::fs::File,
}

fn private_file(path: &std::path::Path, truncate: bool) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(truncate)
        .mode(0o600)
        .open(path)
}

fn cleanup(root: &std::path::Path) -> io::Result<()> {
    let lock = private_file(&root.join(".lock"), false)?;
    lock.lock()?;
    let mut closed = Vec::new();
    for entry in std::fs::read_dir(root)?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let mut parts = name.split('-');
        let valid = parts.next() == Some("run")
            && parts
                .next()
                .is_some_and(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
            && parts
                .next()
                .is_some_and(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
            && parts.next().is_none();
        if !valid || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let path = entry.path();
        let marker = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.join(".active"))
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                std::fs::remove_dir_all(path).ok();
                continue;
            }
            Err(_) => continue,
        };
        if marker.try_lock().is_err() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(UNIX_EPOCH);
        drop(marker);
        closed.push((modified, path));
        if closed.len() > CLOSED_RUNS {
            closed.sort_by_key(|entry| entry.0);
            let (_, old) = closed.remove(0);
            if let Ok(marker) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(old.join(".active"))
                && marker.try_lock().is_ok()
            {
                std::fs::remove_dir_all(old).ok();
            }
        }
    }
    Ok(())
}

impl Disk {
    fn open(root: PathBuf) -> io::Result<Self> {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&root)?;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        let lock = private_file(&root.join(".lock"), false)?;
        lock.lock()?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let run = root.join(format!("run-{}-{stamp}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&run)?;
        let opened = (|| {
            let marker = private_file(&run.join(".active"), false)?;
            marker.try_lock().map_err(io::Error::from)?;
            let file = private_file(&run.join("sprite.log"), true)?;
            Ok::<_, io::Error>((marker, file))
        })();
        let (marker, file) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                std::fs::remove_dir_all(&run).ok();
                return Err(error);
            }
        };
        drop(lock);
        cleanup(&root).ok();
        Ok(Self {
            run,
            file,
            size: 0,
            _marker: marker,
        })
    }

    fn write_event(&mut self, event: &[u8]) -> io::Result<()> {
        let event = &event[..event.len().min(EVENT_LIMIT)];
        if self.size + event.len() as u64 > SEGMENT_LIMIT {
            for index in (1..SEGMENTS).rev() {
                let previous = if index == 1 {
                    self.run.join("sprite.log")
                } else {
                    self.run.join(format!("sprite.{}.log", index - 1))
                };
                let next = self.run.join(format!("sprite.{index}.log"));
                match std::fs::rename(previous, next) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            self.file = private_file(&self.run.join("sprite.log"), true)?;
            self.size = 0;
        }
        self.file.write_all(event)?;
        self.size += event.len() as u64;
        self.file.flush()
    }
}

fn drain_pending(
    receiver: &mpsc::Receiver<Vec<u8>>,
    mut healthy: bool,
    mut write: impl FnMut(&[u8]) -> io::Result<()>,
) -> bool {
    while healthy {
        let Ok(event) = receiver.try_recv() else {
            break;
        };
        healthy = write(&event).is_ok();
    }
    healthy
}

fn report_dropped(disk: &mut Disk, dropped: &AtomicUsize) -> io::Result<()> {
    let count = dropped.swap(0, Ordering::Relaxed);
    if count == 0 {
        return Ok(());
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    disk.write_event(format!("unix_ms={stamp} level=WARN subsystem=sprite_app::diagnostics pid={} dropped_events={count}\n", std::process::id()).as_bytes())
}

pub fn init() -> Option<Guard> {
    #[cfg(target_os = "macos")]
    let root = PathBuf::from(std::env::var_os("HOME")?).join("Library/Logs/Sprite");
    #[cfg(target_os = "linux")]
    let root = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?
        .join("sprite/logs");
    let requested = std::env::var("SPRITE_LOG").unwrap_or_else(|_| "sprite_app=warn".to_owned());
    let filter = if tracing_subscriber::EnvFilter::try_new(&requested).is_ok() {
        requested
    } else {
        "sprite_app=warn".to_owned()
    };
    let guard = init_at(root, &filter).ok()?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(location) = info.location() {
            tracing::error!(target: "sprite_app::panic", file = location.file(), line = location.line(), "application panic");
        } else {
            tracing::error!(target: "sprite_app::panic", "application panic");
        }
        previous(info);
    }));
    Some(guard)
}

pub fn init_at(root: PathBuf, filter: &str) -> io::Result<Guard> {
    use tracing_subscriber::prelude::*;
    let filter = tracing_subscriber::EnvFilter::try_new(filter)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(QUEUE_LIMIT);
    let (stopped_sender, stopped) = mpsc::sync_channel(1);
    let running = Arc::new(AtomicBool::new(true));
    let worker_running = Arc::clone(&running);
    let dropped = Arc::new(AtomicUsize::new(0));
    let worker_dropped = Arc::clone(&dropped);
    std::thread::Builder::new()
        .name("sprite-log".into())
        .spawn(move || {
            let Ok(mut disk) = Disk::open(root.clone()) else {
                return;
            };
            let mut healthy = true;
            while worker_running.load(Ordering::Acquire) {
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(event) => {
                        if report_dropped(&mut disk, &worker_dropped).is_err() {
                            healthy = false;
                            break;
                        }
                        if disk.write_event(&event).is_err() {
                            healthy = false;
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            let healthy = drain_pending(&receiver, healthy, |event| disk.write_event(event));
            if healthy {
                report_dropped(&mut disk, &worker_dropped).ok();
            }
            drop(disk);
            cleanup(&root).ok();
            stopped_sender.try_send(()).ok();
        })?;
    tracing_subscriber::registry()
        .with(filter)
        .with(DiagnosticLayer(Writer { sender, dropped }))
        .try_init()
        .map_err(|error| {
            running.store(false, Ordering::Release);
            io::Error::other(error.to_string())
        })?;
    Ok(Guard { running, stopped })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_event_count_is_written_when_the_sink_recovers() {
        let root = std::env::temp_dir().join(format!("sprite-log-drops-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let mut disk = Disk::open(root.clone()).unwrap();
        let dropped = AtomicUsize::new(7);
        report_dropped(&mut disk, &dropped).unwrap();
        let contents = std::fs::read_to_string(disk.run.join("sprite.log")).unwrap();
        assert!(contents.contains("dropped_events=7"));
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
        drop(disk);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn partial_write_failure_stops_all_later_writes() {
        let (tx, rx) = mpsc::sync_channel(3);
        tx.send(vec![1; 10]).unwrap();
        tx.send(vec![2; 10]).unwrap();
        let mut written = Vec::new();
        drain_pending(&rx, true, |event| {
            written.extend_from_slice(&event[..3]);
            Err(io::Error::from_raw_os_error(nix::libc::ENOSPC))
        });
        assert_eq!(written, vec![1; 3]);
        assert_eq!(rx.try_recv().unwrap(), vec![2; 10]);
        let mut attempted = false;
        drain_pending(&rx, false, |_| {
            attempted = true;
            Ok(())
        });
        assert!(!attempted);
    }

    #[test]
    fn an_unwritable_destination_fails_without_panicking() {
        let root = std::env::temp_dir().join(format!("sprite-log-file-{}", std::process::id()));
        std::fs::write(&root, b"not a directory").unwrap();
        assert!(Disk::open(root.clone()).is_err());
        std::fs::remove_file(root).unwrap();
    }

    #[test]
    fn a_full_queue_drops_without_blocking_and_caps_event_size() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let dropped = Arc::new(AtomicUsize::new(0));
        let writer = Writer {
            sender,
            dropped: Arc::clone(&dropped),
        };
        let mut first = Event {
            writer: writer.clone(),
            bytes: Vec::new(),
        };
        first.write_all(&vec![b'x'; EVENT_LIMIT * 2]).unwrap();
        drop(first);
        let mut second = Event {
            writer,
            bytes: Vec::new(),
        };
        second.write_all(b"second").unwrap();
        drop(second);
        assert_eq!(receiver.try_recv().unwrap().len(), EVENT_LIMIT);
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn cleanup_removes_an_interrupted_run_without_a_marker() {
        let root =
            std::env::temp_dir().join(format!("sprite-log-incomplete-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("run-123-456")).unwrap();
        std::fs::create_dir_all(root.join("run-do-not-delete")).unwrap();
        std::fs::write(root.join("run-do-not-delete/.active"), b"").unwrap();
        for number in 0..CLOSED_RUNS + 2 {
            let path = root.join(format!("run-1-{number}"));
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join(".active"), b"").unwrap();
        }
        cleanup(&root).unwrap();
        assert!(!root.join("run-123-456").exists());
        assert!(root.join("run-do-not-delete").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_handles_many_closed_runs_with_a_low_descriptor_limit() {
        const CHILD: &str = "SPRITE_LOG_LOW_FD_CHILD";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            for number in 0..150 {
                let run = root.join(format!("run-1-{number}"));
                std::fs::create_dir(&run).unwrap();
                std::fs::write(run.join(".active"), b"").unwrap();
            }
            cleanup(&root).unwrap();
            let count = std::fs::read_dir(root)
                .unwrap()
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("run-"))
                .count();
            assert_eq!(count, CLOSED_RUNS);
            return;
        }
        let root = std::env::temp_dir().join(format!("sprite-log-low-fd-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        let status = std::process::Command::new("/bin/sh")
            .args(["-c", "ulimit -n 64; exec \"$1\" --exact diagnostics::tests::cleanup_handles_many_closed_runs_with_a_low_descriptor_limit", "sh"])
            .arg(std::env::current_exe().unwrap()).env(CHILD, &root)
            .status().unwrap();
        assert!(status.success());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_does_not_remove_a_live_process_run() {
        use std::io::Read;
        use std::process::{Command, Stdio};
        use std::time::Instant;
        const CHILD: &str = "SPRITE_LOG_RETENTION_CHILD";
        if let Some(root) = std::env::var_os(CHILD) {
            let disk = Disk::open(PathBuf::from(&root)).unwrap();
            let root = PathBuf::from(root);
            std::fs::write(
                root.join("child-pending"),
                disk.run.to_string_lossy().as_bytes(),
            )
            .unwrap();
            std::fs::rename(root.join("child-pending"), root.join("child-ready")).unwrap();
            let mut byte = [0];
            std::io::stdin().read_exact(&mut byte).ok();
            return;
        }
        let root = std::env::temp_dir().join(format!("sprite-log-child-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "diagnostics::tests::cleanup_does_not_remove_a_live_process_run",
            ])
            .env(CHILD, &root)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let ready = root.join("child-ready");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(Instant::now() < deadline, "child did not create a run");
            crate::test_blocking_wait::pause(Duration::from_millis(10));
        }
        let active = PathBuf::from(std::fs::read_to_string(&ready).unwrap());
        for _ in 0..CLOSED_RUNS + 3 {
            drop(Disk::open(root.clone()).unwrap());
        }
        cleanup(&root).unwrap();
        assert!(active.exists());
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        cleanup(&root).unwrap();
        assert!(!active.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_preserves_active_runs_and_bounds_closed_runs() {
        let root =
            std::env::temp_dir().join(format!("sprite-log-retention-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let active = Disk::open(root.clone()).unwrap();
        for _ in 0..(CLOSED_RUNS + 3) {
            drop(Disk::open(root.clone()).unwrap());
        }
        cleanup(&root).unwrap();
        assert!(active.run.exists());
        let runs = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("run-"))
            .count();
        assert_eq!(runs, CLOSED_RUNS + 1);
        drop(active);
        cleanup(&root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rotation_bounds_the_run_and_retains_recent_events() {
        let root = std::env::temp_dir().join(format!("sprite-log-rotate-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let mut disk = Disk::open(root.clone()).unwrap();
        for _ in 0..3000 {
            disk.write_event(&vec![b'x'; EVENT_LIMIT]).unwrap();
        }
        disk.write_event(b"recent marker\n").unwrap();
        let mut total = 0;
        let mut found = false;
        for entry in std::fs::read_dir(&disk.run).unwrap().flatten() {
            let data = std::fs::read(entry.path()).unwrap();
            total += data.len() as u64;
            found |= data.windows(13).any(|part| part == b"recent marker");
        }
        assert!(total <= SEGMENT_LIMIT * SEGMENTS as u64);
        assert!(found);
        drop(disk);
        std::fs::remove_dir_all(root).unwrap();
    }
}
