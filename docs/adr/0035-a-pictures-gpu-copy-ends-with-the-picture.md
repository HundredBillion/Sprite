# A picture's GPU copy ends with the picture

Approved for issue #140 on 2026-10-27. Amends ADRs 0022 and 0033.

GPUI uploads an image to a window's texture atlas the first time it is drawn
and keeps it until the application drops it, keyed by an identity that every
`RenderImage::new` mints afresh. Sprite never dropped one. Replacing a
512×512 Kitty image 60 times took `IOAccelerator (graphics)` from 11.5 to
66.7 MB, and none of it came back after the image was deleted. Every image
cache had the same gap: Kitty graphics, Surface element pictures, tab icons,
list icons and Markdown images.

Every cache now holds a `Picture`, the only code that makes a `RenderImage`
(a test enforces this). When the last handle to a picture goes, its image joins
a queue, and each window empties the queue through `App::drop_image` at the
start of its next render, before anything paints. Every way a cache lets go
(replacement, eviction, clearing, a closed pane or Surface) is covered without
each path remembering to release, and a cache added later inherits it.

The queue is thread-local, like the ADR 0033 ledger. Only a thread that has
drawn queues anything: a picture let go elsewhere never had a GPU copy, and
holding its pixels in a queue nobody empties would leak them.

GPUI's Metal atlas also kept a removed image's space until its whole texture
emptied. The GPUI patch returns the tile to the texture's allocator and makes
removal safe to repeat.

Consequences: a picture's GPU copy outlives it by at most one frame of the
window that drew it. ADRs 0022 and 0033 bound decoded pixels and excluded
"native texture copies". That exclusion no longer holds for retained pictures;
it still holds for usvg parse allocations and CPU time.

Rejected: calling `drop_image` at each eviction site. It is the scattered
pattern that caused the leak, and the next cache would forget it.
