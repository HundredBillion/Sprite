# Answer every Quit Request through the in-app quit path

Only the quit key and the window's close button asked about busy Panes and
waited for Pane cleanup (`workspace/close_gate.rs`). A Quit from the Dock or
the menu bar, or the system ending the session at logout, went from AppKit's
`terminate:` straight to GPUI's `App::shutdown`. Shutdown drops the windows
without the question and without the HUP → TERM → KILL escalation, so a child
that ignores the hangup kept running after Sprite was gone (GAP-C01).

The vendored GPUI now implements `applicationShouldTerminate:` in its macOS
application delegate. When the app has registered a should-terminate callback,
the delegate answers `NSTerminateLater` and passes the callback a one-shot
reply. Sprite registers one that makes the same decision as the quit key. With
a busy Pane and no question open, it puts the question on screen and replies
cancel at once, so a logout is never left waiting on a question nobody has
seen. Otherwise it replies allow once every pending cleanup has finished. A
quit that Sprite itself requests after its cleanup is not intercepted a second
time. A logout, restart or shutdown is treated like any other Quit Request:
Sprite does not inspect the quit reason, which is how Terminal.app behaves.

A reply dropped without an answer allows the quit, which is how Sprite behaved
before this hook. Cancelling on drop was rejected because a lost reply would
then refuse every later Dock Quit and block logout until the user forced Sprite
to quit. While a reply is unanswered, a further request is cancelled at once,
so Sprite holds at most one.

Rejected: GPUI's `on_app_quit` with a longer `SHUTDOWN_TIMEOUT`. Shutdown
cannot be cancelled once it has begun, so Sprite could not ask about a busy
Pane. The wait would also block the main thread for up to `GIVE_UP_AFTER`
(6 s) with the window unresponsive. Rejected: skipping the question at logout
by reading `kAEQuitReason`. That kills running programs without asking, and it
adds a second quit rule.

Cost: the patch in `patches/` must be carried forward on every GPUI upgrade,
and `scripts/check_gpui_patch.py` fails until it is. Linux has no equivalent
hook here; its session-end behaviour is unchanged.
