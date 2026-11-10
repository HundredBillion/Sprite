# A label is a link only when its address is not shown

Amends the link policy set by issue #96 and PR #98 (2026-10-18). Requested by
the owner in issue #161 on 2026-11-10.

PR #98 made visible `http://` and `https://` URLs the only terminal links and
stopped resolving OSC 8 targets, so a descriptive label such as `Sprite PR #95`
was plain text. The request behind it had one layout in mind: a label printed
next to its address, where the address stays clickable. Programs that print
only the label — Claude Code renders Markdown links this way — left the page
unreachable from the terminal.

A click now resolves in this order:

1. A cell inside a visible URL opens that URL. What the person can read is
   where the click goes; OSC 8 metadata on those cells is ignored.
2. Otherwise, a cell carrying an OSC 8 target opens that target when the
   target passes the scheme policy (`https`, `http`), the linked text on the
   row shows no URL of its own, and the target is not printed as a visible URL
   anywhere on the row.

The two refusals in step 2 keep what PR #98 was protecting. Linked text that
shows a URL offers only that URL, so a label cannot display one address and
open another. A label beside its own address stays plain text, which is the
behavior issue #96 asked for.

Accepted risk: a label that shows no URL can still describe one site and link
to another, as it can in every terminal that honors OSC 8. Sprite does not yet
show the target before the click. The scheme policy limits where such a link
can lead.

The row is the unit for "shown": a label and its address on different rows are
both links. One open per click, mouse-reporting programs owning ordinary
clicks, and Cmd/Ctrl-click are unchanged.

Considered and rejected: restoring every OSC 8 label as a link (reverses the
owner's request in #96 for the label-beside-address case); keeping labels
inert and asking programs to print addresses (Sprite cannot change what Claude
Code prints); a confirmation before opening a labelled link (the owner wants
no Sprite prompt that asks a person to approve an action, ADR 0036).
