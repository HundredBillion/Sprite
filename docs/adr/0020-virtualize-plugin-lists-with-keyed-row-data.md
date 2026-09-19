# Virtualize plugin lists with keyed row data

Plugin lists use a root-only generic virtual-list Surface, with stable row ids,
model revisions, and connection-scoped SVG assets. GPUI draws only the rows in
view; selection and scrolling use small state updates rather than replacing a
large element tree.

Sending every expanded file as nested elements would hit the 4096-element
limit, repeat SVG payloads, and couple scroll state to changing element order.
Raising that limit would retain those costs. A keyed list adds a public protocol
contract but keeps scrolling and asset reuse inside Sprite while filesystem
meaning remains with the client. The existing grid Surface provides the
precedent for a root widget with its own validated operations.

Decided while hardening the native Explorer TSPs on 2026-09-18. The message
contract is in `docs/TSPs/09-18-2026-native-explorer-contract.md`.
