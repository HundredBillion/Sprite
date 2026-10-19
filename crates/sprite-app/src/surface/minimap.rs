use super::Refusal;
use serde::Deserialize;
use serde_json::Value;

const MAX_SAFE: u64 = 9_007_199_254_740_991;
const MAX_ROWS: usize = 1_000_000;
const SOURCE_BYTES: usize = 64 * 1024 * 1024;
const CHUNK_BYTES: usize = 256 * 1024;
const STATE_BYTES: usize = 4 * 1024 * 1024;

fn bad() -> Refusal {
    Refusal::Malformed("invalid minimap operation".into())
}
fn require(ok: bool) -> Result<(), Refusal> {
    if ok { Ok(()) } else { Err(bad()) }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Minimap {
    revision: u64,
    document: Option<Document>,
    state: Option<State>,
    pending: Option<Pending>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    revision: u64,
    target: String,
    generation: u64,
    total: usize,
    palette: Vec<String>,
    background: String,
    rows: Vec<Line>,
}
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Line {
    text: String,
    spans: Vec<[usize; 3]>,
}
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Change {
    start: usize,
    end: usize,
    kind: ChangeKind,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    revision: u64,
    viewport: [usize; 2],
    selection: Option<[usize; 2]>,
    occurrences: Vec<usize>,
    changes: Vec<Change>,
    word: Option<String>,
}
#[derive(Clone, Debug, Deserialize, PartialEq)]
struct Begin {
    target: String,
    generation: u64,
    base: Option<u64>,
    start: usize,
    delete: usize,
    insert: usize,
    total: usize,
    palette: Vec<String>,
    background: String,
}
#[derive(Clone, Debug, PartialEq)]
struct Pending {
    begin: Begin,
    rows: Vec<Line>,
    bytes: usize,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MinimapOp {
    revision: u64,
    kind: Kind,
}
#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Begin(Begin),
    Lines {
        target: String,
        generation: u64,
        offset: usize,
        lines: Vec<Line>,
    },
    Commit {
        target: String,
        generation: u64,
    },
    State {
        target: String,
        generation: u64,
        state: State,
    },
    Hide,
}

impl Document {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn target(&self) -> &str {
        &self.target
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn total(&self) -> usize {
        self.total
    }
    pub fn palette(&self) -> &[String] {
        &self.palette
    }
    pub fn background(&self) -> &str {
        &self.background
    }
    pub fn rows(&self) -> &[Line] {
        &self.rows
    }
}
impl Line {
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn spans(&self) -> &[[usize; 3]] {
        &self.spans
    }
    fn bytes(&self) -> usize {
        48 + self.text.len() + self.spans.len() * 24
    }
}
impl Change {
    pub fn start(&self) -> usize {
        self.start
    }
    pub fn end(&self) -> usize {
        self.end
    }
    pub fn kind(&self) -> ChangeKind {
        self.kind
    }
}
impl State {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn viewport(&self) -> [usize; 2] {
        self.viewport
    }
    pub fn selection(&self) -> Option<[usize; 2]> {
        self.selection
    }
    pub fn occurrences(&self) -> &[usize] {
        &self.occurrences
    }
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }
    pub fn word(&self) -> Option<&str> {
        self.word.as_deref()
    }
}

fn number(v: &Value) -> Result<u64, Refusal> {
    v.as_u64().filter(|n| *n <= MAX_SAFE).ok_or_else(bad)
}
fn positive(v: &Value) -> Result<u64, Refusal> {
    let n = number(v)?;
    require(n > 0)?;
    Ok(n)
}
fn integer_tree(v: &Value) -> Result<(), Refusal> {
    match v {
        Value::Number(_) => {
            number(v)?;
        }
        Value::Array(a) => {
            for v in a {
                integer_tree(v)?;
            }
        }
        Value::Object(o) => {
            for v in o.values() {
                integer_tree(v)?;
            }
        }
        _ => (),
    }
    Ok(())
}
fn target(v: &Value) -> Result<String, Refusal> {
    let s = v.as_str().ok_or_else(bad)?;
    require(!s.is_empty() && s.len() <= 128)?;
    Ok(s.into())
}
fn decode<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, Refusal> {
    T::deserialize(v).map_err(|_| bad())
}
fn encoded_within(value: &Value, limit: usize) -> bool {
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.0 {
                return Err(std::io::Error::other("minimap message budget exceeded"));
            }
            self.0 -= bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget(limit), value).is_ok()
}
fn color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}
fn range(r: [usize; 2], total: usize) -> bool {
    r[0] > 0 && r[0] <= r[1] && r[1] <= total
}

impl Minimap {
    pub fn is_staging(&self) -> bool {
        self.pending.is_some()
    }
    #[cfg(test)]
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref().filter(|d| {
            self.pending
                .as_ref()
                .is_none_or(|p| p.begin.target == d.target)
        })
    }
    pub fn state(&self) -> Option<&State> {
        self.state.as_ref()
    }
    pub fn visible_width(&self, surface_width: f32) -> f32 {
        if self.document().is_some() && surface_width >= 480.0 {
            160.0
        } else {
            0.0
        }
    }
    pub fn parse(value: &Value) -> Result<MinimapOp, Refusal> {
        require(value["type"] == "minimap")?;
        integer_tree(value)?;
        let revision = positive(&value["revision"])?;
        let kind = value["kind"].as_str().ok_or_else(bad)?;
        let allowed: &[&str] = match kind {
            "begin" => &[
                "target",
                "generation",
                "base",
                "start",
                "delete",
                "insert",
                "total",
                "palette",
                "background",
            ],
            "lines" => &["target", "generation", "offset", "lines"],
            "commit" => &["target", "generation"],
            "state" => &[
                "target",
                "generation",
                "viewport",
                "selection",
                "occurrences",
                "changes",
                "word",
            ],
            "hide" => &[],
            _ => return Err(bad()),
        };
        let object = value.as_object().ok_or_else(bad)?;
        require(object.keys().all(|k| {
            matches!(k.as_str(), "type" | "revision" | "kind" | "version")
                || allowed.contains(&k.as_str())
        }))?;
        require(allowed.iter().all(|k| object.contains_key(*k)))?;
        let payload = match kind {
            "begin" => {
                target(&value["target"])?;
                require(value["palette"].as_array().is_some_and(|a| {
                    !a.is_empty()
                        && a.len() <= 256
                        && a.iter().all(|v| v.as_str().is_some_and(color))
                }))?;
                require(value["background"].as_str().is_some_and(color))?;
                let b: Begin = decode(value)?;
                target(&value["target"])?;
                positive(&value["generation"])?;
                if let Some(base) = b.base {
                    require(base > 0)?;
                }
                require(
                    b.start > 0
                        && b.start <= MAX_ROWS + 1
                        && b.delete <= MAX_ROWS
                        && b.insert <= MAX_ROWS
                        && b.total > 0
                        && b.total <= MAX_ROWS,
                )?;
                require(
                    !b.palette.is_empty()
                        && b.palette.len() <= 256
                        && b.palette.iter().all(|s| color(s))
                        && color(&b.background),
                )?;
                require(b.insert * 48 <= SOURCE_BYTES)?;
                Kind::Begin(b)
            }
            "lines" => {
                require(encoded_within(value, CHUNK_BYTES))?;
                let lines = value["lines"].as_array().ok_or_else(bad)?;
                require(!lines.is_empty() && lines.len() <= 256)?;
                // Check bounds before deserialization allocates owned text and span arrays.
                for line in lines {
                    require(
                        line["text"]
                            .as_str()
                            .is_some_and(|s| s.len() <= 2048 && !s.contains(['\n', '\r', '\t'])),
                    )?;
                    require(line["spans"].as_array().is_some_and(|a| a.len() <= 256))?;
                }
                Kind::Lines {
                    target: target(&value["target"])?,
                    generation: positive(&value["generation"])?,
                    offset: usize::try_from(number(&value["offset"])?).map_err(|_| bad())?,
                    lines: decode(&value["lines"])?,
                }
            }
            "commit" => Kind::Commit {
                target: target(&value["target"])?,
                generation: positive(&value["generation"])?,
            },
            "state" => {
                require(encoded_within(value, STATE_BYTES))?;
                for k in ["occurrences", "changes"] {
                    require(value[k].as_array().is_some_and(|a| a.len() <= 100_000))?;
                }
                require(
                    value["word"].is_null()
                        || value["word"].as_str().is_some_and(|s| s.len() <= 1024),
                )?;
                let word: Option<String> = decode(&value["word"])?;
                require(word.as_ref().is_none_or(|s| s.len() <= 1024))?;
                Kind::State {
                    target: target(&value["target"])?,
                    generation: positive(&value["generation"])?,
                    state: State {
                        revision,
                        viewport: decode(&value["viewport"])?,
                        selection: decode(&value["selection"])?,
                        occurrences: decode(&value["occurrences"])?,
                        changes: decode(&value["changes"])?,
                        word,
                    },
                }
            }
            "hide" => Kind::Hide,
            _ => unreachable!(),
        };
        Ok(MinimapOp {
            revision,
            kind: payload,
        })
    }
    pub fn apply(&mut self, op: MinimapOp) -> Result<(), Refusal> {
        #[cfg(feature = "minimap-benchmark")]
        if let Kind::State { state, .. } = &op.kind {
            crate::minimap_benchmark::record(
                serde_json::json!({"event":"state_received", "viewport":state.viewport}),
            );
        }
        require(op.revision > self.revision)?;
        match op.kind {
            Kind::Begin(b) => {
                require(
                    self.document
                        .as_ref()
                        .is_none_or(|d| b.generation > d.generation)
                        && self
                            .pending
                            .as_ref()
                            .is_none_or(|p| b.generation > p.begin.generation),
                )?;
                if let Some(base) = b.base {
                    let d = self.document.as_ref().ok_or_else(bad)?;
                    require(
                        d.target == b.target
                            && d.generation == base
                            && d.palette == b.palette
                            && d.background == b.background,
                    )?;
                    require(
                        b.start <= d.total + 1
                            && b.delete <= d.total - (b.start - 1)
                            && b.total == d.total - b.delete + b.insert,
                    )?;
                } else {
                    require(b.start == 1 && b.delete == 0 && b.insert == b.total)?;
                }
                self.pending = Some(Pending {
                    begin: b,
                    rows: Vec::new(),
                    bytes: 0,
                });
                self.state = None;
            }
            Kind::Lines {
                target,
                generation,
                offset,
                lines,
            } => {
                let p = self.pending.as_ref().ok_or_else(bad)?;
                require(
                    p.begin.target == target
                        && p.begin.generation == generation
                        && offset == p.rows.len()
                        && lines.len() <= p.begin.insert - p.rows.len(),
                )?;
                for line in &lines {
                    let mut end = 0;
                    let mut previous = None;
                    for &[a, b, c] in &line.spans {
                        require(
                            a >= end
                                && a < b
                                && b <= line.text.len()
                                && line.text.is_char_boundary(a)
                                && line.text.is_char_boundary(b)
                                && c < p.begin.palette.len()
                                && previous != Some((a, c)),
                        )?;
                        end = b;
                        previous = Some((b, c));
                    }
                }
                let bytes = lines.iter().map(Line::bytes).sum::<usize>();
                require(bytes <= SOURCE_BYTES - p.bytes)?;
                let p = self.pending.as_mut().unwrap();
                p.bytes += bytes;
                p.rows.reserve_exact(lines.len());
                p.rows.extend(lines);
            }
            Kind::Commit { target, generation } => {
                let p = self.pending.as_ref().ok_or_else(bad)?;
                require(
                    p.begin.target == target
                        && p.begin.generation == generation
                        && p.rows.len() == p.begin.insert,
                )?;
                let b = &p.begin;
                if b.base.is_some() {
                    let d = self.document.as_ref().ok_or_else(bad)?;
                    let retained = d.rows[..b.start - 1]
                        .iter()
                        .chain(d.rows[b.start - 1 + b.delete..].iter())
                        .map(Line::bytes)
                        .sum::<usize>();
                    require(retained <= SOURCE_BYTES - p.bytes)?;
                }
                let p = self.pending.take().unwrap();
                let b = p.begin;
                let rows = if b.base.is_some() {
                    let mut rows = self.document.take().unwrap().rows;
                    rows.reserve_exact(b.total.saturating_sub(rows.len()));
                    rows.splice(b.start - 1..b.start - 1 + b.delete, p.rows);
                    rows.shrink_to_fit();
                    rows
                } else {
                    p.rows
                };
                self.document = Some(Document {
                    revision: op.revision,
                    target: b.target,
                    generation: b.generation,
                    total: b.total,
                    palette: b.palette,
                    background: b.background,
                    rows,
                });
                self.state = None;
            }
            Kind::State {
                target,
                generation,
                state,
            } => {
                require(self.pending.is_none())?;
                let d = self.document.as_ref().ok_or_else(bad)?;
                require(
                    d.target == target
                        && d.generation == generation
                        && range(state.viewport, d.total)
                        && state.selection.is_none_or(|r| range(r, d.total)),
                )?;
                require(
                    state.occurrences.iter().all(|r| *r > 0 && *r <= d.total)
                        && state.occurrences.windows(2).all(|r| r[0] < r[1]),
                )?;
                let mut end = 0;
                for c in &state.changes {
                    require(
                        range([c.start, c.end], d.total)
                            && c.start > end
                            && (c.kind != ChangeKind::Deleted || c.start == c.end),
                    )?;
                    end = c.end;
                }
                self.state = Some(state);
            }
            Kind::Hide => {
                self.document = None;
                self.state = None;
                self.pending = None;
            }
        }
        self.revision = op.revision;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/minimap-v1.json")).unwrap()
    }
    fn send(model: &mut Minimap, value: &Value) -> Result<(), super::super::Refusal> {
        Minimap::parse(value).and_then(|op| model.apply(op))
    }
    #[test]
    fn source_display_controls_cross_the_real_parser() {
        let cases: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/minimap-display-controls.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let mut model = Minimap::default();
            let mut begin = fixture()["valid"][0].clone();
            begin["insert"] = 1.into();
            begin["total"] = 1.into();
            send(&mut model, &begin).unwrap();
            let mut lines = fixture()["valid"][1].clone();
            lines["lines"] = serde_json::json!([case["row"].clone()]);
            send(&mut model, &lines).unwrap();
            send(&mut model, &fixture()["valid"][2]).unwrap();
            let expected: Line = serde_json::from_value(case["row"].clone()).unwrap();
            assert_eq!(model.document().unwrap().rows()[0], expected);
        }
        let plain: Line =
            serde_json::from_value(serde_json::json!({"text":"x".repeat(20),"spans":[]})).unwrap();
        assert_eq!(plain.bytes(), 68);
    }

    #[test]
    fn fixture_commits_code_and_overlays_atomically() {
        let fixture = fixture();
        let mut model = Minimap::default();
        for op in fixture["valid"].as_array().unwrap().iter().take(4) {
            send(&mut model, op).unwrap();
        }
        assert_eq!(model.document().unwrap().rows().len(), 3);
        let state = model.state().unwrap();
        assert_eq!(state.changes().len(), 2);
        assert_eq!(state.occurrences(), &[1, 3]);
        assert_eq!(state.selection(), Some([2, 3]));
        assert_eq!(model.revision(), 4);
        for invalid in fixture["invalid"].as_array().unwrap() {
            let before = model.clone();
            assert!(send(&mut model, invalid).is_err(), "{invalid}");
            assert_eq!(model, before);
        }
        send(&mut model, &fixture["valid"][4]).unwrap();
        assert!(model.document().is_none());
        assert!(model.state().is_none());
        assert_eq!(model.revision(), 5);
    }
    fn committed() -> Minimap {
        let mut m = Minimap::default();
        for op in fixture()["valid"].as_array().unwrap().iter().take(4) {
            send(&mut m, op).unwrap();
        }
        m
    }
    fn refuse(m: &mut Minimap, v: Value) {
        let before = m.clone();
        assert!(send(m, &v).is_err(), "{v}");
        assert_eq!(*m, before);
    }
    #[test]
    fn minimap_is_a_grid_operation_in_standalone_and_batch_messages() {
        use super::super::{grid, wire};
        let mut grid = grid::GridSurface::new(4, 2);
        let fixture = fixture();
        for op in fixture["valid"].as_array().unwrap().iter().take(4) {
            assert!(matches!(
                wire::parse_message(op).unwrap(),
                wire::Message::Grid(_)
            ));
        }
        grid.apply_all(grid::parse_ops(&serde_json::json!({"type":"batch","ops":fixture["valid"].as_array().unwrap()[..4]})).unwrap()).unwrap();
        assert_eq!(grid.minimap.document().unwrap().total(), 3);
    }

    #[test]
    fn staged_refusals_preserve_all_state_and_hide_retains_watermark() {
        let mut m = committed();
        let mut b = fixture()["valid"][0].clone();
        b["revision"] = 5.into();
        b["generation"] = 2.into();
        send(&mut m, &b).unwrap();
        assert!(m.state().is_none());
        assert!(m.document().is_some());
        let mut lines = fixture()["valid"][1].clone();
        lines["revision"] = 6.into();
        lines["generation"] = 2.into();
        for (text, spans) in [
            ("é", serde_json::json!([[0, 1, 1]])),
            ("abc", serde_json::json!([[1, 3, 1], [0, 1, 2]])),
            ("abc", serde_json::json!([[0, 1, 1], [1, 3, 1]])),
            ("x", serde_json::json!([[0, 1, 3]])),
        ] {
            let mut v = lines.clone();
            v["lines"] = serde_json::json!([{"text":text,"spans":spans}]);
            refuse(&mut m, v);
        }
        let mut v = lines.clone();
        v["offset"] = 1.into();
        refuse(&mut m, v);
        let mut v = lines.clone();
        v["lines"] = serde_json::json!(vec![serde_json::json!({"text":"","spans":[]}); 4]);
        refuse(&mut m, v);
        let mut commit = fixture()["valid"][2].clone();
        commit["generation"] = 2.into();
        commit["revision"] = 6.into();
        refuse(&mut m, commit.clone());
        let mut state = fixture()["valid"][3].clone();
        state["revision"] = 6.into();
        refuse(&mut m, state);
        send(&mut m, &lines).unwrap();
        commit["revision"] = 7.into();
        send(&mut m, &commit).unwrap();
        let mut hide = fixture()["valid"][4].clone();
        hide["revision"] = 8.into();
        send(&mut m, &hide).unwrap();
        assert_eq!(m.visible_width(800.0), 0.0);
        assert!(m.state().is_none());
        refuse(&mut m, fixture()["valid"][0].clone());
        let mut state = fixture()["valid"][3].clone();
        state["revision"] = 9.into();
        refuse(&mut m, state);
    }
    #[test]
    fn new_target_hides_old_source_and_cancels_pending_transaction() {
        let mut m = committed();
        assert_eq!(m.visible_width(479.9), 0.0);
        assert_eq!(m.visible_width(480.0), 160.0);
        let mut b = fixture()["valid"][0].clone();
        b["revision"] = 5.into();
        b["generation"] = 2.into();
        send(&mut m, &b).unwrap();
        let mut b2 = b.clone();
        b2["target"] = "new".into();
        b2["generation"] = 3.into();
        b2["revision"] = 6.into();
        send(&mut m, &b2).unwrap();
        assert!(m.document().is_none());
        assert_eq!(m.visible_width(800.0), 0.0);
        b["revision"] = 7.into();
        refuse(&mut m, b);
        let mut c = fixture()["valid"][2].clone();
        c["revision"] = 7.into();
        c["generation"] = 2.into();
        refuse(&mut m, c);
    }
    #[test]
    fn numeric_and_message_bounds_are_refused_without_mutation() {
        let mut m = committed();
        for number in [
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::json!(9_007_199_254_740_992u64),
            serde_json::json!(0),
        ] {
            let mut v = fixture()["valid"][0].clone();
            v["revision"] = number;
            refuse(&mut m, v);
        }
        for (field, value) in [
            ("total", serde_json::json!(1_000_001)),
            ("palette", serde_json::json!([])),
            ("background", serde_json::json!("#abc")),
            ("target", serde_json::json!("x".repeat(129))),
        ] {
            let mut v = fixture()["valid"][0].clone();
            v["revision"] = 5.into();
            v["generation"] = 2.into();
            v[field] = value;
            refuse(&mut m, v);
        }
        let mut b = fixture()["valid"][0].clone();
        b["revision"] = 5.into();
        b["generation"] = 2.into();
        send(&mut m, &b).unwrap();
        for lines in [
            serde_json::json!([]),
            serde_json::json!(vec![serde_json::json!({"text":"","spans":[]}); 257]),
            serde_json::json!([{"text":"x".repeat(2049),"spans":[]}]),
            serde_json::json!([{"text":"a\tb","spans":[]}]),
            serde_json::json!([{"text":"a\nb","spans":[]}]),
            serde_json::json!(vec![
                serde_json::json!({"text":"x".repeat(2048),"spans":[]});
                128
            ]),
        ] {
            let mut v = fixture()["valid"][1].clone();
            v["revision"] = 6.into();
            v["generation"] = 2.into();
            v["lines"] = lines;
            refuse(&mut m, v);
        }
    }
    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config { cases:64, rng_seed: proptest::test_runner::RngSeed::Fixed(0x6d696e696d6170), ..Default::default() })]
        #[test]
        fn splices_match_a_plain_vector_oracle(start in 0usize..4, delete_seed in 0usize..4, inserted in proptest::collection::vec("[a-zé]{0,8}",0..5)) {
            let mut m=committed();let old=m.document().unwrap();let mut oracle:Vec<String>=old.rows().iter().map(|l|l.text().to_owned()).collect();
            let delete=delete_seed.min(oracle.len()-start);
            if oracle.len()-delete+inserted.len()==0 {return Ok(());}
            oracle.splice(start..start+delete,inserted.clone());
            let b=serde_json::json!({"type":"minimap","kind":"begin","revision":5,"target":old.target(),"generation":2,"base":1,"start":start+1,"delete":delete,"insert":inserted.len(),"total":oracle.len(),"palette":old.palette(),"background":old.background()});
            send(&mut m,&b).unwrap();
            if !inserted.is_empty() {
                let lines:Vec<Value>=inserted.iter().map(|text|serde_json::json!({"text":text,"spans":[]})).collect();
                send(&mut m,&serde_json::json!({"type":"minimap","kind":"lines","revision":6,"target":"b7:w3:1","generation":2,"offset":0,"lines":lines})).unwrap();
            }
            send(&mut m,&serde_json::json!({"type":"minimap","kind":"commit","revision":7,"target":"b7:w3:1","generation":2})).unwrap();
            proptest::prop_assert_eq!(m.document().unwrap().rows().iter().map(|l|l.text().to_owned()).collect::<Vec<_>>(),oracle);
        }
    }

    #[test]
    fn source_storage_caps_apply_to_staging_and_final_splice() {
        let row = serde_json::json!({"text":"x".repeat(512),"spans":(0..256).map(|i|[i,i+1,i%2]).collect::<Vec<_>>()});
        let row_size = 48 + 512 + 256 * 24;
        let count = SOURCE_BYTES / row_size;
        let mut m = Minimap::default();
        let mut b = fixture()["valid"][0].clone();
        b["insert"] = (count + 1).into();
        b["total"] = (count + 1).into();
        send(&mut m, &b).unwrap();
        let mut revision = 2;
        for offset in (0..count).step_by(16) {
            let n = 16.min(count - offset);
            send(&mut m,&serde_json::json!({"type":"minimap","kind":"lines","revision":revision,"target":"b7:w3:1","generation":1,"offset":offset,"lines":vec![row.clone();n]})).unwrap();
            revision += 1;
        }
        refuse(
            &mut m,
            serde_json::json!({"type":"minimap","kind":"lines","revision":revision,"target":"b7:w3:1","generation":1,"offset":count,"lines":[row.clone()]}),
        );
        b["generation"] = 2.into();
        b["revision"] = revision.into();
        b["insert"] = count.into();
        b["total"] = count.into();
        send(&mut m, &b).unwrap();
        revision += 1;
        for offset in (0..count).step_by(16) {
            let n = 16.min(count - offset);
            send(&mut m,&serde_json::json!({"type":"minimap","kind":"lines","revision":revision,"target":"b7:w3:1","generation":2,"offset":offset,"lines":vec![row.clone();n]})).unwrap();
            revision += 1;
        }
        send(&mut m,&serde_json::json!({"type":"minimap","kind":"commit","revision":revision,"target":"b7:w3:1","generation":2})).unwrap();
        revision += 1;
        b["generation"] = 3.into();
        b["revision"] = revision.into();
        b["base"] = 2.into();
        b["start"] = (count + 1).into();
        b["insert"] = 1.into();
        b["total"] = (count + 1).into();
        send(&mut m, &b).unwrap();
        revision += 1;
        send(&mut m,&serde_json::json!({"type":"minimap","kind":"lines","revision":revision,"target":"b7:w3:1","generation":3,"offset":0,"lines":[row]})).unwrap();
        revision += 1;
        refuse(
            &mut m,
            serde_json::json!({"type":"minimap","kind":"commit","revision":revision,"target":"b7:w3:1","generation":3}),
        );
    }
    #[test]
    fn overlay_bounds_and_required_fields_preserve_previous_revision() {
        let mut m = committed();
        for (field, value) in [
            ("selection", serde_json::json!([3, 2])),
            ("occurrences", serde_json::json!([3, 1])),
            ("occurrences", serde_json::json!(vec![1; 100_001])),
            (
                "changes",
                serde_json::json!([{"start":1,"end":2,"kind":"deleted"}]),
            ),
            (
                "changes",
                serde_json::json!([{"start":1,"end":2,"kind":"added"},{"start":2,"end":3,"kind":"modified"}]),
            ),
            ("word", serde_json::json!("é".repeat(513))),
            (
                "changes",
                serde_json::json!(vec![
                    serde_json::json!({"start":1,"end":1,"kind":"modified"});
                    100_001
                ]),
            ),
            (
                "changes",
                serde_json::json!(vec![
                    serde_json::json!({"start":1,"end":1,"kind":"modified"});
                    100_000
                ]),
            ),
        ] {
            let mut v = fixture()["valid"][3].clone();
            v["revision"] = 5.into();
            v[field] = value;
            refuse(&mut m, v);
        }
        for field in ["viewport", "selection", "occurrences", "changes", "word"] {
            let mut v = fixture()["valid"][3].clone();
            v["revision"] = 5.into();
            v.as_object_mut().unwrap().remove(field);
            refuse(&mut m, v);
        }
    }
    #[test]
    fn source_revision_and_staging_are_distinct_from_connection_watermark() {
        let mut m = committed();
        assert_eq!(m.document().unwrap().revision(), 3);
        assert_eq!(m.state().unwrap().revision(), 4);
        assert!(!m.is_staging());
        let mut b = fixture()["valid"][0].clone();
        b["revision"] = 5.into();
        b["generation"] = 2.into();
        send(&mut m, &b).unwrap();
        assert!(m.is_staging());
        assert_eq!(m.revision(), 5);
        assert_eq!(m.document().unwrap().revision(), 3);
    }
}
