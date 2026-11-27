"""Exact-source integrity and portable native dispatch policy regressions."""
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock
from scripts.check_gpui_patch import PATCHES, ROOT, archive_path, verify


def rust_test(source):
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "test.rs"
        path.write_text(source, encoding="utf-8")
        suffix = ".exe" if os.name == "nt" else ""
        binary = Path(tmp) / f"tests{suffix}"
        subprocess.run(["rustc", "--edition=2024", "--test", str(path), "-o", str(binary)], check=True)
        return subprocess.run([str(binary)], capture_output=True, text=True, encoding="utf-8")


class GpuiPatchTests(unittest.TestCase):
    def test_wayland_frame_pause_and_wake(self):
        source = (ROOT / "vendor/gpui/src/platform/linux/wayland/window.rs").read_text(encoding="utf-8")
        harness = (ROOT / "tests/wayland_frame_delivery.rs").read_text(encoding="utf-8")
        for marker, signature in (
                ("SET_FRAME_DELIVERY", "    fn set_frame_delivery(&mut self, active: bool) {"),
                ("FRAME", "    pub fn frame(&self) {")):
            start = source.index(signature)
            end = source.index("\n    }", start) + len("\n    }")
            harness = harness.replace("    // PRODUCTION_" + marker, source[start:end])
        result = rust_test(harness)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        result = rust_test(harness.replace(
            "if !state.frame_delivery {", "if false {"))
        self.assertNotEqual(result.returncode, 0, "unconditional rearming must fail")
        self.assertIn("a paused callback must not reach GPUI", result.stdout)

    def test_blade_atlas_reuses_removed_image_space(self):
        source = (ROOT / "vendor/gpui/src/platform/blade/blade_atlas.rs").read_text(encoding="utf-8")
        start = source.index("    fn remove(&self, key: &AtlasKey) {")
        end = source.index("\n    }\n}", start) + len("\n    }")
        harness = (ROOT / "tests/blade_atlas.rs").read_text(encoding="utf-8").replace(
            "    // PRODUCTION_REMOVE", source[start:end])
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)
            (path / "Cargo.toml").write_text(
                '[package]\nname = "blade-atlas-regression"\nversion = "0.0.0"\n'
                'edition = "2024"\n[dependencies]\netagere = "=0.2.15"\n'
                'parking_lot = "=0.12.5"\n',
                encoding="utf-8")
            (path / "src").mkdir()
            (path / "src/lib.rs").write_text(harness, encoding="utf-8")
            result = subprocess.run(
                ["cargo", "test", "--offline", "--manifest-path", str(path / "Cargo.toml")],
                env={**os.environ, "CARGO_TARGET_DIR": str(ROOT / "target/blade-atlas-regression")},
                capture_output=True, text=True, encoding="utf-8")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_exact_source_and_tamper_negative_control(self):
        archive = archive_path()
        verify(ROOT / "vendor/gpui", archive)
        with tempfile.TemporaryDirectory() as tmp:
            tree = Path(tmp) / "gpui"
            shutil.copytree(ROOT / "vendor/gpui", tree)
            (tree / "LICENSE-APACHE").write_text("tampered", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "LICENSE-APACHE"):
                verify(tree, archive)

    def test_every_recorded_patch_is_required(self):
        # Each patch applies to the tree the ones before it produce, so a patch
        # can only be dropped with every later one. Then each file it changes
        # no longer matches.
        for count, name in enumerate(PATCHES):
            changed = re.findall(r"^\+\+\+ b/(\S+)$", (ROOT / "patches" / name).read_text(encoding="utf-8"), re.M)
            with self.subTest(dropped=name), mock.patch("scripts.check_gpui_patch.PATCHES", PATCHES[:count]):
                self.assertTrue(changed)
                with self.assertRaises(ValueError) as raised:
                    verify(ROOT / "vendor/gpui", archive_path())
                differing = set(str(raised.exception).split(": ", 1)[1].split(", "))
                self.assertLessEqual(set(changed), differing)

    def test_cocoa_scope_policy_from_production_source(self):
        helper = ROOT / "vendor/gpui/src/platform/key_text_scope.rs"
        result = rust_test('mod platform { #[path = "' + helper.as_posix()
                           + '"] mod key_text_scope; }')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_wayland_commit_string_done_production_trace(self):
        source = (ROOT / "vendor/gpui/src/platform/linux/wayland/client.rs").read_text(encoding="utf-8")
        start = source.index('            zwp_text_input_v3::Event::CommitString { text } => {')
        end = source.index('            _ => {}', start)
        arms = source[start:end]
        harness = r'''
#![allow(dead_code, unused_variables)]
use std::{cell::RefCell, rc::Rc};
#[derive(Clone, Debug, PartialEq)]
struct KeyDownEvent { keystroke: Keystroke, is_held: bool }
#[derive(Clone, Debug, PartialEq)]
struct Keystroke { modifiers: Modifiers, key: String, key_char: Option<String> }
#[derive(Clone, Debug, Default, PartialEq)]
struct Modifiers;
#[derive(Clone, Debug, PartialEq)]
enum PlatformInput { KeyDown(KeyDownEvent) }
#[derive(Clone, Debug, PartialEq)]
enum ImeInput { InsertText(String), SetMarkedText(String), DeleteText }
#[derive(Clone, Debug, PartialEq)]
enum Trace { Key(PlatformInput), Ime(ImeInput) }
#[derive(Clone)]
struct Window(Rc<RefCell<Vec<Trace>>>);
struct Area { origin: Point, size: Size }
struct Point { x: Unit, y: Unit }
struct Size { width: Unit, height: Unit }
struct Unit(f32);
impl Window {
 fn handle_input(&self, input: PlatformInput) { self.0.borrow_mut().push(Trace::Key(input)); }
 fn handle_ime(&self, input: ImeInput) { self.0.borrow_mut().push(Trace::Ime(input)); }
 fn get_ime_area(&self) -> Option<Area> { None }
}
struct SerialTracker;
enum SerialKind { InputMethod }
impl SerialTracker {
 fn get(&self, _: SerialKind) -> u32 { 0 }
 fn update(&mut self, _: SerialKind, _: u32) {}
}
struct State { composing: bool, ime_pre_edit: Option<String>, keyboard_focused_window: Option<Window>, serial_tracker: SerialTracker }
struct Client(Rc<RefCell<State>>);
impl Client { fn get_client(&self) -> Rc<RefCell<State>> { self.0.clone() } }
mod zwp_text_input_v3 {
 pub enum Event { CommitString { text: Option<String> }, PreeditString { text: Option<String> }, Done { serial: u32 }, Other }
 pub struct TextInput;
 impl TextInput { pub fn set_cursor_rectangle(&self, _:i32, _:i32, _:i32, _:i32) {} pub fn commit(&self) {} }
}
fn dispatch(this: &Client, text_input: &zwp_text_input_v3::TextInput, event: zwp_text_input_v3::Event) {
 let client = this.get_client();
 let mut state = client.borrow_mut();
 match event {
''' + arms + r'''
 _ => {}
 }
}
#[test]
fn composing_one_byte_commit_before_done_is_native() {
 let trace=Rc::new(RefCell::new(Vec::new()));
 let client=Client(Rc::new(RefCell::new(State { composing:true, ime_pre_edit:None, keyboard_focused_window:Some(Window(trace.clone())), serial_tracker:SerialTracker })));
 dispatch(&client, &zwp_text_input_v3::TextInput, zwp_text_input_v3::Event::CommitString { text:Some("a".into()) });
 dispatch(&client, &zwp_text_input_v3::TextInput, zwp_text_input_v3::Event::Done { serial:1 });
 assert_eq!(*trace.borrow(), vec![Trace::Ime(ImeInput::InsertText("a".into())), Trace::Ime(ImeInput::DeleteText)]);
 assert!(!client.0.borrow().composing);
}
#[test]
fn uncomposed_ascii_retains_key_delivery_and_multibyte_is_native() {
 let trace=Rc::new(RefCell::new(Vec::new()));
 let client=Client(Rc::new(RefCell::new(State { composing:false, ime_pre_edit:None, keyboard_focused_window:Some(Window(trace.clone())), serial_tracker:SerialTracker })));
 for text in ["a", "abc", "日本", "😀"] { dispatch(&client, &zwp_text_input_v3::TextInput, zwp_text_input_v3::Event::CommitString { text:Some(text.into()) }); }
 assert!(matches!(&trace.borrow()[0], Trace::Key(_)));
 assert_eq!(&trace.borrow()[1..], &[Trace::Ime(ImeInput::InsertText("abc".into())), Trace::Ime(ImeInput::InsertText("日本".into())), Trace::Ime(ImeInput::InsertText("😀".into()))]);
}
'''
        result = rust_test(harness)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        # The pre-patch production condition must fail the very same trace.
        result = rust_test(harness.replace('commit_text.len() == 1 && !was_composing',
                                          'commit_text.len() == 1'))
        self.assertNotEqual(result.returncode, 0, 'baseline Wayland branch must be red')
        self.assertIn('composing_one_byte_commit_before_done_is_native', result.stdout)

    def test_directx_atlas_reuses_removed_image_space(self):
        source = (ROOT / "vendor/gpui/src/platform/windows/directx_atlas.rs").read_text(encoding="utf-8")
        start = source.index("    fn remove(&self, key: &AtlasKey) {")
        end = source.index("\n    }\n}", start) + len("\n    }")
        production_remove = source[start:end]
        template = r'''
#![cfg(test)]
#![allow(dead_code)]

use etagere::{BucketedAtlasAllocator, size2};
use parking_lot::Mutex;
use std::collections::HashMap;

type AtlasKey = u32;

#[derive(Clone, Copy)]
enum AtlasTextureKind {
    Monochrome,
    Polychrome,
}

#[derive(Clone, Copy)]
struct AtlasTextureId {
    kind: AtlasTextureKind,
    index: u32,
}

struct AtlasTile {
    texture_id: AtlasTextureId,
    tile_id: etagere::AllocId,
}

struct DirectXAtlasTexture {
    id: AtlasTextureId,
    allocator: BucketedAtlasAllocator,
    live_atlas_keys: u32,
}

impl DirectXAtlasTexture {
    fn decrement_ref_count(&mut self) {
        self.live_atlas_keys -= 1;
    }
    fn is_unreferenced(&self) -> bool {
        self.live_atlas_keys == 0
    }
}

#[derive(Default)]
struct TextureList {
    textures: Vec<Option<DirectXAtlasTexture>>,
    free_list: Vec<usize>,
}

struct DirectXAtlasState {
    monochrome_textures: TextureList,
    polychrome_textures: TextureList,
    tiles_by_key: HashMap<AtlasKey, AtlasTile>,
}

struct DirectXAtlas(Mutex<DirectXAtlasState>);

impl DirectXAtlas {
__PRODUCTION_REMOVE__
}

fn new_atlas(kind: AtlasTextureKind) -> DirectXAtlas {
    let mut state = DirectXAtlasState {
        monochrome_textures: TextureList::default(),
        polychrome_textures: TextureList::default(),
        tiles_by_key: HashMap::new(),
    };
    let textures = match kind {
        AtlasTextureKind::Monochrome => &mut state.monochrome_textures,
        AtlasTextureKind::Polychrome => &mut state.polychrome_textures,
    };
    textures.textures.push(Some(DirectXAtlasTexture {
        id: AtlasTextureId { kind, index: 0 },
        allocator: BucketedAtlasAllocator::new(size2(1024, 1024)),
        live_atlas_keys: 0,
    }));
    DirectXAtlas(Mutex::new(state))
}

fn insert(atlas: &DirectXAtlas, kind: AtlasTextureKind, key: AtlasKey) {
    let mut state = atlas.0.lock();
    let textures = match kind {
        AtlasTextureKind::Monochrome => &mut state.monochrome_textures,
        AtlasTextureKind::Polychrome => &mut state.polychrome_textures,
    };
    let texture = textures.textures[0].as_mut().unwrap();
    let allocation = texture
        .allocator
        .allocate(size2(512, 512))
        .expect("a removed DirectX image's space must be reusable");
    texture.live_atlas_keys += 1;
    let tile = AtlasTile {
        texture_id: texture.id,
        tile_id: allocation.id,
    };
    assert!(state.tiles_by_key.insert(key, tile).is_none());
}

#[test]
fn removed_images_reuse_space_while_another_image_stays_live() {
    for kind in [AtlasTextureKind::Monochrome, AtlasTextureKind::Polychrome] {
        let atlas = new_atlas(kind);
        insert(&atlas, kind, 0);
        for key in 1..=60 {
            insert(&atlas, kind, key);
            atlas.remove(&key);
            atlas.remove(&key);
            let state = atlas.0.lock();
            let textures = match kind {
                AtlasTextureKind::Monochrome => &state.monochrome_textures,
                AtlasTextureKind::Polychrome => &state.polychrome_textures,
            };
            assert_eq!(textures.textures.len(), 1);
            assert_eq!(textures.textures[0].as_ref().unwrap().live_atlas_keys, 1);
            assert!(textures.free_list.is_empty());
        }
        atlas.remove(&0);
        atlas.remove(&0);
        atlas.remove(&999);
        let state = atlas.0.lock();
        let textures = match kind {
            AtlasTextureKind::Monochrome => &state.monochrome_textures,
            AtlasTextureKind::Polychrome => &state.polychrome_textures,
        };
        assert!(state.tiles_by_key.is_empty());
        assert!(textures.textures[0].is_none());
        assert_eq!(textures.free_list, vec![0]);
    }
}
'''
        for expect_ok, snippet in (
            (True, production_remove),
            (False, production_remove.replace("texture.allocator.deallocate(tile.tile_id.into());", "")),
        ):
            harness = template.replace("__PRODUCTION_REMOVE__", snippet)
            with tempfile.TemporaryDirectory() as tmp:
                path = Path(tmp)
                (path / "Cargo.toml").write_text(
                    '[package]\nname = "directx-atlas-regression"\nversion = "0.0.0"\n'
                    'edition = "2024"\n[dependencies]\netagere = "=0.2.15"\n'
                    'parking_lot = "=0.12.5"\n',
                    encoding="utf-8")
                (path / "src").mkdir()
                (path / "src/lib.rs").write_text(harness, encoding="utf-8")
                result = subprocess.run(
                    ["cargo", "test", "--offline", "--manifest-path", str(path / "Cargo.toml")],
                    env={**os.environ, "CARGO_TARGET_DIR": str(ROOT / "target/blade-atlas-regression")},
                    capture_output=True, text=True, encoding="utf-8")
            if expect_ok:
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            else:
                self.assertNotEqual(result.returncode, 0, "missing deallocate must fail")
                self.assertIn("a removed DirectX image's space must be reusable", result.stdout)

    def test_windows_native_text_char_origin_from_production_source(self):
        source = (ROOT / "vendor/gpui/src/platform/windows/events.rs").read_text(encoding="utf-8")

        def extract(sig, end_marker):
            start = source.index(sig)
            end = source.index(end_marker, start)
            return source[start:end]

        keydown = extract(
            "    fn handle_keydown_msg(&self, handle: HWND, wparam: WPARAM, lparam: LPARAM) -> Option<isize> {",
            "\n\n    fn handle_keyup_msg",
        )
        char_msg = extract(
            "    fn handle_char_msg(&self, wparam: WPARAM, lparam: LPARAM) -> Option<isize> {",
            "\n\n    fn handle_dead_char_msg",
        )
        dead_char = extract(
            "    fn handle_dead_char_msg(&self, wparam: WPARAM) -> Option<isize> {",
            "\n\n    fn handle_mouse_down_msg",
        )
        ime_comp = extract(
            "    fn handle_ime_composition(&self, handle: HWND, lparam: LPARAM) -> Option<isize> {",
            "\n\n    fn handle_ime_composition_inner",
        )
        take_pending = extract(
            "    fn take_pending_key_char(&self, lparam: isize) -> bool {",
            "\n\n    #[inline]\n    fn parse_char_message",
        )
        parse_char = extract(
            "    fn parse_char_message(&self, wparam: WPARAM) -> Option<String> {",
            "\n\n    fn start_tracking_mouse",
        )
        key_event = extract(
            "fn handle_key_event<F>(",
            "\n\nfn parse_immutable",
        )

        template = r'''
#![allow(dead_code, unused_variables, unused_unsafe, non_camel_case_types, non_snake_case)]
use std::{cell::RefCell, collections::VecDeque, ops::Range, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HWND(usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HIMC(usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WPARAM(usize);
impl WPARAM {
    fn loword(&self) -> u16 { (self.0 & 0xffff) as u16 }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LPARAM(isize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VIRTUAL_KEY(u16);

const VK_SHIFT: VIRTUAL_KEY = VIRTUAL_KEY(0x10);
const VK_CONTROL: VIRTUAL_KEY = VIRTUAL_KEY(0x11);
const VK_MENU: VIRTUAL_KEY = VIRTUAL_KEY(0x12);
const VK_CAPITAL: VIRTUAL_KEY = VIRTUAL_KEY(0x14);
const VK_SPACE: VIRTUAL_KEY = VIRTUAL_KEY(0x20);
const VK_LEFT: VIRTUAL_KEY = VIRTUAL_KEY(0x25);
const VK_LWIN: VIRTUAL_KEY = VIRTUAL_KEY(0x5B);
const VK_RWIN: VIRTUAL_KEY = VIRTUAL_KEY(0x5C);
const VK_LMENU: VIRTUAL_KEY = VIRTUAL_KEY(0xA4);
const VK_RMENU: VIRTUAL_KEY = VIRTUAL_KEY(0xA5);
const VK_PACKET: VIRTUAL_KEY = VIRTUAL_KEY(0xE7);
const VK_PROCESSKEY: VIRTUAL_KEY = VIRTUAL_KEY(0xE5);

const VK_A: VIRTUAL_KEY = VIRTUAL_KEY(0x41);
const VK_B: VIRTUAL_KEY = VIRTUAL_KEY(0x42);
const VK_E: VIRTUAL_KEY = VIRTUAL_KEY(0x45);
const VK_X: VIRTUAL_KEY = VIRTUAL_KEY(0x58);
const VK_DEAD_CARET: VIRTUAL_KEY = VIRTUAL_KEY(0xDD);

mod log {
    macro_rules! warn_impl { ($($arg:tt)*) => {}; }
    pub(crate) use warn_impl as warn;
}
trait ResultExt { fn log_err(self) where Self: Sized {} }
impl<T, E> ResultExt for Result<T, E> {}
impl<T> ResultExt for Option<T> {}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Modifiers;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Capslock { on: bool }
#[derive(Clone, Debug, PartialEq)]
struct Keystroke { modifiers: Modifiers, key: String, key_char: Option<String> }
#[derive(Clone, Debug, PartialEq)]
struct KeyDownEvent { keystroke: Keystroke, is_held: bool }
#[derive(Clone, Debug, PartialEq)]
struct ModifiersChangedEvent { modifiers: Modifiers, capslock: Capslock }
#[derive(Clone, Debug, PartialEq)]
enum PlatformInput {
    KeyDown(KeyDownEvent),
    ModifiersChanged(ModifiersChangedEvent),
}
struct DispatchEventResult { propagate: bool }

#[derive(Clone, Debug, PartialEq)]
enum Recorded {
    FromKey(String),
    Native(String),
    Mark(String),
}

struct PlatformInputHandler {
    marked: Option<Range<usize>>,
    events: Rc<RefCell<Vec<Recorded>>>,
}
impl PlatformInputHandler {
    fn marked_text_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }
    fn replace_text_in_range(&mut self, _range: Option<Range<usize>>, text: &str) {
        self.marked = None;
        self.events.borrow_mut().push(Recorded::Native(text.to_string()));
    }
    fn replace_text_in_range_from_key(&mut self, _range: Option<Range<usize>>, text: &str) {
        self.events.borrow_mut().push(Recorded::FromKey(text.to_string()));
    }
    fn replace_and_mark_text_in_range(&mut self, _range: Option<Range<usize>>, text: &str, _new_sel: Option<Range<usize>>) {
        self.marked = if text.is_empty() { None } else { Some(0..text.len()) };
        self.events.borrow_mut().push(Recorded::Mark(text.to_string()));
    }
}

#[derive(Default)]
struct Callbacks {
    input: Option<Box<dyn FnMut(PlatformInput) -> DispatchEventResult>>,
}

struct WindowsWindowState {
    callbacks: Callbacks,
    input_handler: Option<PlatformInputHandler>,
    pending_surrogate: Option<u16>,
    pending_key_char_lparams: VecDeque<isize>,
    last_reported_modifiers: Option<Modifiers>,
    last_reported_capslock: Option<Capslock>,
}

struct WindowsWindowInner {
    state: RefCell<WindowsWindowState>,
}

fn current_modifiers() -> Modifiers { Modifiers }
fn current_capslock() -> Capslock { Capslock { on: false } }
unsafe fn ImmGetVirtualKey(_: HWND) -> u32 { VK_A.0 as u32 }
unsafe fn ImmGetContext(_: HWND) -> HIMC { HIMC(1) }
unsafe fn ImmReleaseContext(_: HWND, _: HIMC) -> Result<(), ()> { Ok(()) }
fn translate_message(_: HWND, _: WPARAM, _: LPARAM) {}

fn parse_normal_key(vkey: VIRTUAL_KEY, _lparam: LPARAM, modifiers: Modifiers) -> Option<Keystroke> {
    let (key, key_char) = match vkey {
        VK_SPACE => ("space".to_string(), None),
        VK_LEFT => ("left".to_string(), None),
        VK_A => ("a".to_string(), Some("a".to_string())),
        VK_B => ("b".to_string(), Some("b".to_string())),
        VK_E => ("e".to_string(), Some("e".to_string())),
        VK_X => ("x".to_string(), Some("x".to_string())),
        VK_DEAD_CARET => ("^".to_string(), Some("^".to_string())),
        _ => return None,
    };
    Some(Keystroke { modifiers, key, key_char })
}

__HANDLE_KEY_EVENT__

impl WindowsWindowInner {
    fn new(handled_keys: &'static [&'static str]) -> (Self, Rc<RefCell<Vec<Recorded>>>) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let inner = Self {
            state: RefCell::new(WindowsWindowState {
                callbacks: Callbacks {
                    input: Some(Box::new(move |input| {
                        let propagate = match input {
                            PlatformInput::KeyDown(ev) => !handled_keys.contains(&ev.keystroke.key.as_str()),
                            _ => true,
                        };
                        DispatchEventResult { propagate }
                    })),
                },
                input_handler: Some(PlatformInputHandler {
                    marked: None,
                    events: events.clone(),
                }),
                pending_surrogate: None,
                pending_key_char_lparams: VecDeque::new(),
                last_reported_modifiers: None,
                last_reported_capslock: None,
            }),
        };
        (inner, events)
    }

    fn with_input_handler<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut PlatformInputHandler) -> R,
    {
        let mut input_handler = self.state.borrow_mut().input_handler.take()?;
        let result = f(&mut input_handler);
        self.state.borrow_mut().input_handler = Some(input_handler);
        Some(result)
    }

    fn handle_ime_composition_inner(&self, _ctx: HIMC, _lparam: LPARAM) -> Option<isize> {
        self.with_input_handler(|input_handler| {
            input_handler.replace_text_in_range(None, "漢");
        });
        Some(0)
    }

__HANDLE_KEYDOWN__
__HANDLE_CHAR__
__HANDLE_DEAD_CHAR__
__HANDLE_IME_COMP__
__TAKE_PENDING__
__PARSE_CHAR__
}

#[test]
fn ordinary_and_space_and_rapid_keys_route_from_key_while_ime_deadkey_and_packet_stay_native() {
    let (win, events) = WindowsWindowInner::new(&["x"]);
    let hwnd = HWND(1);

    // 1. Ordinary printable key ('a') -> FromKey
    win.handle_keydown_msg(hwnd, WPARAM(VK_A.0 as usize), LPARAM(0x001E_0001));
    win.handle_char_msg(WPARAM('a' as usize), LPARAM(0x001E_0001));

    // 2. VK_SPACE (key_char == None, key == "space") -> FromKey
    win.handle_keydown_msg(hwnd, WPARAM(VK_SPACE.0 as usize), LPARAM(0x0039_0001));
    win.handle_char_msg(WPARAM(' ' as usize), LPARAM(0x0039_0001));

    // 3. Handled key ('x') does not track lparam -> synthetic WM_CHAR stays Native
    win.handle_keydown_msg(hwnd, WPARAM(VK_X.0 as usize), LPARAM(0x002D_0001));
    win.handle_char_msg(WPARAM('x' as usize), LPARAM(0x002D_0001));

    // 4. Dead key sequence: WM_KEYDOWN('^') -> WM_DEADCHAR('^') -> WM_KEYDOWN('e') while composing -> WM_CHAR('ê') -> Native
    win.handle_keydown_msg(hwnd, WPARAM(VK_DEAD_CARET.0 as usize), LPARAM(0x0007_0001));
    win.handle_dead_char_msg(WPARAM('^' as usize));
    win.handle_keydown_msg(hwnd, WPARAM(VK_E.0 as usize), LPARAM(0x0012_0001));
    win.handle_char_msg(WPARAM('ê' as usize), LPARAM(0x0012_0001));

    // 5. Non-char key (VK_LEFT) followed by VK_PACKET surrogate pair ('😀') -> Native
    win.handle_keydown_msg(hwnd, WPARAM(VK_LEFT.0 as usize), LPARAM(0x014B_0001));
    win.handle_keydown_msg(hwnd, WPARAM(VK_PACKET.0 as usize), LPARAM(0x0000_0001));
    assert_eq!(win.handle_char_msg(WPARAM(0xD83D), LPARAM(0x0000_0001)), None);
    assert_eq!(win.handle_char_msg(WPARAM(0xDE00), LPARAM(0x0000_0001)), Some(0));

    // 6. Tracked key surrogate pair: high surrogate does NOT pop lparam; low surrogate completes FromKey
    win.handle_keydown_msg(hwnd, WPARAM(VK_A.0 as usize), LPARAM(0x0099_0001));
    assert_eq!(win.handle_char_msg(WPARAM(0xD83D), LPARAM(0x0099_0001)), None);
    assert_eq!(win.state.borrow().pending_key_char_lparams.len(), 1);
    assert_eq!(win.handle_char_msg(WPARAM(0xDE00), LPARAM(0x0099_0001)), Some(0));
    assert!(win.state.borrow().pending_key_char_lparams.is_empty());

    // 7. Rapid typing: two WM_KEYDOWNs before two WM_CHARs
    win.handle_keydown_msg(hwnd, WPARAM(VK_A.0 as usize), LPARAM(0x001E_0001));
    win.handle_keydown_msg(hwnd, WPARAM(VK_B.0 as usize), LPARAM(0x0030_0001));
    win.handle_char_msg(WPARAM('a' as usize), LPARAM(0x001E_0001));
    win.handle_char_msg(WPARAM('b' as usize), LPARAM(0x0030_0001));

    // 8. WM_IME_COMPOSITION clears pending entries so subsequent WM_CHAR with same lparam is Native
    win.handle_keydown_msg(hwnd, WPARAM(VK_A.0 as usize), LPARAM(0x001E_0001));
    win.handle_ime_composition(hwnd, LPARAM(0));
    win.handle_char_msg(WPARAM('a' as usize), LPARAM(0x001E_0001));

    // 9. Stale matching lparam before VK_PACKET is cleared by VK_PACKET so emoji stays Native
    win.handle_keydown_msg(hwnd, WPARAM(VK_A.0 as usize), LPARAM(0x0000_0001));
    win.handle_keydown_msg(hwnd, WPARAM(VK_PACKET.0 as usize), LPARAM(0x0000_0001));
    assert_eq!(win.handle_char_msg(WPARAM(0xD83D), LPARAM(0x0000_0001)), None);
    assert_eq!(win.handle_char_msg(WPARAM(0xDE00), LPARAM(0x0000_0001)), Some(0));

    assert_eq!(
        *events.borrow(),
        vec![
            Recorded::FromKey("a".into()),
            Recorded::FromKey(" ".into()),
            Recorded::Native("x".into()),
            Recorded::Mark("^".into()),
            Recorded::Native("ê".into()),
            Recorded::Native("😀".into()),
            Recorded::FromKey("😀".into()),
            Recorded::FromKey("a".into()),
            Recorded::FromKey("b".into()),
            Recorded::Native("漢".into()),
            Recorded::Native("a".into()),
            Recorded::Native("😀".into()),
        ]
    );
}
'''
        harness = (
            template.replace("__HANDLE_KEY_EVENT__", key_event)
            .replace("__HANDLE_KEYDOWN__", keydown)
            .replace("__HANDLE_CHAR__", char_msg)
            .replace("__HANDLE_DEAD_CHAR__", dead_char)
            .replace("__HANDLE_IME_COMP__", ime_comp)
            .replace("__TAKE_PENDING__", take_pending)
            .replace("__PARSE_CHAR__", parse_char)
        )
        result = rust_test(harness)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

        # Negative controls: removing VK_SPACE check or VK_PACKET clear must fail.
        for bad_from, bad_to in (
            ('|| event.keystroke.key == "space"', "|| false"),
            ("state.pending_key_char_lparams.clear();", ""),
        ):
            bad = rust_test(harness.replace(bad_from, bad_to, 1))
            self.assertNotEqual(bad.returncode, 0, f"negative control ({bad_from}) must fail")

    def test_windows_vsync_hub_pause_and_wake_from_production_source(self):
        platform_src = (ROOT / "vendor/gpui/src/platform/windows/platform.rs").read_text(encoding="utf-8")
        window_src = (ROOT / "vendor/gpui/src/platform/windows/window.rs").read_text(encoding="utf-8")

        hub_start = platform_src.index("pub(crate) struct VSyncHub {")
        hub_end = platform_src.index("\npub(crate) struct WindowsPlatformState", hub_start)
        vsync_hub_src = platform_src[hub_start:hub_end]

        win_start = window_src.index("    fn set_frame_delivery(&self, active: bool) {")
        win_end = window_src.index("\n\n    fn sprite_atlas", win_start)
        window_methods_src = window_src[win_start:win_end]

        template = r'''
#![allow(dead_code)]
use std::{
    rc::Rc,
    sync::{
        Arc, Condvar, Mutex, MutexGuard, Weak,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HWND(usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SafeHwnd(usize);
impl SafeHwnd {
    fn as_raw(&self) -> usize { self.0 }
}
impl From<HWND> for SafeHwnd {
    fn from(value: HWND) -> Self { SafeHwnd(value.0) }
}

__VSYNC_HUB__

trait PlatformWindow {
    fn set_frame_delivery(&self, active: bool);
    fn frame_waker(&self) -> Option<Rc<dyn Fn()>>;
}

struct WindowsWindowInner {
    hwnd: HWND,
    vsync_hub: Weak<VSyncHub>,
}

struct WindowsWindow(Rc<WindowsWindowInner>);

impl PlatformWindow for WindowsWindow {
__WINDOW_METHODS__
}

fn wait_until(cond: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for condition");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn idle_pause_partial_pause_waker_and_shutdown() {
    let hub = Arc::new(VSyncHub::new());
    let win_a = WindowsWindow(Rc::new(WindowsWindowInner {
        hwnd: HWND(1),
        vsync_hub: Arc::downgrade(&hub),
    }));
    let win_b = WindowsWindow(Rc::new(WindowsWindowInner {
        hwnd: HWND(2),
        vsync_hub: Arc::downgrade(&hub),
    }));

    hub.register_window(HWND(1).into());

    let ticks = Arc::new(AtomicUsize::new(0));
    let exited = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let parked = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let hub_thread = Arc::downgrade(&hub);
    let ticks_thread = ticks.clone();
    let exited_thread = exited.clone();
    let parked_thread = parked.clone();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let gate_thread = gate.clone();

    let handle = std::thread::spawn(move || {
        let mut active = Vec::new();
        loop {
            let Some(hub) = hub_thread.upgrade() else {
                break;
            };
            parked_thread.store(true, Ordering::SeqCst);
            let keep_going = hub.wait_until_active();
            parked_thread.store(false, Ordering::SeqCst);
            if !keep_going {
                break;
            }
            hub.collect_active_windows(&mut active);
            ticks_thread.fetch_add(1, Ordering::SeqCst);
            let (lock, cvar) = &*gate_thread;
            let mut allow = lock.lock().unwrap();
            while !*allow {
                allow = cvar.wait(allow).unwrap();
            }
            *allow = false;
        }
        exited_thread.store(true, Ordering::SeqCst);
    });

    let release_gate = || {
        let (lock, cvar) = &*gate;
        *lock.lock().unwrap() = true;
        cvar.notify_one();
    };

    let step_vsync = || {
        let before = ticks.load(Ordering::SeqCst);
        release_gate();
        wait_until(|| ticks.load(Ordering::SeqCst) > before);
    };

    // Wait for initial tick with win_a active.
    wait_until(|| ticks.load(Ordering::SeqCst) >= 1);
    let mut active = Vec::new();
    hub.collect_active_windows(&mut active);
    assert_eq!(active, vec![SafeHwnd(1)]);

    // Pause win_a and release gate -> vsync thread loops and parks.
    win_a.set_frame_delivery(false);
    release_gate();
    wait_until(|| parked.load(Ordering::SeqCst));
    let paused_ticks = ticks.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(25));
    assert_eq!(ticks.load(Ordering::SeqCst), paused_ticks, "paused vsync must stay parked");

    // Wake win_a via frame_waker -> vsync thread resumes.
    let waker_a = win_a.frame_waker().expect("frame_waker must be Some");
    waker_a();
    wait_until(|| ticks.load(Ordering::SeqCst) > paused_ticks);

    // Multi-window partial pause: register B, pause A -> B stays active and vsync keeps running.
    hub.register_window(HWND(2).into());
    win_a.set_frame_delivery(false);
    hub.collect_active_windows(&mut active);
    assert_eq!(active, vec![SafeHwnd(2)]);
    step_vsync();

    // Pause B as well and release gate -> vsync thread parks cleanly.
    win_b.set_frame_delivery(false);
    release_gate();
    wait_until(|| parked.load(Ordering::SeqCst));

    // Unregister B, verify shutdown wakes parked thread cleanly.
    hub.unregister_window(HWND(2).into());
    hub.shutdown();
    wait_until(|| exited.load(Ordering::SeqCst));
    handle.join().unwrap();
}
'''
        harness = (
            template.replace("__VSYNC_HUB__", vsync_hub_src)
            .replace("__WINDOW_METHODS__", window_methods_src)
        )
        result = rust_test(harness)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

        # Negative controls: missing notify_one on wake or missing shutdown flag must fail.
        for bad_from, bad_to in (
            ("self.condvar.notify_one();", ""),
            ("self.lock().shutdown = true;", ""),
        ):
            # Replace last occurrence of notify_one (inside set_frame_delivery) or shutdown
            idx = harness.rfind(bad_from)
            self.assertNotEqual(idx, -1)
            bad_harness = harness[:idx] + bad_to + harness[idx + len(bad_from):]
            bad = rust_test(bad_harness)
            self.assertNotEqual(bad.returncode, 0, f"negative control ({bad_from}) must fail")


if __name__ == '__main__':
    unittest.main()
