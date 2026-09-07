# Images inside tmux

Sprite shows images through the Kitty graphics protocol. Inside tmux, whether
they appear is tmux's decision, not Sprite's.

## The setting

tmux does not forward escape sequences it does not itself understand. To let
them through, set its documented option:

~~~tmux
# ~/.tmux.conf
set -g allow-passthrough on
~~~

Then reload tmux (`tmux source-file ~/.tmux.conf`) or start a new server.

With the option off — which is tmux's default — an image transmitted inside a
tmux pane does not appear. That is tmux working as documented.

## What Sprite deliberately does not do

**Sprite does not patch tmux, override the setting, or detect tmux and work
around it.** A terminal that quietly circumvented a multiplexer's own security
decision would be doing something its user did not ask for, and passthrough
exists precisely so that forwarding arbitrary escape sequences is a choice
somebody makes on purpose.

Both halves are tested: `crates/sprite-term/tests/graphics_tmux.rs` asserts that
an image survives with passthrough on **and** that it does not survive with
passthrough off. The second assertion is as much a promise as the first — it is
what stops a future change from "fixing" tmux from inside Sprite.

## Applications

An application printing an image inside tmux must wrap the sequence itself, in
tmux's format: a DCS `tmux;` sequence with every escape doubled. Most programs
that support tmux do this already. Sprite receives whatever tmux forwards and
treats it exactly as it treats anything else a program prints.
