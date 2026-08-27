# Limit Ghostty patches to exposing existing library behavior

Sprite may carry small, deterministic, independently tested patches that expose
behavior already implemented by the pinned Ghostty source through libghostty,
and should offer those patches upstream. It will not bypass Ghostty's thread or
allocator ownership, add unsafe `Send`/`Sync`, or alter parsing and terminal
semantics; any capability that requires those changes stops its checkpoint for
architectural review instead of growing an undocumented permanent fork.
