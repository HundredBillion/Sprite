# Cancel local listeners through private descriptors

A LocalSocket owns a private cancellation socket pair and waits on its listener and cancellation read descriptor through the existing nix readiness facility. Closing the writer wakes cancellation even when the public socket pathname has been removed or replaced. Connecting to the pathname cannot guarantee listener shutdown, and detaching a stuck listener trades a window freeze for a leaked thread.

This deepens the shared local transport without changing observation/Surface grammars or adding threads, periodic polling or a runtime. nix 0.28.0 is already a workspace dependency; sprite-app names it directly for poll. Authentication, connection accounting and scoped reply exemption remain adapter-independent.
