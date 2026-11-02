use std::collections::HashMap;

use serde_json::{Value, json};

use super::*;
use crate::control::{self, Message, Operation, Request, failure, response};
use crate::local_socket::{LocalSocket, ReplyConnection, SocketKey};

/// What Pane Control keeps for one window: its id, its socket, and the labels
/// callers gave its panes.
pub(super) struct ControlState {
    window: String,
    socket: Option<LocalSocket>,
    labels: HashMap<PaneId, String>,
}

/// Hex digits in a window id. Short enough that `window-<id>.sock` fits a
/// macOS socket path inside `$TMPDIR`, and long enough that two windows never
/// share one.
const WINDOW_ID_HEX: usize = 24;

impl ControlState {
    pub(super) fn new() -> Self {
        let mut window = SocketKey::generate()
            .map(|key| key.to_hex())
            .unwrap_or_default();
        window.truncate(WINDOW_ID_HEX);
        Self {
            window,
            socket: None,
            labels: HashMap::new(),
        }
    }

    /// Opens this window's socket. A window that can't open one still runs;
    /// its panes just can't be reached from outside.
    pub(super) fn open(&mut self, lanes: control::Lanes) {
        let opened = crate::local_socket::runtime_directory().and_then(|directory| {
            control::open_window_socket(&directory, self.window.clone(), lanes)
        });
        self.socket = opened
            .inspect_err(|error| { tracing::warn!(target: "sprite_app::ipc", kind = ?error.kind(), "pane control socket failed"); })
            .ok();
    }

    pub(super) fn window(&self) -> &str {
        &self.window
    }

    #[cfg(test)]
    pub(super) fn socket_path(&self) -> Option<&std::path::Path> {
        self.socket.as_ref().map(LocalSocket::socket_path)
    }

    /// Stops serving and removes the socket, letting `connection` finish the
    /// reply it is writing.
    pub(super) fn close(&mut self, connection: Option<&ReplyConnection>) {
        if let Some(mut socket) = self.socket.take() {
            socket.close_after_reply(connection);
        }
    }
}

impl Workspace {
    fn prepare_control_layout(&mut self, cx: &mut Context<Self>) -> Result<(), &'static str> {
        if matches!(self.mode, Mode::DraggingDivider(_)) {
            return Err("busy");
        }
        self.dismiss_pending_close(cx);
        self.mode = Mode::Idle;
        Ok(())
    }

    /// Forgets the labels of closed panes. Every way of closing a pane ends in
    /// a layout refresh, which calls this, so no close path has to remember it.
    pub(super) fn prune_closed_panes(&mut self) {
        let tabs = &self.tabs;
        self.control
            .labels
            .retain(|pane, _| tabs.get(*pane).is_some());
    }

    pub(super) fn serve_control(
        &mut self,
        message: Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Message {
            request,
            reply,
            connection,
        } = message;
        if !reply.claim() {
            return;
        }
        let answer = self.apply_control(&request, connection.as_ref(), window, cx);
        reply.send(answer).ok();
    }

    fn pane_address(&self, pane: PaneId) -> String {
        format!("{}:{}", self.control.window, pane.0)
    }
    fn parse_control_pane(&self, address: &str) -> Option<PaneId> {
        let (window, pane) = address.split_once(':')?;
        (window == self.control.window)
            .then(|| pane.parse().ok().map(PaneId))
            .flatten()
    }

    fn control_metadata(&self, pane: PaneId, cx: &gpui::App) -> Option<Value> {
        let tab = self.tabs.tab_of(pane)?;
        let tree = self.tabs.tree(tab)?;
        let (_, rect, handle) = tree.layout().into_iter().find(|(id, _, _)| *id == pane)?;
        let (width, height, _) = self.cached_pane_area();
        let allocated = pane_allocation(self.viewport, self.tabs.len(), rect);
        let info = match handle.view().downcast::<TerminalView>() {
            Ok(terminal) => terminal.read(cx).control_info(),
            Err(_) => {
                json!({"kind": "hosted", "terminal_read": false, "terminal_input": false, "native_content": "unavailable"})
            }
        };
        Some(json!({
            "id": self.pane_address(pane), "tab": format!("{}:t{}", self.control.window, tab.0),
            "label": self.control.labels.get(&pane), "child_title": handle.title(cx).map(|s| s.to_string()),
            "tab_name": self.tabs.name(tab), "current": tree.focus() == Some(pane),
            "visible": self.tabs.active_tab() == Some(tab),
            "geometry": {"x": rect.x * width, "y": rect.y * height, "width": f32::from(allocated.width), "height": f32::from(allocated.height)},
            "content": info,
            "capabilities": ["read_metadata", "rename", "move", "focus", "resize_divider", "close"]
        }))
    }

    fn apply_control(
        &mut self,
        request: &Request,
        connection: Option<&ReplyConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Value {
        let id = &request.id;
        if self.stopping {
            return failure(id, "closing");
        }
        if request.protocol != control::PROTOCOL || !request.operation.valid() || id.len() > 64 {
            return failure(id, "invalid_request");
        }
        if request.window != self.control.window {
            return failure(id, "invalid_request");
        }
        let pane = if let Some(address) = request.operation.pane() {
            let Some(pane) = self.parse_control_pane(address) else {
                return failure(id, "invalid_request");
            };
            if self.tabs.get(pane).is_none() {
                return failure(id, "gone");
            }
            Some(pane)
        } else {
            None
        };
        let result: Result<Value, &'static str> = (|| match &request.operation {
            Operation::List => {
                let panes: Vec<_> = self
                    .tabs
                    .all_panes()
                    .into_iter()
                    .filter_map(|(_, pane, _)| self.control_metadata(pane, cx))
                    .collect();
                Ok(json!({"window": self.control.window, "panes": panes}))
            }
            Operation::Read { .. } => {
                let metadata = self
                    .control_metadata(pane.ok_or("invalid_request")?, cx)
                    .ok_or("gone")?;
                if metadata["content"]["terminal_read"] != true {
                    return Err("unsupported");
                }
                Ok(metadata)
            }
            Operation::Create { beside, side } => {
                self.pending_cleanups
                    .retain(|c| !c.completed.load(std::sync::atomic::Ordering::Acquire));
                if self.tabs.all_panes().len() >= control::MAX_PANES
                    || self.pending_cleanups.len() >= control::MAX_CLEANUPS
                {
                    return Err("busy");
                }
                let anchor = beside
                    .as_ref()
                    .map(|s| self.parse_control_pane(s).ok_or("invalid_request"))
                    .transpose()?;
                if anchor.is_some_and(|p| self.tabs.get(p).is_none()) {
                    return Err("gone");
                }
                self.prepare_control_layout(cx)?;
                let old_tab = self.tabs.active_tab();
                let make = make_pane(
                    None,
                    self.active_settings(),
                    None,
                    PaneServices {
                        panes: &self.panes,
                        window: &self.control.window,
                        surfaces: self.surfaces.as_ref(),
                        exit: self.exit_sender.clone(),
                    },
                    window,
                    cx,
                );
                let created = match (anchor, side) {
                    (Some(anchor), Some(side)) => self
                        .tabs
                        .insert_beside(anchor, side.direction(), make)
                        .ok_or("gone")?,
                    (None, None) => {
                        let tab = self.tabs.open(make);
                        self.tabs.tree(tab).and_then(|t| t.focus()).ok_or("busy")?
                    }
                    _ => return Err("invalid_request"),
                };
                if let Some(old) = old_tab {
                    self.tabs.focus_tab(old);
                }
                self.refresh_layout(cx);
                cx.notify();
                self.control_metadata(created, cx).ok_or("gone")
            }
            Operation::Rename { label, .. } => {
                let pane = pane.ok_or("invalid_request")?;
                if let Some(label) = label {
                    self.control.labels.insert(pane, label.clone());
                } else {
                    self.control.labels.remove(&pane);
                }
                cx.notify();
                self.control_metadata(pane, cx).ok_or("gone")
            }
            Operation::Move { beside, side, .. } => {
                let pane = pane.ok_or("invalid_request")?;
                let destination = match (beside, side) {
                    (Some(address), Some(side)) => {
                        let (target_window, _) =
                            address.split_once(':').ok_or("invalid_request")?;
                        if target_window != self.control.window {
                            return Err("unsupported");
                        }
                        let anchor = self.parse_control_pane(address).ok_or("invalid_request")?;
                        if self.tabs.get(anchor).is_none() {
                            return Err("gone");
                        }
                        if anchor == pane {
                            return Err("invalid_request");
                        }
                        Some((anchor, side.direction()))
                    }
                    (None, None) => None,
                    _ => return Err("invalid_request"),
                };
                self.prepare_control_layout(cx)?;
                if !self.tabs.move_pane(pane, destination) {
                    return Err("invalid_request");
                }
                self.refresh_layout(cx);
                cx.notify();
                self.control_metadata(pane, cx).ok_or("gone")
            }
            Operation::Focus { .. } => {
                let pane = pane.ok_or("invalid_request")?;
                self.prepare_control_layout(cx)?;
                self.tabs.focus_tab(self.tabs.tab_of(pane).ok_or("gone")?);
                self.tabs.focus_pane(pane);
                self.refresh_layout(cx);
                self.focus_active_pane(window, cx);
                window.activate_window();
                cx.notify();
                Ok(
                    json!({"pane": self.pane_address(pane), "activation_requested": true, "window_active": window.is_window_active()}),
                )
            }
            Operation::Resize { side, pixels, .. } => {
                if matches!(self.mode, Mode::DraggingDivider(_)) {
                    return Err("busy");
                }
                let pane = pane.ok_or("invalid_request")?;
                let tab = self.tabs.tab_of(pane).ok_or("gone")?;
                let divider = self
                    .tabs
                    .tree(tab)
                    .and_then(|t| t.divider(pane, side.direction()))
                    .ok_or("unsupported")?;
                let (width, height, _) = self.cached_pane_area();
                let extent = match divider.orientation {
                    Orientation::Horizontal => divider.area.width * width,
                    Orientation::Vertical => divider.area.height * height,
                };
                let ratio = divider_ratio(0.0, extent, *pixels as f32, DIVIDER_FLOOR_PX);
                self.tabs.tree_mut(tab).ok_or("gone")?.set_divider_ratio(
                    pane,
                    side.direction(),
                    ratio,
                );
                let accepted = self
                    .tabs
                    .tree(tab)
                    .and_then(|t| t.divider(pane, side.direction()))
                    .ok_or("unsupported")?
                    .ratio
                    * extent;
                self.refresh_layout(cx);
                cx.notify();
                Ok(json!({"pixels": accepted, "pane": self.control_metadata(pane, cx)}))
            }
            Operation::Close { .. } => {
                let pane = pane.ok_or("invalid_request")?;
                self.pending_cleanups
                    .retain(|c| !c.completed.load(std::sync::atomic::Ordering::Acquire));
                if self.pending_cleanups.len() >= control::MAX_CLEANUPS {
                    return Err("busy");
                }
                self.prepare_control_layout(cx)?;
                let tab = self.tabs.tab_of(pane).ok_or("gone")?;
                let handle = self.tabs.close_pane(tab, pane).ok_or("gone")?;
                self.shut_down(handle, cx);
                if self.tabs.is_empty() {
                    self.control.close(connection);
                }
                self.after_close(cx);
                Ok(
                    json!({"pane": request.operation.pane(), "removed": true, "cleanup": "scheduled"}),
                )
            }
            Operation::Input { .. } => {
                let pane = pane.ok_or("invalid_request")?;
                let view = self
                    .tabs
                    .get(pane)
                    .ok_or("gone")?
                    .view()
                    .downcast::<TerminalView>()
                    .map_err(|_| "unsupported")?;
                view.update(cx, |view, _| view.control_input(&request.operation))?;
                self.dismiss_pending_close(cx);
                Ok(
                    json!({"pane": self.pane_address(pane), "input": "admitted", "target": "terminal"}),
                )
            }
            // Answered on the socket thread, through the reload lane.
            Operation::ConfigReload | Operation::ConfigPrint => Err("invalid_request"),
        })();
        cx.notify();
        match result {
            Ok(value) => response(id, "ok", value),
            Err(outcome) => failure(id, outcome),
        }
    }
}

#[cfg(test)]
impl Workspace {
    /// Every pane that has a label.
    pub(super) fn labelled_panes(&self) -> std::collections::BTreeSet<u64> {
        self.control.labels.keys().map(|pane| pane.0).collect()
    }
}
