# Platform Copy, Paste, and Selection

**Date:** September 18, 2026  
**Status:** Approved design

## Problem

Sprite recognizes only `Ctrl+Shift+C` and `Ctrl+Shift+V` as application copy
and paste shortcuts. That differs from the platform shortcuts people use in
ordinary macOS and Linux applications. Sprite also copies a terminal selection
as soon as a pointer drag ends. On a Mac trackpad this makes highlighting and
copying behave unlike the normal select-then-copy interaction.

## Goal

Make terminal selection and clipboard commands predictable on macOS and Linux:

| Action | macOS | Linux | Compatibility binding |
|---|---|---|---|
| Copy the terminal selection | `Cmd+C` | `Super+C` | `Ctrl+Shift+C` |
| Paste clipboard text | `Cmd+V` | `Super+V` | `Ctrl+Shift+V` |

A pointer or trackpad drag creates and preserves a selection. Releasing the
pointer does not change the clipboard. The person explicitly copies that
selection with either supported copy binding.

## Requirements

1. Sprite recognizes platform-modified `C` and `V` as copy and paste. GPUI's
   `platform` modifier represents Command on macOS and Super on Linux.
2. Sprite continues to recognize `Ctrl+Shift+C` and `Ctrl+Shift+V`.
3. A shortcut is accepted only with its exact modifier family. Adding Alt or
   mixing the platform modifier with Control or Shift does not trigger a
   clipboard command.
4. Plain `Ctrl+C` and `Ctrl+V` continue to reach the terminal child.
5. Copy with a selection uses the existing `CopySelection` command and
   `SelectionCopied` event. Copy with no selection leaves the clipboard alone.
6. Paste keeps the existing bracketed-paste and unsafe-paste protections. A
   second invocation of either paste binding confirms a held paste.
7. Ending a character-selection drag preserves the visible selection and sends
   no copy command.
8. Mouse reporting behavior remains unchanged. When a terminal child owns the
   pointer, Shift remains the override that gives selection back to Sprite.
9. The README and the unsafe-paste status text describe both shortcut families
   without claiming that one concrete platform key applies everywhere.

## Design

The change stays in the existing terminal input boundary. The shortcut resolver
classifies two exact modifier combinations before mapping `c` or `v` to the
existing `Shortcut` enum:

- `platform` alone
- `control + shift` together

All other combinations return no application shortcut and follow the existing
terminal key path.

The pointer-release handler ends Sprite's drag state but does not request the
selection text. Selection construction during pointer movement is unchanged.
Explicit copy remains the only application path that sends `CopySelection`.

No configuration setting, new action framework, dependency, or Terminal Core
protocol change is needed.

## Verification

Automated tests cover:

- `Cmd/Super+C` and `Cmd/Super+V` resolving to copy and paste;
- `Ctrl+Shift+C` and `Ctrl+Shift+V` remaining supported;
- plain Control, plain Shift, Alt combinations, and mixed platform modifiers
  remaining unclaimed;
- drag release ending the gesture without requesting a copy;
- the existing Terminal Core selection and paste tests continuing to pass.

Manual acceptance on macOS:

1. Drag over terminal text with a trackpad. The text remains highlighted after
   release and the clipboard is unchanged.
2. Press `Cmd+C`. The highlighted text appears on the clipboard.
3. Press `Cmd+V`. Clipboard text is pasted through the existing safety path.
4. Repeat with `Ctrl+Shift+C` and `Ctrl+Shift+V`.
5. Run a command and press plain `Ctrl+C`; the terminal child receives the
   interrupt.

Manual acceptance on Linux repeats the same checks with `Super+C` and
`Super+V`, plus the compatibility bindings.

## Out of Scope

- User-configurable keybindings.
- Primary-selection or copy-on-select behavior.
- Changes to selection modes such as word, line, or rectangular selection.
- Clipboard behavior inside hosted Surfaces beyond their existing paste
  routing.
