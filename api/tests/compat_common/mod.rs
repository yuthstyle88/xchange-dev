//! Reader of the recorded v2 contract of `/api/rpc` and `/api/staff/rpc`.
//!
//! The files are `tests/fixtures/snapshot/*.json` and `tests/fixtures/rpc/*.json`,
//! written by `api/tools/v2-fixtures/gen_compat.mjs`. Nothing here needs a
//! database. Every function panics with the file and step in the message when a
//! file is not what this reader was written against: a fixture that cannot be
//! read is a broken test, never a skipped one.
//!
//! Two things to know before comparing:
//!
//! - [`Step::body`] is a `serde_json::Value`, so it has **no key order** (the
//!   workspace does not switch on `preserve_order`). Where a page shows key
//!   order, compare against [`Fixture::body_text`], which is v2's own text.
//! - Ids v2 made (`SH-…`, `MV-…`, bare uuids) differ from the ones v3 makes;
//!   [`IdMap`] pairs them by position within their kind, so one extra id on
//!   one side shifts the later pairs of that kind (see [`IdMap`]).

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{Map, Value};

/// The six snapshot scenarios, relative to [`fixtures_dir`].
pub const SNAPSHOT_FILES: [&str; 6] = [
    "snapshot/empty.json",
    "snapshot/rates.json",
    "snapshot/shift.json",
    "snapshot/transactions.json",
    "snapshot/custody.json",
    "snapshot/branches.json",
];

/// The three RPC scenarios, relative to [`fixtures_dir`].
pub const RPC_FILES: [&str; 3] = [
    "rpc/transport.json",
    "rpc/engine_actions.json",
    "rpc/ops_actions.json",
];

/// The per-action summary of the nine files above; read it with [`Index::load`].
pub const INDEX_FILE: &str = "rpc/index.json";

/// The key that marks a reference into a file's `shared` table.
pub const SHARED_KEY: &str = "$shared";

/// The committed `api/tests/fixtures` directory.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Somebody the generator signed in. `key` is what a step's `as` names.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Person {
    /// `owner`, `FR1`, `RT1`, …
    pub key: String,
    /// `owner` or `staff`.
    pub kind: String,
    /// `owner`, or the employee's uuid.
    pub id: String,
    pub username: String,
    pub name: String,
    /// `OWNER`, `FRONT`, `BACKEND`, `MANAGER`, `RATES`, `MONITOR`, `FINANCE`,
    /// `SYSTEM`, `DOCUMENTS`, `DOCUMENT_REVIEW`.
    pub role: String,
    /// `None` for the owner.
    pub branch_id: Option<String>,
    /// True only for an employee whose first password is unchanged.
    #[serde(default)]
    pub must_change: bool,
}

impl Person {
    pub fn is_owner(&self) -> bool {
        self.kind == "owner"
    }
}

/// One recorded request and what v2 answered, with every `$shared` reference
/// already replaced.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Step {
    /// The step's number, counted from 1: `file#n` in the contract document.
    pub n: usize,
    /// The server clock when the request arrived (ISO-8601 UTC, milliseconds).
    pub at: String,
    /// Who sent it: a [`Person::key`], `nobody` (no session), or `generator`
    /// (a row written straight to the database, see [`Step::sql`]).
    #[serde(rename = "as")]
    pub sent_as: String,
    /// `POST`, `GET`, `PUT`, `DELETE`. `None` only for a `generator` step.
    pub method: Option<String>,
    /// The path and query (`/api/rpc`, `/api/upload?branch=MAIN`), or a whole
    /// URL when the request went to another host.
    pub route: Option<String>,
    /// The `action` of the JSON body, when the body was built from
    /// `{action, payload}`.
    pub action: Option<String>,
    /// The `payload` of the JSON body; present exactly when `action` is.
    pub payload: Option<Value>,
    /// The request body as sent, when the step is about the body itself.
    pub raw: Option<String>,
    /// For a body too long to write down: its length in UTF-16 units …
    pub raw_length: Option<usize>,
    /// … and how it begins.
    pub raw_start: Option<String>,
    /// Request headers that differ from what the screens send: a value replaces
    /// the header, `None` removes it.
    #[serde(default)]
    pub request_headers: BTreeMap<String, Option<String>>,
    /// For a file upload: which file was sent (`png` or `pdf`).
    pub upload: Option<String>,
    /// What the generator wrote to the database instead of sending a request.
    pub sql: Option<String>,
    /// Why the step is there.
    pub note: Option<String>,
    /// The name of the state this read shows. Reads of one state by different
    /// callers and routes carry the same name.
    pub view: Option<String>,
    /// The recorded HTTP status. `None` only for a `generator` step.
    pub status: Option<u16>,
    /// The response headers as written in the file: present only where they
    /// differ from the file's `defaultHeaders`. Use [`Fixture::response_headers`].
    #[serde(rename = "headers")]
    pub recorded_headers: Option<BTreeMap<String, String>>,
    /// The JSON v2 answered, references expanded.
    pub body: Option<Value>,
    /// The answer when it was not JSON.
    pub text: Option<String>,
}

impl Step {
    /// True when the step is a request (everything but a `generator` step).
    pub fn is_request(&self) -> bool {
        self.method.is_some()
    }

    /// The route without its query string.
    pub fn path(&self) -> Option<&str> {
        self.route
            .as_deref()
            .map(|r| r.split_once('?').map_or(r, |(path, _)| path))
    }

    /// The sentence of a refusal: `body.error` when it is a string.
    pub fn error(&self) -> Option<&str> {
        self.body.as_ref()?.get("error")?.as_str()
    }

    /// The JSON answer; panics when the step has none.
    pub fn json(&self) -> &Value {
        self.body
            .as_ref()
            .unwrap_or_else(|| panic!("step {} has no JSON body", self.n))
    }
}

/// One of the nine step files.
#[derive(Debug, Clone)]
pub struct Fixture {
    /// The name it was loaded by, e.g. `snapshot/empty.json`.
    pub name: String,
    pub source: String,
    pub about: String,
    /// The origin every request was sent from and to (`http://desk.test`).
    pub origin: String,
    /// The response headers of every step that records none of its own.
    pub default_headers: BTreeMap<String, String>,
    pub people: Vec<Person>,
    steps: Vec<Step>,
    raw: String,
    body_spans: Vec<Option<Span>>,
    shared_spans: BTreeMap<String, Span>,
}

impl Fixture {
    /// Reads `tests/fixtures/<name>` and expands every `$shared` reference.
    pub fn load(name: &str) -> Fixture {
        let path = fixtures_dir().join(name);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{name}: cannot read {}: {e}", path.display()));
        let mut root: Map<String, Value> =
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{name}: not a JSON object: {e}"));
        let mut take = |key: &str| {
            root.remove(key)
                .unwrap_or_else(|| panic!("{name}: no `{key}`"))
        };
        let shared = match take("shared") {
            Value::Object(table) => table,
            other => panic!("{name}: `shared` is not an object: {other}"),
        };
        let steps = expand_shared(&take("steps"), &shared);
        let steps: Vec<Step> = serde_json::from_value(steps)
            .unwrap_or_else(|e| panic!("{name}: a step is not what the reader expects: {e}"));
        for (i, step) in steps.iter().enumerate() {
            assert_eq!(step.n, i + 1, "{name}: steps are numbered from 1 in order");
        }
        let text = |v: Value, key: &str| match v {
            Value::String(s) => s,
            other => panic!("{name}: `{key}` is not text: {other}"),
        };
        let source = text(take("source"), "source");
        let about = text(take("about"), "about");
        let origin = text(take("origin"), "origin");
        let default_headers = serde_json::from_value(take("defaultHeaders"))
            .unwrap_or_else(|e| panic!("{name}: `defaultHeaders`: {e}"));
        let people = serde_json::from_value(take("people"))
            .unwrap_or_else(|e| panic!("{name}: `people`: {e}"));

        let top = object_spans(&raw, value_start(&raw, 0));
        let find = |key: &str| {
            top.iter()
                .find(|(k, _)| k == key)
                .map(|(_, span)| *span)
                .unwrap_or_else(|| panic!("{name}: no `{key}` in the text"))
        };
        let shared_spans = object_spans(&raw, find("shared").0).into_iter().collect();
        let body_spans = array_spans(&raw, find("steps").0)
            .into_iter()
            .map(|step| {
                object_spans(&raw, step.0)
                    .into_iter()
                    .find(|(k, _)| k == "body")
                    .map(|(_, span)| span)
            })
            .collect();
        Fixture {
            name: name.to_string(),
            source,
            about,
            origin,
            default_headers,
            people,
            steps,
            raw,
            body_spans,
            shared_spans,
        }
    }

    /// Every step, in the order recorded.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Step `n`, counted from 1 as the contract document counts (`file#n`).
    pub fn step(&self, n: usize) -> &Step {
        n.checked_sub(1)
            .and_then(|i| self.steps.get(i))
            .unwrap_or_else(|| panic!("{}: no step {n}", self.name))
    }

    /// The steps that read the state called `view`, in the order recorded.
    pub fn steps_with_view(&self, view: &str) -> Vec<&Step> {
        self.steps
            .iter()
            .filter(|s| s.view.as_deref() == Some(view))
            .collect()
    }

    /// The distinct `view` names, in order of first appearance.
    pub fn views(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for view in self.steps.iter().filter_map(|s| s.view.as_deref()) {
            if !seen.contains(&view) {
                seen.push(view);
            }
        }
        seen
    }

    /// The person a step's `as` names.
    pub fn person(&self, key: &str) -> Option<&Person> {
        self.people.iter().find(|p| p.key == key)
    }

    /// Who sent the step: `None` for `nobody` and `generator`.
    pub fn sender(&self, step: &Step) -> Option<&Person> {
        self.person(&step.sent_as)
    }

    /// The response headers v2 sent: the step's own where recorded, the file's
    /// `defaultHeaders` otherwise.
    pub fn response_headers<'a>(&'a self, step: &'a Step) -> &'a BTreeMap<String, String> {
        step.recorded_headers
            .as_ref()
            .unwrap_or(&self.default_headers)
    }

    /// The JSON body of step `n` as v2 sent it: references expanded, no spaces,
    /// **keys in v2's order**. `None` when the step has no JSON body.
    pub fn body_text(&self, n: usize) -> Option<String> {
        let _ = self.step(n);
        let span = self.body_spans[n - 1]?;
        let mut out = String::new();
        self.write_compact(span.0, &mut out);
        Some(out)
    }

    fn write_compact(&self, at: usize, out: &mut String) {
        let raw = &self.raw;
        match raw.as_bytes()[at] {
            b'{' => {
                let entries = object_spans(raw, at);
                if let [(key, value)] = entries.as_slice()
                    && key == SHARED_KEY
                {
                    let id: String = serde_json::from_str(&raw[value.0..value.1])
                        .unwrap_or_else(|e| panic!("{}: a reference is not text: {e}", self.name));
                    let target = self
                        .shared_spans
                        .get(&id)
                        .unwrap_or_else(|| panic!("{}: no shared value `{id}`", self.name));
                    return self.write_compact(target.0, out);
                }
                out.push('{');
                for (i, (key, value)) in object_entries(raw, at).iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&raw[key.0..key.1]); // the key exactly as written
                    out.push(':');
                    self.write_compact(value.0, out);
                }
                out.push('}');
            }
            b'[' => {
                out.push('[');
                for (i, item) in array_spans(raw, at).iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    self.write_compact(item.0, out);
                }
                out.push(']');
            }
            _ => out.push_str(&raw[at..value_end(raw, at)]),
        }
    }
}

/// Replaces every `{"$shared":"sN"}` in `value` by `shared.sN`, recursively:
/// a shared value may itself hold references.
pub fn expand_shared(value: &Value, shared: &Map<String, Value>) -> Value {
    match value {
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| expand_shared(v, shared)).collect())
        }
        Value::Object(map) => {
            if let Some(reference) = map.get(SHARED_KEY) {
                let id = reference
                    .as_str()
                    .filter(|_| map.len() == 1)
                    .unwrap_or_else(|| panic!("a malformed reference: {value}"));
                let target = shared
                    .get(id)
                    .unwrap_or_else(|| panic!("no shared value `{id}`"));
                return expand_shared(target, shared);
            }
            Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), expand_shared(v, shared)))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

/// True when `value` holds the key `$shared` at any depth.
pub fn has_shared_reference(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().any(has_shared_reference),
        Value::Object(map) => {
            map.contains_key(SHARED_KEY) || map.values().any(has_shared_reference)
        }
        _ => false,
    }
}

// ── rpc/index.json ───────────────────────────────────────────────────────────

/// `rpc/index.json`: per action and route, what the nine step files recorded.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub source: String,
    pub about: String,
    /// How many recorded answers each claim of the generator was checked on:
    /// `writeAnswersAreTheSnapshot`, `deskConfigIsPartOfTheSnapshot`,
    /// `staffPathIsTheSnapshotRewritten`.
    pub claims: BTreeMap<String, u64>,
    /// Action name → route → what was recorded there.
    pub actions: BTreeMap<String, BTreeMap<String, ActionRecord>>,
    /// Every distinct refusal sentence and how many steps were answered with it.
    pub sentences: Vec<(String, u64)>,
}

/// What one action was seen to do on one route.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionRecord {
    /// The roles a recorded step was accepted from.
    pub accepted_from: Vec<String>,
    pub refusals: Vec<RecordedRefusal>,
}

/// One distinct refusal of an action on a route.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedRefusal {
    pub status: u16,
    pub error: String,
    /// The roles it was said to.
    pub from: Vec<String>,
    /// The first step that shows it, as `rpc/engine_actions.json#145`.
    pub first: String,
}

impl Index {
    /// Reads `tests/fixtures/rpc/index.json`.
    pub fn load() -> Index {
        let path = fixtures_dir().join(INDEX_FILE);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{INDEX_FILE}: cannot read {}: {e}", path.display()));
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("{INDEX_FILE}: not what the reader expects: {e}"))
    }
}

// ── ids the server makes ─────────────────────────────────────────────────────

/// The ids a server makes, found anywhere inside the strings of `value` (so
/// also inside `/files/<uuid>`), each once, in order of first appearance:
/// `SH-<uuid>`, `MV-<uuid>` and bare uuids. Object keys are not looked at.
///
/// "Order" is the order `serde_json` walks a `Value`: arrays as written, object
/// keys **sorted by name**. Two answers of the same shape are walked alike,
/// which is all [`IdMap`] needs.
pub fn server_ids(value: &Value) -> Vec<String> {
    let mut found = Vec::new();
    collect_ids(value, &mut found);
    found
}

fn collect_ids(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            for (start, end) in id_spans(text) {
                let id = &text[start..end];
                if !found.iter().any(|known| known == id) {
                    found.push(id.to_string());
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_ids(v, found)),
        Value::Object(map) => map.values().for_each(|v| collect_ids(v, found)),
        _ => {}
    }
}

/// `SH`, `MV` or `uuid`: ids are paired within their own kind.
fn id_kind(id: &str) -> &'static str {
    match id.get(..3) {
        Some("SH-") => "SH",
        Some("MV-") => "MV",
        _ => "uuid",
    }
}

const UUID_LEN: usize = 36;

fn is_uuid_at(bytes: &[u8], at: usize) -> bool {
    let Some(candidate) = bytes.get(at..at + UUID_LEN) else {
        return false;
    };
    let shaped = candidate.iter().enumerate().all(|(i, b)| match i {
        8 | 13 | 18 | 23 => *b == b'-',
        _ => b.is_ascii_hexdigit(),
    });
    let alone_before = at == 0 || !bytes[at - 1].is_ascii_alphanumeric();
    let alone_after = bytes
        .get(at + UUID_LEN)
        .is_none_or(|b| !b.is_ascii_alphanumeric());
    shaped && alone_before && alone_after
}

/// The byte ranges of the server-made ids inside `text`.
fn id_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let prefixed = (bytes[at..].starts_with(b"SH-") || bytes[at..].starts_with(b"MV-"))
            && (at == 0 || !bytes[at - 1].is_ascii_alphanumeric());
        // `SH-` is followed by a uuid whose own "alone before" test sees the `-`
        if prefixed && is_uuid_at(bytes, at + 3) {
            spans.push((at, at + 3 + UUID_LEN));
            at += 3 + UUID_LEN;
        } else if is_uuid_at(bytes, at) {
            spans.push((at, at + UUID_LEN));
            at += UUID_LEN;
        } else {
            at += 1;
        }
    }
    spans
}

/// Pairs the ids of a recorded answer with the ids of the answer under test.
///
/// Within a kind (`SH-…`, `MV-…`, bare uuids) ids pair by **position**: the
/// n-th distinct one of the recorded answer pairs with the n-th of the actual
/// one. That position is the cost: one server-made id more or fewer earlier
/// on one side shifts every later pair of that kind. An echoed `requestId`
/// pairs with itself only while it stands at the same position among the
/// uuids of its kind on both sides — one extra uuid in front of it on the
/// actual side pairs the echo with that extra id, and the comparison that
/// follows fails at the shifted id rather than at the extra one. To place
/// the difference exactly: learn from an answer where the extra id does not
/// occur first, or compare the extra id's field on its own. A pair once
/// learned is kept: learn from the steps in the order they were recorded.
///
/// ```ignore
/// let mut ids = IdMap::new();
/// ids.learn(step.json(), &actual);
/// assert_eq!(ids.to_actual(step.json()), actual);
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IdMap {
    to_actual: BTreeMap<String, String>,
    /// Recorded customer id → the earlier customer v3 answered instead ([`SAME_CUSTOMER_AS`]).
    customers: BTreeMap<String, String>,
}

impl IdMap {
    pub fn new() -> IdMap {
        IdMap::default()
    }

    /// A map learned from one pair of answers.
    pub fn pair(recorded: &Value, actual: &Value) -> IdMap {
        let mut map = IdMap::new();
        map.learn(recorded, actual);
        map
    }

    /// Adds the pairs this pair of answers shows. Ids already paired (on either
    /// side) are left out before the rest is paired by position, so an answer
    /// that repeats old ids and adds new ones teaches only the new ones. Extra
    /// ids on one side stay unpaired: the comparison that follows shows them.
    pub fn learn(&mut self, recorded: &Value, actual: &Value) {
        let recorded_ids = server_ids(recorded);
        let actual_ids = server_ids(actual);
        for kind in ["SH", "MV", "uuid"] {
            let known_actual: Vec<&String> = self.to_actual.values().collect();
            let fresh_recorded: Vec<&String> = recorded_ids
                .iter()
                .filter(|id| id_kind(id) == kind && !self.to_actual.contains_key(*id))
                .collect();
            let fresh_actual: Vec<&String> = actual_ids
                .iter()
                .filter(|id| id_kind(id) == kind && !known_actual.contains(id))
                .collect();
            let pairs: Vec<(String, String)> = fresh_recorded
                .into_iter()
                .zip(fresh_actual)
                .map(|(r, a)| (r.clone(), a.clone()))
                .collect();
            self.to_actual.extend(pairs);
        }
    }

    /// CI-4 (#651): from now on the recorded customer id `recorded` stands for the actual customer `actual`
    /// (a whole-string match: customer ids are whole values in v2's answers). Only [`SAME_CUSTOMER_AS`] adds these.
    pub fn alias_customer(&mut self, recorded: &str, actual: &str) {
        self.customers
            .insert(recorded.to_owned(), actual.to_owned());
    }

    /// Whether `recorded` is a customer id a [`SAME_CUSTOMER_AS`] step aliased.
    pub fn is_aliased_customer(&self, recorded: &str) -> bool {
        self.customers.contains_key(recorded)
    }

    /// The actual id paired with a recorded one.
    pub fn actual(&self, recorded_id: &str) -> Option<&str> {
        self.to_actual.get(recorded_id).map(String::as_str)
    }

    /// The recorded id paired with an actual one.
    pub fn recorded(&self, actual_id: &str) -> Option<&str> {
        self.to_actual
            .iter()
            .find(|(_, actual)| actual.as_str() == actual_id)
            .map(|(recorded, _)| recorded.as_str())
    }

    pub fn len(&self) -> usize {
        self.to_actual.len()
    }

    pub fn is_empty(&self) -> bool {
        self.to_actual.is_empty()
    }

    /// `recorded` with every paired id replaced by its actual one, inside
    /// strings too. Unpaired ids are left as recorded.
    pub fn to_actual(&self, recorded: &Value) -> Value {
        match recorded {
            Value::String(text) if self.customers.contains_key(text) => {
                Value::String(self.customers[text].clone())
            }
            Value::String(text) => {
                let mut out = String::with_capacity(text.len());
                let mut copied = 0;
                for (start, end) in id_spans(text) {
                    out.push_str(&text[copied..start]);
                    let id = &text[start..end];
                    out.push_str(self.actual(id).unwrap_or(id));
                    copied = end;
                }
                out.push_str(&text[copied..]);
                Value::String(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(|v| self.to_actual(v)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), self.to_actual(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

// ── the text of a file, by position ──────────────────────────────────────────
//
// `serde_json::Value` forgets key order, and v2's key order is part of what was
// recorded. These few functions find where a value starts and ends in the text
// of a file that `serde_json` has already accepted, so they check nothing.

/// `(start, end)` of a JSON value in the text of the file.
type Span = (usize, usize);

/// The first byte at or after `at` that is not white space.
fn value_start(raw: &str, at: usize) -> usize {
    let bytes = raw.as_bytes();
    let mut at = at;
    while bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    at
}

/// One past the last byte of the value that starts at `start`.
fn value_end(raw: &str, start: usize) -> usize {
    let bytes = raw.as_bytes();
    let string_end = |from: usize| {
        let mut at = from + 1;
        while bytes[at] != b'"' {
            at += if bytes[at] == b'\\' { 2 } else { 1 };
        }
        at + 1
    };
    match bytes[start] {
        b'"' => string_end(start),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut at = start;
            loop {
                match bytes[at] {
                    b'"' => {
                        at = string_end(at);
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return at + 1;
                        }
                    }
                    _ => {}
                }
                at += 1;
            }
        }
        _ => {
            let mut at = start;
            while !matches!(bytes[at], b',' | b'}' | b']') && !bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            at
        }
    }
}

/// The entries of the object that starts at `start`: the span of each key (with
/// its quotes) and of its value.
fn object_entries(raw: &str, start: usize) -> Vec<(Span, Span)> {
    let bytes = raw.as_bytes();
    let mut entries = Vec::new();
    let mut at = value_start(raw, start + 1);
    while bytes[at] != b'}' {
        let key_end = value_end(raw, at);
        let value_at = value_start(raw, value_start(raw, key_end) + 1); // past the colon
        let end = value_end(raw, value_at);
        entries.push(((at, key_end), (value_at, end)));
        at = value_start(raw, end);
        if bytes[at] == b',' {
            at = value_start(raw, at + 1);
        }
    }
    entries
}

/// The same with the keys read: key and the span of its value.
fn object_spans(raw: &str, start: usize) -> Vec<(String, Span)> {
    object_entries(raw, start)
        .into_iter()
        .map(|(key, value)| {
            let key = serde_json::from_str(&raw[key.0..key.1]).expect("an object key is text");
            (key, value)
        })
        .collect()
}

/// The spans of the items of the array that starts at `start`.
fn array_spans(raw: &str, start: usize) -> Vec<Span> {
    let bytes = raw.as_bytes();
    let mut items = Vec::new();
    let mut at = value_start(raw, start + 1);
    while bytes[at] != b']' {
        let end = value_end(raw, at);
        items.push((at, end));
        at = value_start(raw, end);
        if bytes[at] == b',' {
            at = value_start(raw, at + 1);
        }
    }
    items
}

// ── decided customer differences (CI-4 #651) ─────────────────────────────────

/// A recorded `reserveCustomer` that v2 answered with a new customer, and that v3 answers with an earlier
/// step's customer. Lane controller's ruling (A), 2026-10-09: exactly these three, each with its cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SameCustomer {
    pub fixture: &'static str,
    pub step: usize,
    pub same_as: usize,
    pub reason: &'static str,
}

pub const SAME_CUSTOMER_AS: [SameCustomer; 3] = [
    SameCustomer {
        fixture: "rpc/engine_actions.json",
        step: 124,
        same_as: 77,
        reason: "owner 2026-10-09: DEMO shares LIVE",
    },
    SameCustomer {
        fixture: "snapshot/transactions.json",
        step: 8,
        same_as: 4,
        reason: "owner 2026-10-09: DEMO shares LIVE",
    },
    SameCustomer {
        fixture: "snapshot/branches.json",
        step: 10,
        same_as: 9,
        reason: "K1 (customer per company)",
    },
];

/// The decided customer difference of this step, if it has one. A use without a reason panics.
pub fn same_customer_as(fixture: &str, step: usize) -> Option<SameCustomer> {
    let found = SAME_CUSTOMER_AS
        .iter()
        .find(|s| s.fixture == fixture && s.step == step)
        .copied();
    if let Some(s) = found {
        assert!(
            !s.reason.trim().is_empty(),
            "{fixture}#{step}: a SameCustomerAs needs its reason"
        );
    }
    found
}

/// The answer a [`SameCustomer`] step must give: the recorded one with `customerId` the earlier step's actual
/// customer and `existing: true` (that customer already existed); nothing else changes. Panics, naming the
/// step, when the recording is not a new customer.
pub fn same_customer_answer(recorded: &Value, earlier_actual: &str, label: &str) -> Value {
    assert_eq!(
        recorded.get("existing"),
        Some(&Value::Bool(false)),
        "{label}: SameCustomerAs applies only where v2 made a new customer"
    );
    let mut expected = recorded.clone();
    expected["customerId"] = Value::String(earlier_actual.to_owned());
    expected["existing"] = Value::Bool(true);
    expected
}

/// The answer a save for an aliased customer must give: the recorded one with `receiptUrl` the URL v3 gave,
/// accepted only when it is `/files/receipt-<SHA-256 of the stored receipt>` (the receipt names its customer, so
/// its hash moves with the alias). Nothing else changes. Panics, naming the step, on any other URL.
pub fn same_customer_save_answer(
    recorded: &Value,
    actual_url: &str,
    stored_receipt: &[u8],
    label: &str,
) -> Value {
    use sha2::{Digest, Sha256};
    let computed = format!("/files/receipt-{:x}", Sha256::digest(stored_receipt));
    assert_eq!(
        actual_url, computed,
        "{label}: the receipt URL is not the hash of the stored receipt"
    );
    let mut expected = recorded.clone();
    expected["receiptUrl"] = Value::String(actual_url.to_owned());
    expected
}

/// The two recordings whose snapshots v3 answers with the company's customers (list_live company-wide, owner
/// 2026-10-09, (c)). Lane controller's ruling, 2026-10-09: exactly these two fixtures.
pub const COMPANY_CUSTOMERS_FIXTURES: [&str; 2] =
    ["rpc/engine_actions.json", "snapshot/branches.json"];

pub const COMPANY_CUSTOMERS_REASON: &str = "list_live company-wide, owner 2026-10-09, (c)";

fn is_main_answer(step: &Step) -> bool {
    step.body
        .as_ref()
        .and_then(|b| b.get("branchId"))
        .and_then(Value::as_str)
        == Some("MAIN")
}

fn carries_customers(step: &Step) -> bool {
    step.body
        .as_ref()
        .is_some_and(|b| b.get("customers").is_some())
}

/// The steps the rule applies to, per fixture (lane controller's ruling 2026-10-09: explicit lists, not a
/// predicate). `company_customers_rule_covers_every_non_main_customers_answer` walks both fixtures and proves every
/// non-MAIN answer carrying `customers` is here or in COMPANY_CUSTOMERS_EXCEPTIONS, never both, never neither.
pub const COMPANY_CUSTOMERS_STEPS: [(&str, &[usize]); 2] = [
    (
        "rpc/engine_actions.json",
        &[
            158, 160, 164, 166, 167, 168, 169, 170, 171, 172, 173, 174, 175, 176, 177, 178, 179,
            180, 181, 182, 183,
        ],
    ),
    (
        "snapshot/branches.json",
        &[2, 4, 5, 6, 7, 8, 13, 15, 21, 23, 25, 27, 29, 33, 38],
    ),
];

/// Non-MAIN answers carrying `customers` that stay exact, each with its reason.
pub const COMPANY_CUSTOMERS_EXCEPTIONS: [(&str, usize, &str); 1] = [(
    "snapshot/branches.json",
    31,
    "restricted staff view: empty in v2 and v3",
)];

/// Every recorded answer of `fixture` carrying `customers` at a branch other than MAIN (what the two lists above
/// must cover between them).
pub fn non_main_customers_answers(fixture: &Fixture) -> Vec<usize> {
    fixture
        .steps()
        .iter()
        .filter(|s| carries_customers(s) && !is_main_answer(s))
        .map(|s| s.n)
        .collect()
}

/// The steps of `fixture` the rule applies to; empty for any other fixture.
pub fn company_customers_steps(fixture: &Fixture) -> Vec<usize> {
    COMPANY_CUSTOMERS_STEPS
        .iter()
        .find(|(name, _)| *name == fixture.name)
        .map(|(_, steps)| steps.to_vec())
        .unwrap_or_default()
}

/// The `customers` a step of the rule must answer: those of the latest earlier **recorded** MAIN answer, `[]` when
/// none precedes it. `None` when the rule does not apply to this step (it never applies to a MAIN answer).
pub fn company_customers(fixture: &Fixture, n: usize) -> Option<Value> {
    if !company_customers_steps(fixture).contains(&n) {
        return None;
    }
    let latest_main = fixture
        .steps()
        .iter()
        .rev()
        .find(|s| s.n < n && carries_customers(s) && is_main_answer(s));
    Some(match latest_main {
        Some(s) => json_customers(s.json()),
        None => Value::Array(Vec::new()),
    })
    .map(|list| customers_shaped_for(fixture.step(n).action.as_deref().unwrap_or(""), &list))
}

/// The steps whose CompanyCustomers expectation `customers_shaped_for` changes (its source answer had the other
/// shape). Pinned by `same_customer_rule_test`.
pub fn member_shape_rewritten_steps(fixture: &Fixture) -> Vec<usize> {
    company_customers_steps(fixture)
        .into_iter()
        .filter(|&n| {
            let source = fixture
                .steps()
                .iter()
                .rev()
                .find(|s| s.n < n && carries_customers(s) && is_main_answer(s))
                .map(|s| json_customers(s.json()))
                .unwrap_or(Value::Array(Vec::new()));
            let action = fixture.step(n).action.as_deref().unwrap_or("");
            customers_shaped_for(action, &source) != source
        })
        .collect()
}

/// The six actions v2 answers without `member`/`memberVerifiedAt` on each customer. The adapter keeps no constant
/// of them, so `same_customer_rule_test` proves this list equals the six `write_answer` names
/// (`api/src/application/compat/snapshot.rs`) and what every recorded answer shows.
pub const WRITE_ANSWER_ACTIONS: [&str; 6] = [
    "adminSaveRate",
    "adminCreateBranch",
    "adminOpenShift",
    "adminMoveStock",
    "adminCloseShift",
    "adminAbandonRequest",
];

pub const MEMBER_SHAPE_REASON: &str = "new branch: no member rows; v2 write answers carry no member fields (snapshot.rs write_answer)";

/// `customers` in the shape v2 answers `action` with: a write answer without `member`/`memberVerifiedAt`; a read
/// answer with them, kept as recorded when the source has them and `false`/`null` when it does not (the source was
/// a write answer).
pub fn customers_shaped_for(action: &str, customers: &Value) -> Value {
    let write = WRITE_ANSWER_ACTIONS.contains(&action);
    let Value::Array(rows) = customers else {
        return customers.clone();
    };
    Value::Array(
        rows.iter()
            .map(|row| {
                let mut row = row.clone();
                if let Value::Object(fields) = &mut row {
                    if write {
                        fields.remove("member");
                        fields.remove("memberVerifiedAt");
                    } else {
                        fields.entry("member").or_insert(Value::Bool(false));
                        fields.entry("memberVerifiedAt").or_insert(Value::Null);
                    }
                }
                row
            })
            .collect(),
    )
}

fn json_customers(answer: &Value) -> Value {
    answer
        .get("customers")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()))
}

/// `recorded` with its `customers` replaced by `reference`'s `customers`; nothing else changes. Panics, naming the
/// step, when either has no `customers`.
pub fn with_customers_of(recorded: &Value, reference: &Value, label: &str) -> Value {
    let customers = reference
        .get("customers")
        .unwrap_or_else(|| panic!("{label}: the referenced step recorded no customers"))
        .clone();
    assert!(
        recorded.get("customers").is_some(),
        "{label}: CompanyCustomers applies only to an answer with customers"
    );
    let mut expected = recorded.clone();
    expected["customers"] = customers;
    expected
}
