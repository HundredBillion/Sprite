"""Exact-source integrity and portable native dispatch policy regressions."""
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from scripts.check_gpui_patch import ROOT, archive_path, verify


def rust_test(source):
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "test.rs"
        path.write_text(source)
        binary = Path(tmp) / "tests"
        subprocess.run(["rustc", "--edition=2024", "--test", str(path), "-o", str(binary)], check=True)
        return subprocess.run([str(binary)], capture_output=True, text=True)


class GpuiPatchTests(unittest.TestCase):
    def test_exact_source_and_tamper_negative_control(self):
        archive = archive_path()
        verify(ROOT / "vendor/gpui", archive)
        with tempfile.TemporaryDirectory() as tmp:
            tree = Path(tmp) / "gpui"
            shutil.copytree(ROOT / "vendor/gpui", tree)
            (tree / "LICENSE-APACHE").write_text("tampered")
            with self.assertRaisesRegex(ValueError, "LICENSE-APACHE"):
                verify(tree, archive)

    def test_cocoa_scope_policy_from_production_source(self):
        helper = ROOT / "vendor/gpui/src/platform/key_text_scope.rs"
        result = rust_test('mod platform { #[path = ' + repr(str(helper)).replace("'", '"')
                           + '] mod key_text_scope; }')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_wayland_commit_string_done_production_trace(self):
        source = (ROOT / "vendor/gpui/src/platform/linux/wayland/client.rs").read_text()
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


if __name__ == '__main__':
    unittest.main()
