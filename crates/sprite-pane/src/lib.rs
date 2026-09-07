//! The pane interface: what Sprite Terminal needs from anything that occupies
//! a pane, and nothing about what that thing is.
//!
//! Two traits, on purpose. [`Pane`] is the one a pane type implements; it may
//! demand `Render` and `Focusable` and is not object-safe, which does not
//! matter because nothing stores a `Pane`. [`PaneHandle`] is object-safe and is
//! implemented once, on `gpui::Entity<V>` for every `V: Pane`, so the workspace
//! can hold panes of different types in one list. The trait object lives around
//! the handle rather than inside it because `Entity<dyn Pane>` cannot be built:
//! the entity arena keys on a concrete `TypeId`. ADR 0016 records the choice.
//!
//! This crate depends on `gpui` and nothing else. That is the dependency
//! invariant — Sprite names no editor — in the one form a tool can check.

use gpui::{AnyView, App, FocusHandle, Focusable, Pixels, Render, SharedString, Size};

/// What Sprite needs from anything that occupies a pane.
///
/// Every member has a caller in the workspace. Members with no caller are not
/// added on speculation: an interface shaped by guesswork encodes the guesser's
/// assumptions, and the first pane from another repository is the right place
/// to discover what else is owed.
pub trait Pane: Render + Focusable + 'static {
    /// What this pane calls itself, or `None` when it does not know.
    ///
    /// Never a guess. A pane that has not been told its name says so, and the
    /// workspace decides what to show instead; a pane that invented one would
    /// leave the workspace unable to tell a real name from a placeholder.
    fn title(&self) -> Option<SharedString>;

    /// The pane's allocation, set before it lays out its own contents.
    fn set_allocated(&mut self, size: Size<Pixels>);

    /// Begins an orderly shutdown, returning any blocking cleanup left to do.
    ///
    /// The closure is run off the GPUI thread. A terminal pane waits for its
    /// child to be reaped; a pane with nothing to wait for returns `None`. A
    /// closure rather than a named handle type, so that "a pane is a thing
    /// with a child process" never becomes part of the contract.
    fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>>;

    /// What closing this pane would interrupt, or `None` if closing is
    /// uneventful.
    fn close_warning(&self) -> Option<CloseWarning>;
}

/// What a person is told before a pane that is busy is closed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloseWarning {
    /// The program that would be interrupted, when it can be named.
    ///
    /// `None` still warns: "something is running" is the part that matters,
    /// and inventing a name would be worse than admitting to none.
    pub program: Option<SharedString>,
}

/// The object-safe view of a pane. Implemented on the handle, never written by
/// hand: the blanket implementation below covers every `Pane`.
///
/// Every method takes `&self`, because `Entity::update` takes `&self` and
/// yields `&mut V` inside. That is what lets the workspace hold
/// `Rc<dyn PaneHandle>` and clone it for free.
pub trait PaneHandle {
    /// The pane as an element the workspace can place.
    fn view(&self) -> AnyView;
    /// See [`Pane::title`].
    fn title(&self, cx: &App) -> Option<SharedString>;
    /// The handle the workspace focuses to hand this pane the keyboard.
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    /// See [`Pane::set_allocated`].
    fn set_allocated(&self, size: Size<Pixels>, cx: &mut App);
    /// See [`Pane::begin_shutdown`].
    fn begin_shutdown(&self, cx: &mut App) -> Option<Box<dyn FnOnce() + Send>>;
    /// See [`Pane::close_warning`].
    fn close_warning(&self, cx: &App) -> Option<CloseWarning>;
}

impl<V: Pane> PaneHandle for gpui::Entity<V> {
    fn view(&self) -> AnyView {
        self.clone().into()
    }

    fn title(&self, cx: &App) -> Option<SharedString> {
        self.read(cx).title()
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }

    fn set_allocated(&self, size: Size<Pixels>, cx: &mut App) {
        self.update(cx, |pane, _cx| pane.set_allocated(size));
    }

    fn begin_shutdown(&self, cx: &mut App) -> Option<Box<dyn FnOnce() + Send>> {
        self.update(cx, |pane, _cx| pane.begin_shutdown())
    }

    fn close_warning(&self, cx: &App) -> Option<CloseWarning> {
        self.read(cx).close_warning()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, FocusHandle, IntoElement, Render, Window, div};

    /// A pane with no terminal in it. If this type cannot satisfy the trait,
    /// the trait is shaped like `TerminalView` rather than like a pane.
    struct Placeholder {
        focus: FocusHandle,
        allocated: Option<Size<Pixels>>,
    }

    impl Render for Placeholder {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    impl gpui::Focusable for Placeholder {
        fn focus_handle(&self, _cx: &App) -> FocusHandle {
            self.focus.clone()
        }
    }

    impl Pane for Placeholder {
        fn title(&self) -> Option<SharedString> {
            None
        }

        fn set_allocated(&mut self, size: Size<Pixels>) {
            self.allocated = Some(size);
        }

        fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
            None
        }

        fn close_warning(&self) -> Option<CloseWarning> {
            None
        }
    }

    /// Compiles only if the blanket implementation reaches a second type. The
    /// arena needs an `App` to construct an entity, and `gpui`'s test support
    /// drags in Wayland and X11, so this is proven at compile time rather than
    /// run.
    fn accepts_handle<H: PaneHandle + ?Sized>() {}

    #[test]
    fn a_second_pane_type_is_a_pane_handle() {
        accepts_handle::<gpui::Entity<Placeholder>>();
        accepts_handle::<dyn PaneHandle>();
    }

    /// The manifest is the invariant. Anything beside `gpui` under
    /// `[dependencies]` is a coupling the socket must not have.
    #[test]
    fn the_crate_depends_on_gpui_alone() {
        let manifest = include_str!("../Cargo.toml");
        let dependencies = manifest
            .split("[dependencies]")
            .nth(1)
            .expect("a [dependencies] table");
        let names: Vec<&str> = dependencies
            .lines()
            .map(str::trim)
            // Up to the next table, whatever it is called.
            .take_while(|line| !line.starts_with('['))
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| line.split('=').next().unwrap().trim())
            .collect();
        assert_eq!(
            names,
            ["gpui"],
            "sprite-pane depends on gpui and nothing else"
        );
    }

    #[test]
    fn a_close_warning_names_the_program_when_it_can() {
        let named = CloseWarning {
            program: Some("vim".into()),
        };
        let unnamed = CloseWarning { program: None };
        assert_eq!(
            named.program.as_ref().map(|p| -> &str { p.as_ref() }),
            Some("vim")
        );
        assert!(unnamed.program.is_none());
    }
}
