# Bound Surface raster memory at the decoder

Both element images and list assets pass through one SVG raster entry. It validates positive finite scaled dimensions, limits each dimension to 4096 and each bitmap to 16MiB before allocation. Each Surface cache retains at most 64MiB of decoded pixels. Encoded-document limits cannot enforce this invariant: a small SVG can describe huge intrinsic dimensions or extreme aspect ratios. Rejected rasters are omitted using the existing absent-image behavior.

These limits preserve normal icons while refusing previously accepted oversized images; adding unrestricted widget-specific decoders would recreate the failure. They bound retained bitmap data, not every usvg parse/filter allocation, CPU time, native texture copy, or aggregate memory across windows. A renderer sandbox/global memory manager would cover more failure classes but add a separate subsystem beyond the confirmed raster defect.
