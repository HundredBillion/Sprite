use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone, Default)]
struct Surface {
    frames: Rc<Cell<usize>>,
    commits: Rc<Cell<usize>>,
}
impl Surface {
    fn frame(&self, _: &(), _: ()) {
        self.frames.set(self.frames.get() + 1);
    }
    fn commit(&self) {
        self.commits.set(self.commits.get() + 1);
    }
    fn id(&self) {}
}
struct Globals {
    qh: (),
}
struct WaylandWindowState {
    frame_delivery: bool,
    frame_pending: bool,
    resize_throttle: bool,
    surface: Surface,
    globals: Globals,
}
impl WaylandWindowState {
    // PRODUCTION_SET_FRAME_DELIVERY
}
struct Callbacks {
    request_frame: Option<Box<dyn FnMut(())>>,
}
struct WaylandWindowStatePtr {
    state: Rc<RefCell<WaylandWindowState>>,
    callbacks: RefCell<Callbacks>,
}
impl WaylandWindowStatePtr {
    // PRODUCTION_FRAME
}

#[test]
fn pause_drains_the_pending_callback_and_wake_requests_one_frame() {
    let surface = Surface::default();
    let delivered = Rc::new(Cell::new(0));
    let counted = delivered.clone();
    let ptr = WaylandWindowStatePtr {
        state: Rc::new(RefCell::new(WaylandWindowState {
            frame_delivery: true,
            frame_pending: true,
            resize_throttle: true,
            surface: surface.clone(),
            globals: Globals { qh: () },
        })),
        callbacks: RefCell::new(Callbacks {
            request_frame: Some(Box::new(move |_| counted.set(counted.get() + 1))),
        }),
    };
    ptr.state.borrow_mut().set_frame_delivery(false);
    ptr.frame();
    assert_eq!(delivered.get(), 0, "a paused callback must not reach GPUI");
    assert_eq!(
        surface.frames.get(),
        0,
        "a paused callback must not rearm itself"
    );
    assert!(!ptr.state.borrow().frame_pending);
    assert!(
        !ptr.state.borrow().resize_throttle,
        "a drained callback releases resize throttling"
    );
    ptr.state.borrow_mut().set_frame_delivery(true);
    ptr.state.borrow_mut().set_frame_delivery(true);
    assert_eq!(
        surface.frames.get(),
        1,
        "repeated wake requests must coalesce"
    );
    assert_eq!(
        surface.commits.get(),
        1,
        "the compositor must receive the wake"
    );
    ptr.frame();
    assert_eq!(delivered.get(), 1);
    assert_eq!(surface.frames.get(), 2);
    ptr.state.borrow_mut().set_frame_delivery(false);
    ptr.state.borrow_mut().set_frame_delivery(true);
    assert_eq!(
        surface.frames.get(),
        2,
        "an outstanding callback can deliver the wake"
    );
    ptr.frame();
    assert_eq!(delivered.get(), 2);
}
