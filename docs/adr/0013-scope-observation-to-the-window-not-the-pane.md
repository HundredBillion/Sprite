# Scope pane observation to the window, not the pane

Any pane in a Sprite window may read any other pane in that window. The
observation key is a boundary between windows and everything else; it is
deliberately **not** a boundary between the panes inside one window.

This was implicit in Checkpoint 3's implementation and is now an explicit
product decision, taken by the project owner on 2026-08-23. It resolves the
Checkpoint 3 TSP's open question 2.

## Why

The capability exists so that tools can coordinate across panes: a program
running in one pane needs to see what another pane is doing — a build failing on
the left, a server logging on the right. Agents driving a shell are the
motivating consumer, but the capability is a general terminal one, and nothing
in the implementation is specific to them. The PRD is explicit on that point and
this decision does not change it.

Per-pane isolation would make the common case the awkward one. A tool would have
to be granted access pane by pane, for a boundary that would not hold anyway:
every session in a window is given the same key, so a caller refused its
neighbour's contents could simply ask for `--window` instead. A boundary that
can be walked around is worse than no boundary, because it suggests a protection
that is not there.

## What follows from it

**A program in one pane can read anything visible in another pane of the same
window** — including a password, a token, or a private file someone happened to
display. The window is the unit of trust. Things that should not see each other
belong in different windows.

**Separate windows stay separate.** Each has its own socket and its own key, and
scope resolves only against the panes of the window that was asked, so a request
cannot name a pane elsewhere. That is structural rather than a check: the broker
is handed one window's panes and cannot see any others.

**`--from` may be self-reported without harm.** A caller claiming to be a
different pane gains nothing it could not already have by asking for `--window`.
It shapes the default scope for convenience; it is not a privilege, and the code
says so where it is used.

**Unix peer credentials are not needed for pane identity.** They were the
obvious way to make `--from` trustworthy, and there is now no reason to want
that. What keeps other *users* out is the runtime directory's `0700` mode, the
socket's `0600`, and the key itself — not the caller's identity.

## What would reopen it

A product feature that gives panes different privilege — a pane explicitly
marked private, or a per-pane grant a person confirms — would need peer
credentials plus a policy to enforce it. That is a feature to design, not a
defect to fix. Until such a feature exists, code should not add partial per-pane
checks: half a boundary is the thing this decision rejects.

## Alternatives considered

**Peer credentials binding a request to the pane that sent it.** Rejected: it
makes `--from` honest without making it meaningful, since the same key still
authorises `--window`.

**Per-pane keys.** Rejected: it would isolate panes properly, at the cost of
making the ordinary cross-pane request require an explicit grant, which is the
case the feature exists to serve.

**Prompting a person per request.** Rejected by the PRD, which requires
observation to be automatically available to local tools without prompts.
