# Bound Surface raster memory at the decoder

Both element images and list assets pass through one SVG raster entry. It validates positive finite scaled dimensions, limits each dimension to 4096 and each bitmap to 16MiB before allocation. Each Surface cache retains at most 64MiB of decoded pixels. Encoded-document limits cannot enforce this invariant: a small SVG can describe huge intrinsic dimensions or extreme aspect ratios. Rejected rasters are omitted using the existing absent-image behavior.

These limits preserve normal icons while refusing previously accepted oversized images; adding unrestricted widget-specific decoders would recreate the failure. They bound retained bitmap data, not every usvg parse/filter allocation, CPU time, native texture copy, or aggregate memory across windows. A renderer sandbox/global memory manager would cover more failure classes but add a separate subsystem beyond the confirmed raster defect.

## Amendment, October 2026

Decoding is asynchronous, and filter work is bounded by counting the uses of filter primitives.

Neither cache parses or rasterises SVG on the UI thread. An image is `Pending` from the first frame that draws it until its decode finishes on GPUI's background executor. A pending image draws nothing, and the finished decode tells the view that drew it to draw again. Each cache runs at most `MAX_CONCURRENT_SVG_DECODES` (2) decodes at a time, oldest request first. The memory that filter intermediates take is therefore bounded per cache by two capped decodes, not by how many images a description holds. The font database is first loaded by a decode, so it too is loaded on the background executor.

The 64MiB cache budget is charged when a finished decode is installed. A result that no longer fits is dropped, and a result whose image the description no longer draws is discarded. An element Surface forgets a refusal for lack of budget when its next description arrives, and tries again. A document that does not parse, applies too many filter primitives, or cannot be one raster stays refused for as long as a description draws it. A list keeps every outcome until its icon raster width changes, as before.

Before rasterising, a decode walks the parsed tree, including clip paths, masks, patterns, nested images and text. It adds up the primitives of every filter each group applies, so a filter shared by two groups counts twice. A document with more than `MAX_FILTER_PRIMITIVE_USES` uses is refused. The cap is the largest measured count whose 512x512 document, made of differently named `feFlood` results merged by one `feMerge`, decodes in under 250 ms in a release build: `MAX_FILTER_PRIMITIVE_USES` = 161; a 512x512 document of 160 merged flood results (161 uses) decodes in 229 ms (median of 3 runs of 5, release build, macos/aarch64). Medians for 33, 65, 97, 129, 161, 193 and 257 uses were 47, 92, 139, 178, 229, 277 and 359 ms. The machine was under background load, so the cap is conservative. The measurement is a manual run of the ignored test `measure_filter_cap`. No test asserts on time.

The cap was calibrated on a 512x512 canvas. A document at the cap on a much larger canvas, still within the 4096 px and 16 MiB raster limits, can take longer than 250 ms. That is accepted because decoding is off the UI thread and at most two decodes run per cache.

Parse cost is bounded by the existing Surface Channel message limit (16MiB) and list asset limit (64MiB), not by the cap. CPU time spent on content other than filters, and memory across Surfaces, remain outside this decision.

Memory across Surfaces is now bounded by ADR 0033, which replaces this deferral.

Amended 2026-10-27 by ADR 0035: a retained picture's native texture copy is
now released with the picture, so that copy is no longer outside this bound.
