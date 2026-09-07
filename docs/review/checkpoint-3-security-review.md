# Checkpoint 3 security review

**Status: PERFORMED, 2026-09-05.** This is the review
[checkpoint-3-security-review-request.md](checkpoint-3-security-review-request.md)
asked for. It answers that document's four areas and every "push hardest on"
question in them, against the code as it stands on `phase_1` at `5806e45`.

**Scope and method.** A read of `observation/endpoint.rs`, `broker.rs`,
`panes.rs`, `request.rs`, `schema.rs`, and the window wiring in `workspace.rs`,
with the suite run locally (`cargo test --workspace --locked --offline`, green).
Reviewed by Claude Opus 5 at the project owner's direction. **This is a code
review, not a penetration test:** nothing here was attacked at runtime, no
fuzzing was run, and the findings are what reading the implementation against
its stated guarantees produced.

**Verdict: the model in [ADR 0013](../adr/0013-scope-observation-to-the-window-not-the-pane.md)
is implemented as described.** The window boundary is structural rather than
checked, and I could not find a path across it. Three findings are recorded
below; one of them (F1) is not a boundary defect but will decide whether pane
observation works on macOS next week, which is why it leads.

---

## Findings

### F1 — The socket-path guard is a flat 100 bytes, and it is what makes macOS CI red

`endpoint.rs:228` refuses any socket path of 100 bytes or more. That number has
no platform basis: `sockaddr_un.sun_path` is 108 bytes on Linux and 104 on
macOS, so the guard is conservative on both, and it is applied identically to
each.

What it actually costs is the test harness, not the product. `Scratch::new`
(`endpoint.rs:~500`) builds a directory under `std::env::temp_dir()`:

| | bytes |
|---|---:|
| macOS `TMPDIR` (`/var/folders/<2>/<~30>/T/`) | ~49 |
| `sprite-endpoint-<pid>-<ordinal>/` | ~24 |
| 24 hex characters + `.sock` | 29 |
| **total** | **~102 — refused** |

`Endpoint::open_in` therefore returns `Err`, and `.expect("open endpoint")`
panics. That is exactly the failure CI reports: eleven
`observation::endpoint::tests` panicking at `endpoint.rs:552`, plus
`every_refusal_is_the_same_answer` at `endpoint.rs:666`.

**The product path is not affected.** `runtime_directory()` on macOS returns
`$TMPDIR/sprite`, giving `~49 + 7 + 29 = ~85` bytes — roughly 15 bytes of
headroom under the guard and 19 under the platform limit. So the README's claim
that "pane observation does not work" on macOS appears to be **wrong**: what
does not work is the test harness that stands in for it. That is an arithmetic
argument, not a measurement, and it should be settled on the hardware next week
before either claim is repeated.

**Recommended:** derive the bound from the platform (`104`/`108` less one for
the terminator) rather than hardcoding 100, and shorten `Scratch`'s directory
name. Correct the README once the machine has confirmed it.

> **Fixed, 2026-09-05.** `MAX_SOCKET_PATH` is now 103 on macOS and 107 on Linux,
> the guard names the limit it applied, and `Scratch` uses a short hex name.
> Three tests were added: a path at the platform's maximum is accepted and
> serves requests, one byte past it is refused with the limit in the message,
> and — the one that would have caught this from Linux — the harness's widest
> possible scratch name must still fit inside a macOS `$TMPDIR`. The whole
> workspace suite also passes with `$TMPDIR` set to a macOS-length path.
> Confirmation on real hardware is still owed.

### F2 — A captured key is valid for the life of the window, and there is no rotation

The key is minted per window, injected into the environment of every session
that window starts, and inherited by every descendant of every pane. On Linux
`/proc/<pid>/environ` is owner-readable only, so exposure stays inside the user
— which is already the trust boundary ADR 0013 draws. The mechanism is sound
for what it has to do: `sprite panes snapshot` is an ordinary command a person
types, and a descriptor passed at spawn would not survive being typed.

The consequence worth recording is duration, not reach. Anything that reads the
environment once — a shell history dump, a crash reporter, a child that outlives
its pane — holds a working key until the **window** closes. Closing the *pane*
does not revoke it. There is no rotation, and `set_observation_enabled(false)`
then `true` is the only way to mint a new one.

**Recommended:** document "close the window" as the revocation, in the README's
observation section next to the sentence about the per-window key. No code
change; this is the model working as designed, and a reader should be able to
learn its edge without reading `endpoint.rs`.

### F3 — `Drop for ObservationKey` defends less than it appears to

`fill(0)` plus `black_box` wipes the 32 bytes the struct owns. It does not touch
the `String` that `to_hex()` returns, and that string is what reaches
`environment()`, the child's environment block, and every `key_hex()` caller.
Those copies are never wiped.

This is harmless and I would keep it — but it should not be cited as a defence,
because the key's real residency is the environment of every child, which the
wipe cannot reach. The doc comment on the `Drop` impl is honest about the
`unsafe` trade and should simply say this too.

---

## 1. Key handling

**Environment injection.** Right mechanism, given the CLI. See F2 for the
duration consequence.

**`/dev/urandom` directly.** Acceptable. It is the same CSPRNG a crate would
reach for, this is the only randomness Sprite needs, and the failure mode is
correct: `read_exact` propagates, `Endpoint::open` returns `Err`, and
`workspace.rs` starts the window with no endpoint. A window with no observation
is the safe failure, and it is the one that happens.

**Constant-time comparison.** Correct, and unusually carefully so. `decode_hex`
always fills the output and always inspects all 32 slots; `matches` folds with
`|=` over a `zip` of two fixed 32-byte arrays and evaluates `well_formed` only
in the final `&&`, after the loop. A wrong length costs the same as a wrong key.
I found no early return on any path.

**The wipe.** See F3.

**Sweeping dead sockets.** No race that I can find. `sweep_dead_sockets` unlinks
only a socket that *refuses* a connection, and `UnixListener::bind` makes a
socket connectable immediately — the kernel queues connections up to the backlog
whether or not anyone has called `accept` yet. So a window that has bound is
never swept, including in the gap before its serving thread first parks in
`accept`. The window between this endpoint's own sweep and its own `bind` is
likewise safe: another window's sweep in that gap finds no file at our path
because we have not created it.

The one thing resting on an unstated assumption is "bind implies connectable".
It is true on both platforms and is not going to change, but it is the load-
bearing fact of this function and the comment does not say it.

## 2. Scope enforcement

**Does the window boundary hold? Yes, and structurally.** `resolve`
(`broker.rs:171`) reads `source.panes()` and never anything else; `WindowPanes`
is constructed per window in `workspace.rs:114` and reached only through the
`Arc` that window holds. Every arm filters *that* list:

- `Scope::Window` returns it unchanged.
- `Scope::Pane(wanted)` uses `find` over it, so an id from another window simply
  does not match and produces `Denied` — the same answer as an id that never
  existed.
- `Scope::Tab` resolves `from` against the same list before using its tab, so a
  caller claiming to be a pane this window does not have is refused identically.

There is no lookup against a global registry, and no path takes a `PaneId` to a
session except through this list. A stale entry cannot help either: `forget`
removes the pane and releases its waiters, and `set_layout` *ignores* unknown
panes rather than inserting them, so a layout message cannot resurrect a closed
pane. That last detail is the one I most expected to be wrong, and it is right.

**One thing a consumer could misread: pane ids are window-local.** `PaneIds`
counts from zero per window (`pane_tree.rs:376`), so `PaneId(0)` exists in every
window and the JSON's `pane` field is not a machine-wide identifier. This is not
a leak — resolution is window-scoped, so the collision is unobservable — but a
tool that keyed a cache on `pane` across two windows would conflate them.
**Recommended:** one sentence in the schema documentation.

**Does the malformed/denied distinction leak anything? No.** The distinction is
decided by `request::parse` on grammar alone, before `resolve` runs, so no pane
state can reach it. More to the point, an unauthenticated caller never sees it:
`answer()` checks `key.matches` and writes the bare `DENIED` *before* the
handler — and therefore before the parser — is reached. Grammar feedback is
available only to a caller that already holds the key, and it tells that caller
something it already knows about its own request.

**Does anything still imply a per-pane boundary it does not enforce?** `--from`
is the candidate, and it is honest. It selects a default scope; it is not
authentication, cannot narrow what the key already permits, and a caller that
lies about it gains nothing it could not get with `--window`. The request doc's
reasoning holds. I found no other per-pane language in the code or schema.

## 3. The deadline

**It bounds collection, not the exchange — and the name does not say so.**
Confirmed in code: `collect` returns after the 500 ms window, and
`schema::render_limited` encodes afterwards, so a 16 MiB answer adds its
encoding time on top. The behaviour is right (a deadline that included encoding
would discard work already paid for), but "deadline" reads as end-to-end.
**Recommended:** rename the doc comment's subject to "collection deadline", or
say the sentence explicitly. No code change.

**TSP open question 3 — a permanently loud pane. Acceptable, and already
reported honestly.** A pane under sustained output can miss 500 ms on every
request, so a client can be starved of it indefinitely. What makes this
tolerable is that the failure is *distinguishable*: it arrives as
`FailureKind::Timeout` with a reason, alongside every healthy pane's content,
and `complete: false`. A client can retry or tell its user. Silent omission
would not be acceptable; this is not that. I would close the question as
answered rather than leave it open.

**16 concurrent connections. Not worth closing.** A caller that can occupy all
sixteen already holds the key, and therefore already has the strictly greater
capability of reading every pane in the window. Denying itself service is not an
escalation. The cap is doing its actual job, which is bounding thread growth.

## 4. The exclusion list

**Hand-written JSON is the right construction** and the reason to keep it. No
field is derived from a Rust type, so a field added to `HistorySnapshot` for the
renderer cannot appear on the wire by default — it has to be written in
deliberately. The key-set equality test is what keeps that true over time.

**`working_directory` (OSC 7) and `title` (OSC 2) are attacker-controlled, and
including them is defensible.** They are chosen by the child, so a hostile
program controls both strings entirely. The risk is not that they are reported —
a caller that may read the pane's rows may already read anything the program
printed — but that a *consumer* treats `working_directory` as a path and acts on
it. `content_trust: "untrusted_terminal_output"` is per-pane and reads as though
it is about the rows.

**Recommended:** say in the schema documentation that the declaration covers
every string in the pane object, metadata included, not only `rows`. That is the
one place a reader could reasonably draw the line in the wrong place.

**Row content — the boundary is drawn where I would draw it.** The rows *are*
the feature; a password echoed to the screen is in them, and no exclusion list
can change that without making observation useless. The PRD's exclusions are
about metadata Sprite knows and the terminal does not display — environment,
clipboard, image bytes, control sequences — and those are excluded. The
foreground executable comes from `/proc/<pid>/comm`, never `cmdline` or
`environ`, which is the correct choice: `cmdline` routinely carries secrets.

**`content_trust` is a declaration, not a defence, and saying so is
sufficient — for Phase 1.** Sprite does not neutralise prompt injection and
should not try to: it cannot know what its consumer's parser treats as an
instruction, and a terminal that mangled output to protect a hypothetical LLM
would be a worse terminal. Naming the hazard at the boundary and leaving the
decision to the consumer is the right division. This should be revisited if
Sprite ever ships a consumer of its own.

---

## What this review did not do

- No runtime attack, no fuzzing of the request grammar or the JSON encoder.
- No review of libghostty's own handling of the sequences that produce OSC 7
  and OSC 2.
- The two *general* reviews of Checkpoints 1 and 2, and Checkpoint 3's own
  general review, remain owed. This covers the security review only.
