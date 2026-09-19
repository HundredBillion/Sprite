# Plugin Surface Support Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an ordinary foreground program or an existing grid client host an owned, resizable, SVG-backed virtual list in a Sprite pane.

**Architecture:** Extend the existing authenticated Surface Channel with discovery and owned docks. Add a root-only virtual list using GPUI's existing uniform-list and scroll handles. Keep process-group checks in sprite-term, pure list validation in surface/list.rs, and GPUI drawing in surface/list_view.rs.

**Tech Stack:** Rust 1.97.1, GPUI =0.2.2, serde_json =1.0.151, existing Unix PTY facilities; no new runtime dependency.

**PRD:** [SVGTree native Explorer](../PRDs/09-18-2026-svgtree-native-explorer.md).
**Interface authority:** [Shared contract](09-18-2026-native-explorer-contract.md).

## Global Constraints

- Sprite's Rust code and manifest must not depend on Neovim, SVGTree, Material Icon Theme, or filesystem-tree business rules.
- Preserve the authenticated NDJSON protocol and generic Surface boundaries.
- Current limits are 4,096 described elements, nesting depth 32, and 16 MiB per channel message.
- Native features do not depend on Kitty detection, terminal image transmission, raster conversion, or a nightly-only API.
- Retain Linux/macOS coverage and existing grid/element-client behavior.
- No thread::sleep, Timer::after, or request_animation_frame under crates/sprite-app, including tests.
- The common contract defines every message name, field, limit, and error behavior consumed by later plans.
- Make an isolated worktree at execution time; preserve the unrelated main-worktree edits to THIRD-PARTY-NOTICES.md and packaging/installed.png.

## File responsibilities

| File | Responsibility |
| --- | --- |
| `crates/sprite-term/src/foreground.rs` | Check a candidate PID against the live foreground group |
| `crates/sprite-term/src/lib.rs` | Expose that check through Session |
| `crates/sprite-app/src/surface/channel.rs` | Decode discovery/list messages; write acknowledgements/events |
| `crates/sprite-app/src/surface/list.rs` (new) | Pure row/state/asset validation and atomic updates |
| `crates/sprite-app/src/surface/list_view.rs` (new) | GPUI virtual list, SVG images, styling, scroll and hit testing |
| `crates/sprite-app/src/surface/description.rs` | Parse the root-only list configuration |
| `crates/sprite-app/src/surface.rs` | Export new modules |
| `crates/sprite-app/src/terminal_view/surfaces.rs` | Ownership, focus, list hosting, dock resizing |
| `crates/sprite-app/src/terminal_view/geometry.rs` | Shared dock-width clamping |
| `crates/sprite-app/src/workspace.rs` | Route new SurfaceRequest variants |
| `scripts/surface-list-demo.py` (new) | Real-server protocol/interaction fixture using Python stdlib |
| `README.md` | Document optional discovery, owned docks and virtual lists |

## Task 1: Foreground ownership and side-effect-free discovery

**Interfaces:** Produce `ForegroundWatch::owner_group(pid: u32) -> Option<i32>` and
`Session::foreground_owner_group(pid: u32) -> Option<i32>`. Reuse
`pty_unix::{foreground_group,process_group_of}`. Add
`SurfaceRequest::Capabilities { pane, owner_pid, return_target, reply }` where
reply carries `Result<serde_json::Value, Refusal>`. This query creates no Surface.

- [ ] Add an unattached-watch assertion and a real-PTY foreground test in sprite-term's existing test modules. The latter must observe acceptance while the process is foreground and rejection when its job is suspended or the wrong PID is supplied.

```rust
#[test]
fn owner_is_unknown_until_the_pty_is_attached() {
    let watch = ForegroundWatch::default();
    assert_eq!(watch.owner_group(std::process::id()), None);
}
```

- [ ] Run `cargo test -p sprite-term --locked --offline owner_`; expect the new API test to fail before implementation. Keep real-PTY tests under sprite-term, where existing lifecycle tests already exercise job control.
- [ ] Add the ownership method using the existing private PTY duplicate:

```rust
pub fn owner_group(&self, pid: u32) -> Option<i32> {
    let attached = self.attached.get()?;
    let foreground = pty_unix::foreground_group(&attached.master)?;
    let candidate = pty_unix::process_group_of(pid)?;
    (foreground == candidate).then_some(candidate)
}
```

Expose it through Session without exposing the PTY fd. Do not reject the shell
group categorically: Sprite can launch a program directly rather than via shell.

Task 1 advertises no feature flags. Until Task 2 adds fill ownership metadata,
Surface return targets are ineligible; terminal targets still require no fill.
Task 2 completes live owned-fill validation and advertises owned-dock-v1;
Tasks 3–4 complete and advertise svg-assets-v1 and virtual-list-v1 respectively;
Task 5 advertises dock-resize-v1.

- [ ] Extend channel first-message dispatch with the exact capabilities request/response in the common contract. Parse finite safe integer fields; validate pane/return target on the GPUI thread via workspace routing. Implement a separate JSON-reply exchange rather than changing existing `Reply`/`one_shot` consumers.
- [ ] Add channel tests using the existing listener/converse harness: successful discovery emits no Open request; wrong auth reveals nothing; unsupported version, unknown pane, nonforeground owner and stale return target are refused. Old open/focus/token tests remain unchanged.
- [ ] Run `cargo test -p sprite-term --locked --offline` and `cargo test -p sprite-app --locked --offline surface::channel`. Commit only these implementation files and their tests as `feat: discover plugin surface capabilities without opening a surface`.

## Task 2: Owned dock lifecycle and return focus

**Interfaces:** Extend Open with optional ownership metadata and resizable flag.
Store `Owner { pid:u32, group:i32, return_target:FocusTarget }` on owned docks;
store optional PID/group on fill surfaces. Produce a private
`TerminalView::validate_surface_owner(id) -> Result<(), Refusal>` that delegates
to its Session and verifies the target belongs to the same pane and group.
No plugin name enters these types.

- [ ] Add tests for terminal-target and fill-target eligibility, an occupied dock, malformed partial ownership, no-foreground/unknown ownership, and a return id naming a dock instead of a fill. Refuse before placement or focus changes.
- [ ] Run `cargo test -p sprite-app --locked --offline surface`; expect the new ownership cases to fail against old parsing/hosting.
- [ ] Extend parse_open and HostedSurface initialization; keep every old constructor/test valid by explicit None/false fields. Use the ownership check again at open, not a cached capabilities result.
- [ ] Extend close_surface: a focused owned dock returns to its valid declared target; a blurred dock closes without changing focus. A fill closes dependent docks before dropping its focus handle. Connection failure and owner invalidation use this same cleanup path.
- [ ] Guard Surface input dispatch and focus transitions with owner validation. Discard the event that discovered an invalid owner. Revalidate on terminal-driven paints without creating an idle polling loop. Preserve existing overlay previous-focus behavior and legacy dock behavior.
- [ ] Exercise real Surface clients with two panes and two fill ids: cross-pane focus is refused; deleting the owning fill removes its dock; suspending an ordinary terminal client does not deliver keys to it or the shell through a stale handler. Record interactive checks separately from unit results.
- [ ] Run `cargo test -p sprite-app --locked --offline`; commit as `feat: bind plugin docks to their owner and editor focus target`.

## Task 3: Validated list model and connection-scoped SVG assets

**Interfaces:** Add these types and methods in `surface/list.rs`:

```rust
pub struct ListRow {
    pub id: String,
    pub text: String,
    pub indent: f32,
    pub icon: Option<String>,
    pub leading: Option<String>,
    pub guides: Vec<f32>,
}
pub struct ScrollAnchor { pub id: String, pub offset: f32 }
pub enum ListOp {
    Assets(std::collections::BTreeMap<String, String>),
    Rows { revision: u64, rows: Vec<ListRow>, selected: Option<String> },
    State { revision: u64, patch: serde_json::Value },
}
pub struct ListModel {
    pub revision: u64,
    pub rows: std::sync::Arc<Vec<ListRow>>,
    pub selected: Option<String>,
    pub status: Option<String>,
    pub assets: std::collections::BTreeMap<String, String>,
    pub scroll: Option<ScrollAnchor>,
    pub reveal: Option<String>,
}
```

Implement `Default`, `parse_op(&Value) -> Result<ListOp,Refusal>`,
`ListModel::apply(ListOp) -> Result<(),Refusal>`, and
`ListModel::index_of(&str) -> Option<usize>`. Maintain a private id-index map
rebuilt on Rows, so selected-row lookup does not scan 100000 rows per frame.

- [ ] Add atomicity and stale-revision tests, starting with:

```rust
#[test]
fn a_stale_state_cannot_select_a_new_occupant() {
    let mut model = ListModel::default();
    let rows = serde_json::json!({"type":"list_rows","revision":2,
        "rows":[{"id":"new","text":"new","indent":0}],"selected":null});
    model.apply(parse_op(&rows).unwrap()).unwrap();
    let stale = serde_json::json!({"type":"list_state","revision":1,
        "selected":"new"});
    assert!(model.apply(parse_op(&stale).unwrap()).is_err());
    assert_eq!(model.selected, None);
}
```

- [ ] Run `cargo test -p sprite-app --locked --offline surface::list`; expect missing module/types before implementation.
- [ ] Implement validation exactly as in the shared contract. Use `is_finite`, checked numeric conversions, unique ids, and all-or-nothing mutation. Distinguish absent patch fields from JSON null. Add tests for negative/huge dimensions, duplicate ids, bad assets, asset redefinition, over-budget assets, 10000 rows, nonexistent selection, simultaneous reveal+scroll, revision bounds, and malformed state retaining old data.
- [ ] Extend description.rs with Kind::VirtualList and a typed ListConfig field; require root position and prohibit nested children/grid/text payload combinations for this kind. Keep general element limits unchanged. Add color-reference parsing for the ten list roles and range validation for every metric/header, including optional header font_size, font_weight, left_padding and icon_gap from the shared contract.
- [ ] Acknowledge description update on owned Surfaces with applied.operation=update. Preserve old clients' no-ack behavior. For Body::List, a valid virtual_list description reconfigures metrics/headers/colors without replacing row/asset/revision state; reject a root-kind change atomically.
- [ ] Add `SurfaceRequest::List {id,pane,op}` and operation dispatch alongside grid operations. Add exact `applied`, `list_click`, `list_action`, `list_scroll`, and `dock_size` writers with type first. A refused operation leaves the model usable. All lists start at revision zero with no actionable rows.
- [ ] Run `cargo test -p sprite-app --locked --offline surface`; commit as `feat: add atomic virtual list operations and reusable SVG assets`.

## Task 4: GPUI virtual-list drawing and scrolling

**Interfaces:** Create `VirtualListView` as a GPUI Entity owned by
`Body::List { root: Element, view: Entity<VirtualListView> }`. It owns ListModel,
UniformListScrollHandle, cached Arc<Image> assets, header layout, and the last
reported scroll anchor. Expose `new(config,connection)`,
`apply(op, cx) -> Result<(),Refusal>`, and `set_focused(bool,cx)`; the hosted
Surface remains the keyboard focus owner.

- [ ] Add pure tests for anchor preservation and scrollbar geometry. Define the helper result as `(thumb_top:f32,thumb_height:f32)` and cover zero rows, a full viewport, first/last positions, and a tiny viewport.

```rust
pub fn scrollbar_geometry(total: f32, viewport: f32, offset: f32) -> (f32, f32) {
    if total <= viewport || viewport <= 0.0 { return (0.0, viewport.max(0.0)); }
    let thumb = (viewport * viewport / total).max(18.0).min(viewport);
    let top = offset.clamp(0.0, total - viewport)
        / (total - viewport) * (viewport - thumb);
    (top, thumb)
}
```

- [ ] Use GPUI 0.2.2 `uniform_list` directly, with one stable element id per Surface, an Arc row vector captured by the range callback, explicit row height, and its persistent scroll handle. Draw only requested rows. Avoid cloning all SVG strings or rows while painting.

```rust
let rows = std::sync::Arc::clone(&self.model.rows);
let list = gpui::uniform_list("rows", rows.len(), move |range, window, cx| {
    range.map(|index| render_row(&rows[index], window, cx)).collect::<Vec<_>>()
}).track_scroll(self.scroll.clone()).h_full();
```

`render_row` is a private closure capturing resolved palette/metrics, cached
images, selection and connection; it returns AnyElement. Build a flex row with
leading slot, SVG icon, truncated label, and absolute guide lines. Use id/revision
captured from this frame in click handlers, `ClickEvent::click_count()`, and
existing modifier formatting. Only left-click activates; no file paths are
interpreted. Hover is GPUI styling, not a socket round trip.

- [ ] Draw heading and section at configured heights, and status/search text in a single bottom row only when status is present. Reserve its height in viewport calculations. Use the system UI font and the configured exact pixel metrics. Resolve all colors from TokenRegistry each render so explicit theme overrides work.
- [ ] Reuse `scroll.0.borrow().base_handle` for wheel, thumb dragging, and precise saved offsets. Query `logical_scroll_top()` on that base handle, not the uniform-list test-only convenience method. On reveal use `scroll_to_item` with non-strict strategy; restore uses the exact pixel offset. Report changed anchor/count during normal paint without scheduling a new repaint just to send the event.
- [ ] Integrate Body::List in all exhaustive matches in surfaces.rs. Do not let the wrapper consume wheel input before uniform_list. Keep the wrapper swallowing propagation into the terminal after the list handles it. Native rows use the existing Surface input/IME/paste path.
- [ ] Add `scripts/surface-list-demo.py`: authenticate using existing environment, discover, open a 10000-row list, send one shared SVG asset and rows, print events, and close on EOF/interrupt. Use json/socket/select from Python stdlib. Add a `--check` mode asserting exact applied replies and sending stale state to expect refusal without closing the Surface.
- [ ] Run model/channel tests and the demo against an actual Sprite pane; verify scrollbar drag, wheel, selected/inactive/hover colors, click counts, empty list, and narrow sidebar. Commit as `feat: draw and scroll native virtual lists with GPUI`.

## Task 5: Resize the dock without disturbing focus

**Interfaces:** Add `DockDrag { id:SurfaceId, side:Side, start_x:f32,
start_width:f32 }` as transient TerminalView state. Reuse existing split-divider
gesture patterns in workspace.rs; use a separate dock drag so pane divider
behavior remains unchanged. Expose `resize_dock(id,width,cx)` privately.

- [ ] Add a pure width helper and tests for both sides and clamps:

```rust
fn dragged_width(start: f32, delta: f32, left: bool, limit: f32) -> f32 {
    let wanted = start + if left { delta } else { -delta };
    wanted.clamp(64.0_f32.min(limit), limit.min(4096.0).max(0.0))
}
```

- [ ] Render a 4-pixel edge hit area only for resizable docks. Left-down records the drag and stops propagation; movement updates preferred width and invalidates terminal dimensions; release and release-out clear it. Use resize cursor, preserve current focus, and never dispatch a row action from the divider.
- [ ] Send dock_size only for user drag changes. Keep existing resize events for actual allocation changes; when the pane shrinks and expands, a saved requested width is restored within limits.
- [ ] Run tests plus real list-demo dragging on left and right. Verify ordinary PTY resize and fill-grid resize with both one and two docks. Commit as `feat: resize native docks by dragging their edge`.

## Task 6: Protocol fixture and regression gates

- [ ] Update README with discovery, ownership, the list demo, refusal behavior and unchanged legacy protocol. Document limits and expected client cleanup.
- [ ] Add a protocol fixture under `tests/fixtures/surface-list-v1.json` containing the shared-contract example messages and expected success/refusal events. Load it in channel/model tests using `include_str!`; the Lua plans consume the same file through a caller-supplied Sprite checkout path.
- [ ] Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, and `cargo test --workspace --locked --offline --no-fail-fast`. Run the existing CI Forbidden states commands from `.github/workflows/ci.yml`. If an offline dependency is missing, fetch through the authorized network path and rerun the same gate; do not silently drop `--locked`.
- [ ] Record real Linux interaction evidence and obtain the real macOS run before declaring cross-platform acceptance. Headless parser results do not prove drawing or mouse behavior. Document unavailable GUI evidence as pending, not passed.
- [ ] Commit evidence/docs as `test: verify plugin surface protocol and native list behavior`.

## Coverage and hardening

Task 1 covers support discovery; Task 2 covers both editor presentations and
focus/lifecycle; Tasks 3–4 cover bounded rows/assets, state, styling and events;
Task 5 covers width and editor resize; Task 6 covers old-client and platform
regressions. This plan intentionally does not implement tree navigation or ship
Material data in Sprite. Those are owned by the dependent Lua plans.

The view is a deep module because scrolling, asset reuse and rendering state
stay behind `apply` rather than leaking into the plugin. Legacy description
updates remain separate from list operations, mirroring the grid precedent.
