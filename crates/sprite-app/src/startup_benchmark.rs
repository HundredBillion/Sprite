//! Opt-in measurements ending after terminal grid CPU paint, before presentation.
#![cfg_attr(
    feature = "startup-benchmark",
    expect(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "a benchmark is a developer tool; a failed setup stops it with the cause"
    )
)]

use std::time::{Duration, Instant};

use gpui::{App, EntityId, Global};
use sprite_term::SnapshotBundle;

pub(crate) const MARKER: &str = "SPRITE_PAINT_PROBE_7";
const MAX_PANES: usize = 16;

#[derive(Clone, Debug)]
pub(crate) struct Painted {
    pub entity: EntityId,
    pub generation: Option<u64>,
    pub marker: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Sample {
    pub label: &'static str,
    pub elapsed: Duration,
    pub constructor_elapsed: Option<Duration>,
    pub generation: u64,
}

pub(crate) struct Probe {
    pub paints: Vec<Painted>,
    pub sample: Option<Sample>,
    seen: Vec<EntityId>,
    constructors: Vec<(EntityId, Instant)>,
    label: &'static str,
    start: Instant,
}

impl Global for Probe {}

impl Probe {
    pub(crate) fn new(label: &'static str) -> Self {
        Self {
            paints: Vec::new(),
            sample: None,
            seen: Vec::new(),
            constructors: Vec::new(),
            label,
            start: Instant::now(),
        }
    }

    pub(crate) fn arm(&mut self, label: &'static str) {
        self.label = label;
        self.start = Instant::now();
        self.sample = None;
    }
}

pub(crate) fn constructor_started(entity: EntityId, cx: &mut App) {
    if cx.has_global::<Probe>() {
        let probe = cx.global_mut::<Probe>();
        if probe.constructors.len() < MAX_PANES {
            probe.constructors.push((entity, Instant::now()));
        }
    }
}

// Capture from the same immutable bundle used to prepare GridPaint.
// Reading the view during paint could attribute a later generation to older glyphs.
pub(crate) fn capture(
    entity: EntityId,
    bundle: Option<&SnapshotBundle>,
    cx: &App,
) -> Option<Painted> {
    cx.try_global::<Probe>()?;
    Some(Painted {
        entity,
        generation: bundle.map(|bundle| bundle.generation),
        marker: bundle.is_some_and(|bundle| {
            bundle.render.rows.iter().any(|row| {
                row.cells
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<String>()
                    .contains(MARKER)
            })
        }),
    })
}

pub(crate) fn painted(paint: Option<Painted>, _window: &mut gpui::Window, cx: &mut App) {
    let Some(paint) = paint else { return };
    let probe = cx.global_mut::<Probe>();
    if probe.paints.len() < MAX_PANES * 2
        && !probe
            .paints
            .iter()
            .any(|old| old.entity == paint.entity && old.marker == paint.marker)
    {
        probe.paints.push(paint.clone());
    }
    // A marker is found only in a snapshot, so a marked paint has its generation.
    if paint.marker
        && let Some(generation) = paint.generation
        && !probe.seen.contains(&paint.entity)
        && probe.seen.len() < MAX_PANES
    {
        probe.seen.push(paint.entity);
        probe.sample = Some(Sample {
            label: probe.label,
            elapsed: probe.start.elapsed(),
            constructor_elapsed: probe
                .constructors
                .iter()
                .find(|(entity, _)| *entity == paint.entity)
                .map(|(_, start)| start.elapsed()),
            generation,
        });
    }
}

#[cfg(feature = "startup-benchmark")]
pub fn run() {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--prompt-child") => prompt_child(std::path::Path::new(&args[2]), &args[3..]),
        Some("--native") => native(std::path::PathBuf::from(&args[2]), &args[3..]),
        _ => supervise(&args[1..]),
    }
}

#[cfg(feature = "startup-benchmark")]
fn supervise(args: &[String]) {
    let root = std::env::temp_dir().join(format!("sprite-startup-bench-{}", std::process::id()));
    std::fs::create_dir(&root).expect("private probe directory");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--native")
        .arg(&root)
        .args(args)
        .spawn()
        .expect("native probe");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            child.kill().expect("stop stalled native probe");
            child.wait().expect("reap native probe");
            break None;
        }
        bounded_wait(Duration::from_millis(20));
    };
    // The child loop checks this independently of GPUI, even if the UI stalls.
    std::fs::write(root.join("stop"), []).unwrap();
    let cleanup_deadline = Instant::now() + Duration::from_secs(3);
    while std::fs::read_dir(&root).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("child-")
    }) {
        if Instant::now() >= cleanup_deadline {
            break;
        }
        bounded_wait(Duration::from_millis(20));
    }
    let completed = root.join("completed").exists();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        status.is_some_and(|status| status.success()) && completed,
        "native probe failed or exceeded its independent 30 second watchdog"
    );
}

#[cfg(feature = "startup-benchmark")]
fn prompt_child(root: &std::path::Path, args: &[String]) {
    use std::io::Write;
    let registration = root.join(format!("child-{}", std::process::id()));
    std::fs::write(&registration, []).unwrap();
    let delay = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--delay-ms="))
        .map(|ms| ms.parse::<u64>().expect("delay milliseconds"))
        .unwrap_or(0)
        .min(2000);
    let busy = args.iter().any(|arg| arg == "--busy");
    let start = Instant::now();
    let mut emitted = false;
    while start.elapsed() < Duration::from_secs(25) && !root.join("stop").exists() {
        if start.elapsed() >= Duration::from_millis(delay) && (!emitted || busy) {
            if writeln!(std::io::stdout(), "\r{MARKER}")
                .and_then(|_| std::io::stdout().flush())
                .is_err()
            {
                break;
            }
            emitted = true;
        }
        bounded_wait(Duration::from_millis(10));
    }
    std::fs::remove_file(registration).ok();
}

#[cfg(feature = "startup-benchmark")]
fn native(root: std::path::PathBuf, args: &[String]) {
    use gpui::{
        AppContext, Application, Bounds, Focusable, Keystroke, TitlebarOptions, WindowBounds,
        WindowOptions, px, size,
    };
    let manual_draw = args.iter().any(|arg| arg == "--manual-draw");
    println!(
        "paint_mode={}",
        if manual_draw {
            "manually_scheduled_native_cpu_paint"
        } else {
            "automatic_native_cpu_paint"
        }
    );
    let mut command = vec![
        std::env::current_exe().unwrap().into_os_string(),
        "--prompt-child".into(),
        root.clone().into_os_string(),
    ];
    command.extend(args.iter().map(std::ffi::OsString::from));
    Application::new().run(move |cx| {
        let bounds = Bounds::centered(None, size(px(960.0), px(640.0)), cx);
        let window = cx.open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            app_id: Some("sprite-startup-bench".into()),
            titlebar: Some(TitlebarOptions { title: Some("Sprite controlled paint probe".into()), ..Default::default() }),
            ..Default::default()
        }, |window, cx| {
            let probe = Probe::new("workspace_constructor");
            cx.set_global(probe);
            let workspace = cx.new(|cx| crate::Workspace::new(Some(command), crate::Settings::default(), None, window, cx));
            window.focus(&workspace.focus_handle(cx));
            workspace
        }).expect("open private probe window");
        window.update(cx, |_, window, cx| {
            let workspace = cx.entity();
            window.on_window_should_close(cx, move |_, cx| {
                workspace.update(cx, |workspace, cx| workspace.close_window(cx))
            });
            window.activate_window();
        }).expect("activate private probe window and install cleanup");
        cx.activate(true);
        cx.spawn(async move |cx| {
            let actions = [
                ("new_tab", "ctrl-shift-t"),
                ("split_right", "ctrl-shift-d"),
                ("split_down", "ctrl-shift-e"),
                ("warm_tab", "ctrl-shift-t"),
                ("warm_split_right", "ctrl-shift-d"),
                ("warm_split_down", "ctrl-shift-e"),
            ];
            for action in actions.into_iter().map(Some).chain(std::iter::once(None)) {
                let sample = loop {
                    if manual_draw {
                        gpui::AnyWindowHandle::from(window).update(cx, |_, window, cx| {
                            window.refresh();
                            window.draw(cx).clear();
                        }).unwrap();
                    }
                    if let Some(sample) = cx.update(|cx| cx.global_mut::<Probe>().sample.take()).unwrap() {
                        break sample;
                    }
                    cx.background_executor().timer(Duration::from_millis(5)).await;
                };
                println!("{} action_to_marker_cpu_paint_ms={:.3} constructor_to_marker_cpu_paint_ms={} generation={}", sample.label, sample.elapsed.as_secs_f64() * 1000.0, sample.constructor_elapsed.map(|elapsed| format!("{:.3}", elapsed.as_secs_f64() * 1000.0)).unwrap_or_else(|| "unavailable".into()), sample.generation);
                if let Some((label, key)) = action {
                    gpui::AnyWindowHandle::from(window).update(cx, |_, window, cx| {
                        cx.global_mut::<Probe>().arm(label);
                        window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
                    }).unwrap();
                }
            }
            std::fs::write(root.join("completed"), []).unwrap();
            std::fs::write(root.join("stop"), []).unwrap();
            window.update(cx, |workspace, _, cx| {
                workspace.close_window(cx);
                workspace.close_window(cx);
            }).unwrap();
        }).detach();
    });
}

#[cfg(feature = "startup-benchmark")]
fn bounded_wait(duration: Duration) {
    let (_sender, receiver) = std::sync::mpsc::channel::<()>();
    receiver.recv_timeout(duration).ok();
}
