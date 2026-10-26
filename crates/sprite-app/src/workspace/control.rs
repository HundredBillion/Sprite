use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::*;
use crate::control::{self, AccessRequest, Message, Operation, Request, failure, response};
use crate::local_socket::{LocalSocket, ObservationKey, ReplyConnection};

pub(super) struct ControlState {
    window: String,
    next_grant: u64,
    grants: BTreeMap<u64, Grant>,
    pending: Option<PendingAccess>,
    confirmation: Option<PendingOperation>,
    labels: HashMap<PaneId, String>,
    show_access: bool,
}

struct Grant {
    endpoint: LocalSocket,
    scope: AccessRequest,
}
struct PendingAccess {
    request: AccessRequest,
    reply: Relayed<Value>,
    expires: Instant,
}
struct PendingOperation {
    grant: u64,
    request: Request,
    expires: Instant,
    label: String,
}

impl ControlState {
    pub(super) fn new() -> Self {
        Self {
            window: ObservationKey::generate().map(|key| key.to_hex()).unwrap_or_default(),
            next_grant: 0, grants: BTreeMap::new(), pending: None, confirmation: None,
            labels: HashMap::new(), show_access: false,
        }
    }
    pub(super) fn close(&mut self, connection: Option<&ReplyConnection>) {
        for grant in self.grants.values_mut() { grant.endpoint.close_after_reply(connection); }
        self.grants.clear();
        if let Some(pending) = self.pending.take() { pending.reply.send(failure("access", "closing")).ok(); }
        self.confirmation = None;
    }
    fn permits(&self, grant: u64, pane: PaneId) -> bool {
        self.grants.get(&grant).is_some_and(|g| g.scope.all_panes || g.scope.panes.contains(&pane.0))
    }
}

impl Workspace {
    pub(super) fn expire_control(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        if self.control.pending.as_ref().is_some_and(|p| now >= p.expires) {
            if let Some(pending) = self.control.pending.take() { pending.reply.send(failure("access", "denied")).ok(); }
            cx.notify();
        }
        if self.control.confirmation.as_ref().is_some_and(|p| now >= p.expires) {
            self.control.confirmation = None;
            cx.notify();
        }
    }

    pub(super) fn serve_control(&mut self, message: Message, window: &mut Window, cx: &mut Context<Self>) {
        self.expire_control(cx);
        match message {
            Message::Access { request, reply } => {
                if self.stopping { reply.send(failure("access", "closing")).ok(); return; }
                if self.control.pending.is_some() || self.control.grants.len() >= control::MAX_GRANTS || self.control.window.is_empty() {
                    reply.send(failure("access", "busy")).ok(); return;
                }
                if request.all_panes == !request.panes.is_empty() || request.panes.len() > control::MAX_PANES
                    || request.panes.iter().any(|p| self.tabs.get(PaneId(*p)).is_none()) {
                    reply.send(failure("access", "denied")).ok(); return;
                }
                // The claim remains waiting until the owner decides, so an expired requester cannot gain a grant later.
                self.control.pending = Some(PendingAccess { request, reply, expires: Instant::now() + Duration::from_secs(control::ACCESS_SECONDS) });
                cx.notify();
            }
            Message::Control { grant, request, reply, connection } => {
                if !reply.claim() { return; }
                let answer = self.apply_control(grant, &request, false, connection.as_ref(), window, cx);
                reply.send(answer).ok();
            }
        }
    }

    pub(super) fn answer_access(&mut self, allow: bool, cx: &mut Context<Self>) {
        let Some(pending) = self.control.pending.take() else { return; };
        if !pending.reply.claim() { cx.notify(); return; }
        if !allow || Instant::now() >= pending.expires || self.stopping {
            pending.reply.send(failure("access", "denied")).ok(); cx.notify(); return;
        }
        let grant = self.control.next_grant;
        self.control.next_grant += 1;
        let answer = match control::open_grant(self.control.window.clone(), grant, self.control_sender.clone(), Arc::clone(&self.panes)) {
            Ok((endpoint, descriptor)) => {
                self.control.grants.insert(grant, Grant { endpoint, scope: pending.request });
                response("access", "ok", json!(descriptor))
            }
            Err(_) => failure("access", "busy"),
        };
        pending.reply.send(answer).ok();
        cx.notify();
    }

    pub(super) fn revoke_control(&mut self, grant: u64, cx: &mut Context<Self>) {
        self.control.grants.remove(&grant);
        if self.control.confirmation.as_ref().is_some_and(|p| p.grant == grant) { self.control.confirmation = None; }
        cx.notify();
    }

    fn pane_address(&self, pane: PaneId) -> String { format!("{}:{}", self.control.window, pane.0) }
    fn parse_control_pane(&self, address: &str) -> Option<PaneId> {
        let (window, pane) = address.split_once(':')?;
        (window == self.control.window).then(|| pane.parse().ok().map(PaneId)).flatten()
    }

    fn control_metadata(&self, pane: PaneId, cx: &gpui::App) -> Option<Value> {
        let tab = self.tabs.tab_of(pane)?;
        let tree = self.tabs.tree(tab)?;
        let (_, rect, handle) = tree.layout().into_iter().find(|(id, _, _)| *id == pane)?;
        let (width, height, _) = self.cached_pane_area();
        let info = match handle.view().downcast::<TerminalView>() {
            Ok(terminal) => terminal.read(cx).control_info(),
            Err(_) => json!({"kind": "hosted", "terminal_read": false, "terminal_input": false, "native_content": "unavailable"}),
        };
        Some(json!({
            "id": self.pane_address(pane), "tab": format!("{}:t{}", self.control.window, tab.0),
            "label": self.control.labels.get(&pane), "child_title": handle.title(cx).map(|s| s.to_string()),
            "tab_name": self.tabs.name(tab), "current": tree.focus() == Some(pane),
            "visible": self.tabs.active_tab() == Some(tab),
            "geometry": {"x": rect.x * width, "y": rect.y * height, "width": rect.width * width, "height": rect.height * height},
            "content": info,
            "capabilities": ["read_metadata", "rename", "move", "focus", "resize_divider", "close"]
        }))
    }

    fn apply_control(&mut self, grant: u64, request: &Request, confirmed: bool, connection: Option<&ReplyConnection>, window: &mut Window, cx: &mut Context<Self>) -> Value {
        let id = &request.id;
        if self.stopping { return failure(id, "closing"); }
        if request.protocol != control::PROTOCOL || !request.operation.valid() || id.len() > 64 { return failure(id, "invalid_request"); }
        let Some(permission) = self.control.grants.get(&grant) else { return failure(id, "denied"); };
        if request.window != self.control.window { return failure(id, "denied"); }
        let allowed = match request.operation {
            Operation::List | Operation::Read { .. } => true,
            Operation::Input { .. } => permission.scope.input,
            Operation::Create { .. } | Operation::Rename { .. } | Operation::Move { .. } | Operation::Focus { .. } | Operation::Resize { .. } | Operation::Close { .. } => permission.scope.manage,
        };
        if !allowed { return failure(id, "denied"); }
        let pane = if let Some(address) = request.operation.pane() {
            let Some(pane) = self.parse_control_pane(address) else { return failure(id, "denied"); };
            if !self.control.permits(grant, pane) { return failure(id, "denied"); }
            if self.tabs.get(pane).is_none() { return failure(id, if self.tabs.was_allocated(pane) { "gone" } else { "denied" }); }
            Some(pane)
        } else { None };
        let result: Result<Value, &'static str> = (|| {
            match &request.operation {
                Operation::List => {
                    let panes: Vec<_> = self.tabs.all_panes().into_iter().filter(|(_, pane, _)| self.control.permits(grant, *pane))
                        .filter_map(|(_, pane, _)| self.control_metadata(pane, cx)).collect();
                    Ok(json!({"window": self.control.window, "panes": panes}))
                }
                Operation::Read { .. } => {
                    let metadata = self.control_metadata(pane.ok_or("denied")?, cx).ok_or("gone")?;
                    if metadata["content"]["terminal_read"] != true { return Err("unsupported"); }
                    Ok(metadata)
                }
                Operation::Create { beside, side } => {
                    self.pending_cleanups.retain(|c| !c.completed.load(std::sync::atomic::Ordering::Acquire));
                    if self.tabs.all_panes().len() >= control::MAX_PANES || self.pending_cleanups.len() >= control::MAX_CLEANUPS { return Err("busy"); }
                    let scope = &self.control.grants.get(&grant).ok_or("denied")?.scope;
                    if !scope.all_panes && scope.panes.len() >= control::MAX_PANES { return Err("busy"); }
                    let anchor = beside.as_ref().map(|s| self.parse_control_pane(s).filter(|p| self.control.permits(grant, *p)).ok_or("denied")).transpose()?;
                    if anchor.is_some_and(|p| self.tabs.get(p).is_none()) { return Err("gone"); }
                    let old_tab = self.tabs.active_tab();
                    let make = make_pane(None, self.active_settings(), None, PaneServices {
                        panes: &self.panes, endpoint: self.endpoint.as_ref(), surfaces: self.surfaces.as_ref(), exit: self.exit_sender.clone(),
                    }, window, cx);
                    let created = match (anchor, side) {
                        (Some(anchor), Some(side)) => self.tabs.insert_beside(anchor, side.direction(), make).ok_or("gone")?,
                        (None, None) => { let tab = self.tabs.open(make); self.tabs.tree(tab).and_then(|t| t.focus()).ok_or("busy")? }
                        _ => return Err("invalid_request"),
                    };
                    if let Some(old) = old_tab { self.tabs.focus_tab(old); }
                    if let Some(g) = self.control.grants.get_mut(&grant) { if !g.scope.all_panes { g.scope.panes.insert(created.0); } }
                    self.refresh_layout(cx); cx.notify();
                    self.control_metadata(created, cx).ok_or("gone")
                }
                Operation::Rename { label, .. } => {
                    let pane = pane.ok_or("denied")?;
                    if let Some(label) = label { self.control.labels.insert(pane, label.clone()); } else { self.control.labels.remove(&pane); }
                    cx.notify(); self.control_metadata(pane, cx).ok_or("gone")
                }
                Operation::Move { beside, side, .. } => {
                    let pane = pane.ok_or("denied")?;
                    let destination = match (beside, side) {
                        (Some(address), Some(side)) => {
                            let (target_window, _) = address.split_once(':').ok_or("invalid_request")?;
                            if target_window != self.control.window { return Err("unsupported"); }
                            let anchor = self.parse_control_pane(address).filter(|p| self.control.permits(grant, *p)).ok_or("denied")?;
                            if self.tabs.get(anchor).is_none() { return Err("gone"); }
                            Some((anchor, side.direction()))
                        }
                        (None, None) => None,
                        _ => return Err("invalid_request"),
                    };
                    if !self.tabs.move_pane(pane, destination) { return Err("invalid_request"); }
                    self.refresh_layout(cx); cx.notify(); self.control_metadata(pane, cx).ok_or("gone")
                }
                Operation::Focus { .. } => {
                    let pane = pane.ok_or("denied")?;
                    self.tabs.focus_tab(self.tabs.tab_of(pane).ok_or("gone")?);
                    self.tabs.focus_pane(pane);
                    self.refresh_layout(cx); self.focus_active_pane(window, cx); window.activate_window(); cx.notify();
                    Ok(json!({"pane": self.pane_address(pane), "activation_requested": true, "window_active": window.is_window_active()}))
                }
                Operation::Resize { side, pixels, .. } => {
                    if matches!(self.mode, Mode::DraggingDivider(_)) { return Err("busy"); }
                    let pane = pane.ok_or("denied")?;
                    let tab = self.tabs.tab_of(pane).ok_or("gone")?;
                    let divider = self.tabs.tree(tab).and_then(|t| t.divider(pane, side.direction())).ok_or("unsupported")?;
                    let (width, height, _) = self.cached_pane_area();
                    let extent = match divider.orientation { Orientation::Horizontal => divider.area.width * width, Orientation::Vertical => divider.area.height * height };
                    let ratio = divider_ratio(0.0, extent, *pixels as f32, DIVIDER_FLOOR_PX);
                    self.tabs.tree_mut(tab).ok_or("gone")?.set_divider_ratio(pane, side.direction(), ratio);
                    let accepted = self.tabs.tree(tab).and_then(|t| t.divider(pane, side.direction())).ok_or("unsupported")?.ratio * extent;
                    self.refresh_layout(cx); cx.notify();
                    Ok(json!({"pixels": accepted, "pane": self.control_metadata(pane, cx)}))
                }
                Operation::Close { .. } => {
                    let pane = pane.ok_or("denied")?;
                    let handle = Rc::clone(self.tabs.get(pane).ok_or("gone")?);
                    if !confirmed && let Some(warning) = handle.close_warning(cx) {
                        let program = warning.program.map(|s| s.to_string()).unwrap_or_else(|| "a running program".into());
                        return self.confirm_control(grant, request, format!("Close pane {} and stop {program}?", pane.0));
                    }
                    self.pending_cleanups.retain(|c| !c.completed.load(std::sync::atomic::Ordering::Acquire));
                    if self.pending_cleanups.len() >= control::MAX_CLEANUPS { return Err("busy"); }
                    let tab = self.tabs.tab_of(pane).ok_or("gone")?;
                    let handle = self.tabs.close_pane(tab, pane).ok_or("gone")?;
                    self.control.labels.remove(&pane);
                    self.shut_down(handle, cx);
                    if self.tabs.is_empty() { self.control.close(connection); }
                    self.after_close(cx);
                    Ok(json!({"pane": request.operation.pane(), "removed": true, "cleanup": "scheduled"}))
                }
                Operation::Input { .. } => Err("unsupported"),
            }
        })();
        cx.notify();
        match result { Ok(value) => response(id, "ok", value), Err(outcome) => failure(id, outcome) }
    }

    fn confirm_control(&mut self, grant: u64, request: &Request, label: String) -> Result<Value, &'static str> {
        if self.control.confirmation.is_some() { return Err("confirmation_required"); }
        self.control.confirmation = Some(PendingOperation { grant, request: request.clone(), expires: Instant::now() + Duration::from_secs(30), label });
        Err("confirmation_required")
    }

    pub(super) fn answer_control(&mut self, allow: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pending) = self.control.confirmation.take() {
            if allow && Instant::now() < pending.expires {
                self.apply_control(pending.grant, &pending.request, true, None, window, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn control_elements(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let mut elements = Vec::new();
        if !self.control.grants.is_empty() {
            elements.push(div().absolute().top(px(0.)).right(px(0.)).px(px(8.)).bg(rgb(TAB_ACTIVE_BG)).text_color(rgb(TAB_ACTIVE_FG))
                .child("Pane Access").on_mouse_down(gpui::MouseButton::Left, cx.listener(|workspace, _, _, cx| { workspace.control.show_access = !workspace.control.show_access; cx.notify(); })).into_any_element());
        }
        let mut panel = div().absolute().top(px(32.)).left(px(12.)).right(px(12.)).p(px(12.)).flex().flex_col().gap(px(8.)).bg(rgb(CONFIRM_BG)).text_color(rgb(CONFIRM_FG)).occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
        let mut visible = false;
        if let Some(pending) = &self.control.pending {
            visible = true;
            panel = panel.child(format!("Pane access requested (claimed requester: {:?}). Scope: {}. Read: yes. Manage: {} (starts shells). Input: {} (can execute commands).", pending.request.from,
                if pending.request.all_panes { "all existing and future panes".to_owned() } else { format!("panes {:?}", pending.request.panes) }, pending.request.manage, pending.request.input))
                .child(div().child("Allow").on_mouse_down(gpui::MouseButton::Left, cx.listener(|workspace, _, _, cx| workspace.answer_access(true, cx))))
                .child(div().child("Deny").on_mouse_down(gpui::MouseButton::Left, cx.listener(|workspace, _, _, cx| workspace.answer_access(false, cx))));
        }
        if let Some(pending) = &self.control.confirmation {
            visible = true;
            panel = panel.child(pending.label.clone())
                .child(div().child("Confirm").on_mouse_down(gpui::MouseButton::Left, cx.listener(|workspace, _, window, cx| workspace.answer_control(true, window, cx))))
                .child(div().child("Cancel").on_mouse_down(gpui::MouseButton::Left, cx.listener(|workspace, _, window, cx| workspace.answer_control(false, window, cx))));
        }
        if self.control.show_access {
            visible = true;
            for (id, grant) in &self.control.grants {
                let id = *id;
                panel = panel.child(format!("Grant {id}: {}; manage {}; input {}", if grant.scope.all_panes { "all panes" } else { "selected panes" }, grant.scope.manage, grant.scope.input))
                    .child(div().child("Revoke access").on_mouse_down(gpui::MouseButton::Left, cx.listener(move |workspace, _, _, cx| workspace.revoke_control(id, cx))))
                    .child(div().child("Remove management").on_mouse_down(gpui::MouseButton::Left, cx.listener(move |workspace, _, _, cx| { if let Some(grant) = workspace.control.grants.get_mut(&id) { grant.scope.manage = false; } cx.notify(); })))
                    .child(div().child("Remove input").on_mouse_down(gpui::MouseButton::Left, cx.listener(move |workspace, _, _, cx| { if let Some(grant) = workspace.control.grants.get_mut(&id) { grant.scope.input = false; } cx.notify(); })));
            }
        }
        if visible { elements.push(panel.into_any_element()); }
        elements
    }
}
