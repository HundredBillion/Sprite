# Native Windows Support

## Problem Statement

Sprite currently compiles and runs only on Linux and macOS. A Windows PC user
cannot open a native Sprite window, run an interactive PowerShell session, or
use the Pane Control and Surface Channel features that distinguish Sprite from
an ordinary terminal.

## Solution

Support 64-bit Windows 10 version 1809 and later, including Windows 11, as a
native Sprite platform. The application will host Terminal Sessions through
the Windows Pseudoconsole (ConPTY), run PowerShell by default, use a local
same-user named-pipe transport for Pane Control and the Surface Channel, and
publish a signed-by-CI build artifact for `x86_64-pc-windows-msvc`.

The current Linux and macOS behavior remains unchanged. Windows support is
native; it is not a WSL launcher and it does not open a TCP listener.

## User Stories

1. As a Windows user, I want to launch `sprite.exe`, so that I have a native terminal window on my PC.
2. As a Windows user, I want a new Pane to run PowerShell by default, so that I can immediately use my normal shell.
3. As a Windows user, I want to configure another absolute executable as my shell, so that I can use PowerShell 7, `cmd.exe`, or a development shell.
4. As a Windows user, I want terminal input, output, resize, Unicode, colour, mouse, and VT sequences to work, so that interactive console programs work normally.
5. As a Windows user, I want closing a Pane or Sprite to end the directly hosted process, so that no child process is silently left running.
6. As a Windows user, I want the documented quit shortcut and copy/paste bindings to use Windows conventions, so that common application actions are discoverable.
7. As a program running under Sprite on Windows, I want to use Pane Control, so that I can list, read, and manage panes without a network service.
8. As a Surface Client on Windows, I want to use the Surface Channel, so that I can draw a native Surface in my own Pane.
9. As a security-conscious user, I want local control endpoints to accept only my Windows identity and their existing protocol authentication, so that another local user cannot control my panes.
10. As a release consumer, I want a Windows artifact whose provenance is checked in CI, so that I can obtain a reproducible native build.
11. As a Linux or macOS user, I want existing terminal behavior and local endpoint paths to remain unchanged, so that Windows support does not regress supported platforms.
12. As a maintainer, I want Windows compilation and focused headless tests in CI, so that platform-specific build regressions are caught before release.

## Implementation Decisions

- Target `x86_64-pc-windows-msvc` on Windows 10 version 1809 or later. This is the minimum Windows release that supports ConPTY.
- Reuse the resolved `portable-pty` 0.9.0 dependency's native `ConPtySystem`; do not introduce a second PTY implementation or bind ConPTY directly. Its platform-neutral reader, writer, resize, and child interfaces become the Terminal Core seam.
- Split Unix-only PTY pumping and job cleanup from portable Terminal Session behavior. The Windows implementation keeps bounded input/output admission and one Terminal Session owner, but uses ConPTY handles and Windows process termination instead of Unix descriptors, signals, sessions, and process groups.
- On Windows, a Terminal Session owns the directly spawned child. It guarantees termination and reaping of that child on explicit shutdown; Unix-only Ordinary Session Job discovery and process-group escalation are not claimed for Windows until a Windows Job Object design is separately accepted.
- Resolve the default Windows shell in this order: an explicitly configured absolute executable, `pwsh.exe` when it is discoverable, then `%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe`. Existing Unix shell resolution stays unchanged.
- Preserve existing protocol schemas and the `SPRITE_PANE`/Surface environment contracts. Replace only the transport beneath them on Windows.
- Use an authenticated local named pipe for each Sprite Window and Surface Channel endpoint on Windows. Its explicit DACL permits only the current logon SID and denies network access; do not accept Windows' permissive default pipe security. The existing per-window key remains required for Surface Channel authentication. There is no loopback TCP fallback.
- Keep Unix domain sockets and their permission model on Linux and macOS. Extract transport-neutral stream, endpoint, timeout, and cancellation interfaces only where needed to make the two implementations testable.
- Use `BCryptGenRandom` for Windows endpoint-key entropy; retain `/dev/urandom` on Unix. The random bytes and protocol representation remain unchanged.
- Enable GPUI's Windows manifest feature using the vendored GPUI 0.2.2 source already pinned by the workspace; retain current Linux and macOS feature selections.
- Add a Windows GitHub Actions job that installs the pinned Rust and Zig toolchains, fetches Ghostty dependencies, builds the workspace for `x86_64-pc-windows-msvc`, runs portable headless tests, and uploads `sprite.exe` as an artifact. A future installer, code signing certificate, MSIX package, and winget publication are separate release work.

### Source-grounded decision record

| Field | Evidence |
| --- | --- |
| Decision | Use `portable-pty`'s Windows ConPTY implementation rather than a custom Windows FFI layer. |
| Version chain | `Cargo.toml` pins `portable-pty = 0.9.0`; `Cargo.lock` resolves 0.9.0; the installed crate source aliases `NativePtySystem` to `win::conpty::ConPtySystem` on Windows. |
| Primary source | Microsoft documents ConPTY as the Windows terminal-host API, with synchronous I/O streams, UTF-8 VT traffic, separate I/O servicing, resizing, and a Windows 10 1809 minimum: [CreatePseudoConsole](https://learn.microsoft.com/en-us/windows/console/createpseudoconsole) and [Creating a Pseudoconsole session](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session). |
| Local precedent | The Terminal Core already owns a `portable-pty` PTY pair but bypasses its portable reader/writer interfaces for Unix descriptor polling. This is Unix-specific and must be isolated, not copied. |
| Verification | Add Windows `cargo check`, workspace build, and focused terminal/transport tests in hosted CI; run live PowerShell and endpoint smoke tests on a native Windows runner. |

## Testing Decisions

- Tests assert user-visible terminal and protocol behavior, not ConPTY or named-pipe implementation details.
- Add platform-gated Terminal Core tests for default-shell resolution, resize, PowerShell output, stdin delivery, natural exit, and explicit shutdown of a directly hosted child.
- Add platform-gated transport tests for same-logon-session connection acceptance, rejection of unauthenticated Surface requests, endpoint cancellation, timeout bounds, and cleanup.
- Keep existing Unix socket, process-group, signal, and terminfo tests Unix-only; their assertions do not describe Windows semantics.
- Run the current platform-independent parser, render, Pane Control, and Surface protocol suites on Windows where their test helpers no longer assume `UnixStream`.
- Maintain an explicit native Windows manual qualification gate for live window input, clipboard, IMEs, and graphics, because headless CI cannot establish desktop integration.

## Out of Scope

- Windows on ARM, 32-bit Windows, Windows versions before 10 version 1809, and Windows Server before 2019.
- WSL as a substitute for a native Windows build.
- A MSI/MSIX installer, Authenticode signing, winget publication, automatic updates, or Store distribution.
- Windows Job Object support for descendant-job cleanup beyond the directly hosted child.
- Replacing Linux/macOS Unix sockets, shell integration, terminfo installation, packaging, or process-group cleanup.

## Further Notes

The change spans Terminal Core, the Sprite application's local endpoint layer,
platform key wording, packaging documentation, and CI. The named-pipe choice
is deliberately architectural: it retains the product's no-network transport
boundary where Unix domain sockets are unavailable to the current Rust code.
