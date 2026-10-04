# Keep the Surface Channel separate from Pane Observation

Surfaces travel over a **second endpoint** — its own socket file, exported to
children as `SPRITE_SURFACE_SOCKET`, speaking newline-delimited JSON. The two
endpoints share the transport implementation and key type, **never their grammar**.
Production creates independent observation and Surface keys; possessing only
observation credentials does not authorize Surface requests.

Taken while grilling the native-surfaces PRD on 2026-09-07, after the PRD's
first draft had put surface verbs on the observation socket. Qualified on
2026-10-04 to describe the existing configuration commands and independent keys
accurately, while extracting the common transport.

## Why

**Pane queries grant no control of a Pane or its child.** The `Query` type in
`observation/request.rs` contains no operation that draws, sends input, takes
focus, or opens a stream. The observation endpoint also accepts its existing
`config print` and `config reload` commands: the former reads configuration and
the latter changes window settings. The earlier claim that nothing on that
socket could mutate was too broad. Those commands remain supported, but Surface
verbs cannot enter either the pane-query grammar or the configuration grammar.

**The framing differs.** Observation exchanges a bounded line of space-separated
words for a response framed by EOF. Surface descriptions are structured documents
flowing in both directions for the Surface's lifetime. An SVG icon set can be
large, so Surface messages need a larger bound. Sharing socket mechanics does
not require sharing either grammar or connection lifetime.

**A future Surface permission model has a home.** Gating who may draw into which
pane belongs in the Surface adapter. Pane observation cannot construct those
operations. Configuration authorization remains a separate existing concern.

## Shared transport contract

`local_socket::LocalSocket` owns directory preparation, random names, the key,
binding, accepted connection accounting, first-line authentication, cancellation,
and socket unlinking. `ObservationKey` and the runtime helpers remain available
through their old observation module paths. The window still supplies an
independently generated key when constructing the Surface endpoint.

| Policy | Observation | Surface |
| --- | --- | --- |
| Concurrent connections, including unauthenticated clients | 16 | 64 |
| Maximum first line, including key and newline | 8 KiB | 16 MiB |
| Total authentication deadline from acceptance | 2 seconds | 5 seconds |
| Blocking socket write timeout | 2 seconds | 2 seconds |
| Random filename | 24 hex digits + `.sock` | 16 hex digits + `.surface.sock` |
| Authenticated lifetime | One response, then EOF | Until stream/endpoint closes |

Both filenames occupy 29 bytes. This deliberately preserves room for macOS's
long per-user temporary directory. The shared path guard permits 103 bytes on
macOS and 107 on Linux, excluding the terminating NUL. Surface tests reduce
handshake/write timeouts to 200 ms; production values are unchanged.

The directory is made `0700` before binding; the socket is `0600`. The shared
bind helper also creates abandoned-socket test fixtures, so permissions and path
validation have one implementation. Socket names are independent random draws,
never derived from authentication secrets. Stale cleanup only removes paths
whose connection attempt reports `ConnectionRefused`. Other errors prove
nothing. Ordinary closure joins the listener, interrupts all accepted socket I/O,
and unlinks only that socket. The shared runtime directory stays to avoid racing
other windows' startup. Connection slots are returned on normal completion,
spawn failure, and adapter panic.

A configuration reload that disables its own observation endpoint makes one
narrow exception: its already authenticated, one-shot request may finish writing
the reload report under the existing two-second socket write timeout. The
transport first stops accepts/authentication, unlinks the socket, cancels all
other clients (including incomplete handshakes), and shuts down the initiating
connection's read side. The observation adapter writes one reply and closes;
it cannot accept another request on that connection. Ordinary window shutdown
and Surface closure continue to cancel every connection.

Only the transport can construct the opaque reply identity. It combines the
connection number with the identity of its owning connection registry. A stale
request or an identity from another/reopened endpoint cannot spare a different
connection with the same number. The identity passes through the authenticated
observation request and bounded workspace relay; it is never parsed from client
input and does not carry the key. Authentication/connection caps and timeout
values are unchanged; no delayed shutdown timer or second transport is added.

Authentication consumes a complete newline-terminated UTF-8 line before testing
the exact key once. EOF before newline, an oversized line, an invalid key, and
cancellation never dispatch a protocol request. Observation answers `denied`;
Surface retains its JSON `refused` event. The transport passes both the first
body and its original `BufReader` to the adapter: buffered bytes after the first
newline must survive when a client pipelines an open and subsequent messages.

The request framing evidence predates this extraction: the module contract in
`observation/request.rs` calls a request a line of text; the endpoint's
`MAX_REQUEST_BYTES` comment defines its bound relative to a newline; and
`observation/client.rs::exchange` writes with `writeln!` and flushes without
half-closing its request stream. [ADR 0001](0001-protected-local-pane-observation.md)
identifies that bundled CLI as the supported interface and the socket protocol
as private. The checkpoint-3 TSP's EOF framing decision explicitly concerns
**responses**. The old endpoint nevertheless accepted an EOF-terminated partial
request, and could dispatch a prefix truncated at the byte cap. Regression tests
reproduce both old behaviors and require refusal before dispatch. No supported
EOF-terminated request variant was found; this change enforces complete request
lines while retaining EOF-framed responses.

### Timeout semantics

The previous implementations set `SO_RCVTIMEO` once and called `read_line`.
That timeout applies to individual blocking reads, **not the whole handshake**;
a client sending occasional bytes could renew its effective budget. The shared
reader records a deadline at accept and supplies the remaining duration before
each buffered read. It checks that deadline again before dispatch. Silent and
trickling clients therefore both lose their slot after the authentication budget,
subject to OS scheduling. Failure to configure required timeouts refuses the
connection instead of silently allowing an unbounded read.

After authentication the read timeout is cleared, preserving long idle Surface
streams. The Surface write timeout now also covers initial refusals and one-shot
responses; previously only established `SurfaceConnection` writes set it.
Writes remain per-operation inactivity bounds, not total response deadlines.
The observation CLI likewise uses per-operation read/write timeouts, not a total
15-second exchange deadline.

The generic workspace reply relay retains the existing queue submission and
reply-wait semantics: blocking enqueue, then a bounded reply wait (5 seconds for
Surface, the existing configuration timeout for config). Its deadline does not
cover queue submission or arbitrary adapter work. Closing interrupts socket I/O;
it does not join workers awaiting the GPUI queue/reply. Accepted workers remain
bounded by each listener's connection cap. No new timer thread, polling animation,
or dependency is introduced.

The API evidence is Rust's [`UnixStream` documentation](https://doc.rust-lang.org/std/os/unix/net/struct.UnixStream.html#method.set_read_timeout),
[`BufReader` documentation](https://doc.rust-lang.org/std/io/struct.BufReader.html),
and the Linux [`SO_RCVTIMEO` contract](https://man7.org/linux/man-pages/man7/socket.7.html).
The requested versioned Rust documentation URL was unavailable; the installed
Rust 1.97.1 sources confirm `UnixStream::set_read_timeout` delegates to
`SO_RCVTIMEO`, cloned handles share socket options, and dropping a buffered reader
discards unread bytes. The manifest and toolchain both require/use 1.97.1.

## Wire compatibility

`surface/wire.rs` owns pure first-request/stream parsing and event serialization.
One `Envelope` check handles versions. Open and capabilities requests still need
version 1. Existing versionless focus/token callers and messages on an established
stream explicitly default to version 1. An explicitly unsupported version is
refused consistently, including paths that previously ignored that field. Unknown
verbs remain grammar errors. Built-in clients emit versioned requests; an explicit
version supplied by a streaming client is retained for validation, never silently
rewritten. Existing event JSON, field ordering, feature names, and limits remain
unchanged.

This is a compatible transport extraction with retained legacy decoding, not a
contract-removal migration. Old clients continue to work with the new server;
new version-1 requests also work with the old server, which already recognizes
version 1 on open/capabilities and ignores the added version on other verbs.
The reference CLI, the two Surface Python scripts, and existing external Surface
callers may continue to omit the legacy fields. No historical data, database
backfill, dual writes, or consumer-retirement gate applies. Rollback is reverting
the extraction; it requires no data deletion. Tests cover both protocol adapters,
legacy/new envelopes, unchanged event fixtures, and transport resource boundaries.
