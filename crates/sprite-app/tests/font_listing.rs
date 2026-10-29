//! Listing the installed fonts through the real macOS text system, as every
//! configuration reload does, must not leave the listing behind.
//!
//! It runs its own `main` because AppKit insists on the main thread, and a
//! headless GPUI application ends the process when it quits.

#[cfg(target_os = "macos")]
fn main() {
    // One listing costs about 0.7 MB when it leaks, so thirty leaking
    // listings grow the process by about 20 MB. Memory that comes and goes
    // between samples stays well under the limit.
    const LISTINGS: usize = 30;
    const LIMIT_BYTES: u64 = 8 * 1024 * 1024;

    gpui::Application::headless().run(|cx| {
        let resident = || {
            sprite_app::diagnostics::ResourceSample::read()
                .resident_bytes
                .expect("macOS reports resident memory")
        };
        // The first listing warms caches that are meant to stay.
        let names = cx.text_system().all_font_names();
        let before = resident();
        for _ in 0..LISTINGS {
            let again = cx.text_system().all_font_names();
            assert_eq!(again, names, "the same fonts are listed every time");
        }
        let grew = resident().saturating_sub(before);

        let mut failures = Vec::new();
        if !names.iter().any(|name| name == "Menlo") {
            failures.push("Menlo, installed with every macOS, is not listed".to_owned());
        }
        if grew > LIMIT_BYTES {
            failures.push(format!(
                "{LISTINGS} listings grew the process by {} KiB",
                grew / 1024
            ));
        }
        if failures.is_empty() {
            println!(
                "font listing: {LISTINGS} listings grew the process by {} KiB",
                grew / 1024
            );
            std::process::exit(0);
        }
        for failure in failures {
            eprintln!("font listing: {failure}");
        }
        std::process::exit(1);
    });
}

#[cfg(not(target_os = "macos"))]
fn main() {}
