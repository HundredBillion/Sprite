//! Emit the `xterm-ghostty` terminfo source from the exact pinned Ghostty
//! commit to stdout.
//!
//! Upstream generates this through `src/main_build_data.zig`, but that entry
//! point also pulls in shell completions and editor syntax files, which import
//! the `help_strings` module that only Ghostty's own `build.zig` constructs.
//! Running it standalone therefore fails at the pinned commit. The terminfo
//! data itself lives in `src/terminfo/ghostty.zig` and depends on nothing but
//! `std` and its sibling `Source.zig`, so this generator imports that file
//! directly as a module. It reads the vendored submodule unmodified; no Ghostty
//! source is patched or copied.

const std = @import("std");
const ghostty_terminfo = @import("ghostty_terminfo");

pub fn main(init: std.process.Init) !void {
    var buffer: [1024]u8 = undefined;
    var stdout_writer = std.Io.File.stdout().writerStreaming(init.io, &buffer);
    const writer = &stdout_writer.interface;
    try ghostty_terminfo.ghostty.encode(writer);
    try stdout_writer.end();
}
