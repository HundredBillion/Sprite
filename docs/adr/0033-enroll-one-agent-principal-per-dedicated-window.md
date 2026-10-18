# Enroll one agent principal per dedicated window

The owner approved explicit program launch into a dedicated agent Sprite Window,
with grants chosen at launch and a native Revoke access control. Initially the
enrolled agent and its child processes share one principal; separately authorized
agents or mixed human/agent panes within one window require a later design.

This retains ADR 0013's window boundary and existing observation/Surface formats.
It does not turn legacy credentials into general control authority. Agent windows
apply their grant policy to every exposed path, including queued Surface work and
configuration requests. Human-window behavior stays compatible. This is application
authority, not an OS sandbox against arbitrary same-user processes.
