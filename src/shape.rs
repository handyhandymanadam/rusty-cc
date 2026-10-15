//! The last step before a record is written: the detail level (token budget rule A), the removal of repeated and empty
//! fields (B), stable-mode cleaning (F) and the size budget (G). Every record goes through `write`.
//!
//! `full` is what the first version wrote. `normal` (the default in agent mode) leaves out `rendered` (clang's own text, which repeats
//! the message, locations and notes), `files`, `symbols`, every empty, null or zero field (an absent field means empty, zero or null) and
//! whatever another field already says. `brief` is code, message, one place, one fix and counts.
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Detail { Brief, #[default] Normal, Full }

impl Detail {
    pub fn parse(s: &str) -> Option<Detail> { match s { "brief" => Some(Detail::Brief), "normal" => Some(Detail::Normal), "full" => Some(Detail::Full), _ => None } }
}

/// stderr size budget per command at `normal` and `brief` (bytes): past SOFT only errors and the records that were asked for go on,
/// past HARD nothing but the summary
const SOFT_BUDGET: usize = 12_000;
const HARD_BUDGET: usize = 20_000;

struct State { detail: Detail, stable: bool, fix_preview: bool, cwd: Option<String>, spent: usize, cut: BTreeMap<String, u64>, file_given: bool }

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

pub fn init(detail: Detail, stable: bool, fix_preview: bool, file_given: bool) {
    let _ = STATE.set(Mutex::new(State { detail, stable, fix_preview, cwd: std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()), spent: 0, cut: BTreeMap::new(), file_given }));
}

/// shape one record, apply the budget, write it to `out` and to the RUSTY_AGENT_OUT file (which gets every shaped record, budget or not)
pub fn write(out: &mut impl Write, v: &Value) {
    let Some(m) = STATE.get() else { let s = v.to_string(); let _ = writeln!(out, "{s}"); crate::tee::line(&s); return };
    let mut st = m.lock().unwrap_or_else(|e| e.into_inner());
    let mut v = v.clone();
    if v["type"] == "summary" && !st.cut.is_empty() && st.detail != Detail::Full {
        let pointer = if st.file_given { crate::tee::where_all() } else { "--agent-detail=full shows everything; RUSTY_AGENT_OUT=FILE or RUSTY_AGENT_FD=N keeps every record" };
        v["records_omitted"] = json!({"by_type": st.cut, "more": pointer});
    }
    let Some(v) = finish(v, &st) else { return };
    let s = v.to_string();
    crate::tee::line(&s);
    if st.detail != Detail::Full && v["type"] != "summary" {
        let low = matches!(v["type"].as_str(), Some("diagnostic" | "remark")) && !matches!(v["severity"].as_str(), Some("error" | "fatal"));
        if st.spent + s.len() + 1 > HARD_BUDGET || (low && st.spent + s.len() + 1 > SOFT_BUDGET) {
            let t = v["type"].as_str().unwrap_or("record").to_string(); *st.cut.entry(t).or_default() += 1; return;
        }
    }
    st.spent += s.len() + 1;
    let _ = writeln!(out, "{s}");
}

fn finish(mut v: Value, st: &State) -> Option<Value> {
    if st.stable { stable(&mut v); }
    if st.detail == Detail::Full { return Some(v); }
    let ty = v["type"].as_str().unwrap_or("").to_string();
    let brief = st.detail == Detail::Brief;
    match ty.as_str() {
        "diagnostic" | "remark" => if brief { v = brief_diag(&v) } else { diag_normal(&mut v, st) },
        "summary" => summary_normal(&mut v, brief),
        "fix-preview" => fix_normal(&mut v, brief),
        "missing-header" => header_normal(&mut v, brief),
        "effective-flags" => effective_normal(&mut v, brief),
        "why-not" => whynot_normal(&mut v, brief),
        "compile-time" => time_normal(&mut v, brief),
        _ => {}
    }
    drop_empty(&mut v);
    Some(v)
}

/// stable mode: no temp names, no clock values, no private temp directories
fn stable(v: &mut Value) {
    fn walk(v: &mut Value) {
        match v {
            Value::String(s) => { let n = crate::agent::normalize_tmp(s); if n != *s { *s = n; } }
            Value::Array(a) => a.iter_mut().for_each(walk),
            Value::Object(m) => m.values_mut().for_each(walk),
            _ => {}
        }
    }
    walk(v);
    if v["type"] == "summary" { remove(v, &["elapsed_ms"]); }
}

fn remove(v: &mut Value, keys: &[&str]) { if let Some(m) = v.as_object_mut() { for k in keys { m.shift_remove(*k); } } }

/// removes null, "", [] and {} members (an absent field means empty). `text` is kept: an empty replacement text is a deletion.
fn drop_empty(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for (_, x) in m.iter_mut() { drop_empty(x); }
            m.retain(|k, x| k == "text" || !(x.is_null() || x.as_str().is_some_and(str::is_empty) || x.as_array().is_some_and(Vec::is_empty) || x.as_object().is_some_and(|o| o.is_empty())));
        }
        Value::Array(a) => a.iter_mut().for_each(drop_empty),
        _ => {}
    }
}

/// removes members that are false / 0 (their absence means the same)
fn drop_defaults(v: &mut Value, keys: &[&str]) {
    if let Some(m) = v.as_object_mut() { m.retain(|k, x| !(keys.contains(&k.as_str()) && (x == &Value::Bool(false) || x.as_u64() == Some(0)))); }
}

const MORE: &str = "--agent-detail=full shows everything";

/// keeps the first `n` of an array and records how many were left out as `<key>_omitted`
fn cap(m: &mut Map<String, Value>, key: &str, n: usize) -> bool {
    let Some(Value::Array(a)) = m.get_mut(key) else { return false };
    if a.len() <= n { return false; }
    let extra = a.len() - n; a.truncate(n);
    m.insert(format!("{key}_omitted"), extra.into());
    true
}

/// a suggestion that only restates a fix-it ("insert ';' at f.c:4:13") is the fix-it's second copy
fn restates_fixit(s: &Value) -> bool {
    s["message"].as_str().is_some_and(|m| m.starts_with("insert '") && m.contains("' at ") || m.starts_with("remove the text at ") || m.starts_with("replace the text at "))
}

/// {file,line,column,end_line,end_column,role} -> without the end when it is the start
fn slim_loc(l: &mut Value) {
    if l["end_line"] == l["line"] { remove(l, &["end_line"]); if l["end_column"] == l["column"] { remove(l, &["end_column"]); } }
}

fn diag_normal(v: &mut Value, st: &State) {
    let m_message = v["message"].as_str().unwrap_or("").to_string();
    let Some(m) = v.as_object_mut() else { return };
    for k in ["rendered", "files", "symbols"] { m.shift_remove(k); }
    for k in ["suppressed", "notes_omitted", "locations_omitted"] { if m.get(k).and_then(|x| x.as_u64()) == Some(0) { m.shift_remove(k); } }
    let mut more = m.get("notes_omitted").is_some() || m.get("locations_omitted").is_some();
    tidy_paths(m);
    // follow-ons: the place without its extent, at most 5
    let (rcode, rsev, rmsg) = (m.get("code").cloned(), m.get("severity").cloned(), m.get("message").cloned());
    if let Some(Value::Array(fo)) = m.get_mut("follow_on") {
        for f in fo.iter_mut() {
            if let Some(l) = f.get_mut("location") { remove(l, &["role", "end_line", "end_column"]); }
            // what a follower says exactly as its root does ("the same name again") is not said again
            if f.get("code") == rcode.as_ref() { remove(f, &["code"]); }
            if f.get("severity") == rsev.as_ref() { remove(f, &["severity"]); }
            if f.get("message") == rmsg.as_ref() { remove(f, &["message"]); }
        }
    }
    more |= cap(m, "follow_on", 5);
    // suggestions that restate a fix-it go; with --fix-preview the preview carries the edits, so the fix-its go too
    if let Some(Value::Array(sg)) = m.get_mut("suggestions") { sg.retain(|s| !restates_fixit(s)); for s in sg.iter_mut() { if s["option"].is_null() { remove(s, &["option"]); } } }
    if st.fix_preview { m.shift_remove("fixits"); }
    else if let Some(Value::Array(fx)) = m.get_mut("fixits") { for f in fx.iter_mut() { if f["start"]["offset"].is_null() { remove(&mut f["start"], &["offset"]); remove(&mut f["end"], &["offset"]); } } }
    // locations: a note's place is in the note's text ("file:line:col: message")
    let notes: Vec<String> = m.get("notes").and_then(|n| n.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    if let Some(Value::Array(locs)) = m.get_mut("locations") {
        locs.retain(|l| !(l["role"] == "note" && notes.iter().any(|n| n.starts_with(&format!("{}:{}:{}: ", l["file"].as_str().unwrap_or(""), l["line"], l["column"])))));
        for l in locs.iter_mut() { slim_loc(l); }
    }
    more |= cap(m, "notes", 6);
    more |= cap(m, "include_stack", 3);
    // the sanitizer block: the summary line repeats the message and the place, the region is in the first note, the stacks are the
    // notes and locations (user frames) plus the runtime's own frames, which only `full` keeps
    if let Some(Value::Object(s)) = m.get_mut("sanitizer") {
        let had_stacks = s.contains_key("stacks");
        for k in ["summary", "region", "stacks"] { s.shift_remove(k); }
        // the message already says "WRITE of size 4 at 0x..."
        let msg = m_message.as_str();
        if s.get("address").and_then(|a| a.as_str()).is_some_and(|a| msg.contains(a)) { for k in ["access", "size", "address"] { s.shift_remove(k); } }
        more |= had_stacks;
    }
    if more { m.insert("more".into(), MORE.into()); }
}

/// `/usr/bin/../lib/gcc/.../../include/c++/x.h` -> `/usr/include/c++/x.h` in locations and in the paths that start a note
fn tidy_paths(m: &mut Map<String, Value>) {
    if let Some(Value::Array(locs)) = m.get_mut("locations") { for l in locs.iter_mut() { if let Some(f) = l["file"].as_str() { l["file"] = crate::paths::tidy(f).into(); } } }
    if let Some(Value::Array(ns)) = m.get_mut("notes") { for n in ns.iter_mut() { if let Some(t) = n.as_str() { *n = crate::paths::tidy_prefixed(t).into(); } } }
    if let Some(Value::Array(fo)) = m.get_mut("follow_on") { for f in fo.iter_mut() { if let Some(p) = f["location"]["file"].as_str() { f["location"]["file"] = crate::paths::tidy(p).into(); } } }
}

/// brief: code, message, one place, one fix, counts
fn brief_diag(v: &Value) -> Value {
    let mut o = Map::new();
    for k in ["schema", "type", "code", "severity", "message", "flag"] { if !v[k].is_null() { o.insert(k.into(), v[k].clone()); } }
    if v["werror"] == true { o.insert("werror".into(), true.into()); }
    if let Some(p) = v["locations"].as_array().and_then(|a| a.iter().find(|l| l["role"] == "primary")) {
        o.insert("location".into(), json!({"file": p["file"], "line": p["line"], "column": p["column"]}));
    }
    if let Some(f) = v["fixits"].as_array().and_then(|a| a.iter().find(|f| f["alternative"].is_null()).or(a.first())) {
        // the first fix, with the other fix-its that belong to it (same alternative); the edits say what the message would say
        let alt = f["alternative"].clone();
        let edits: Vec<Value> = v["fixits"].as_array().unwrap().iter().filter(|x| x["alternative"] == alt).map(|f| {
            let mut e = json!({"file": f["file"], "line": f["start"]["line"], "column": f["start"]["column"], "text": f["text"]});
            if f["end"]["line"] != f["start"]["line"] || f["end"]["column"] != f["start"]["column"] { e["end_line"] = f["end"]["line"].clone(); e["end_column"] = f["end"]["column"].clone(); }
            e
        }).collect();
        o.insert("fix".into(), json!({"edits": edits}));
    } else if let Some(s) = v["suggestions"].as_array().and_then(|a| a.first()) { o.insert("fix".into(), json!({"message": s["message"]})); }
    if let Some(k) = v["sanitizer"]["kind"].as_str() { o.insert("kind".into(), k.into()); }
    let n = v["notes"].as_array().map(|a| a.len()).unwrap_or(0);
    if n > 0 { o.insert("notes".into(), n.into()); }
    if let Some(n) = v["suppressed"].as_u64().filter(|n| *n > 0) { o.insert("suppressed".into(), n.into()); }
    Value::Object(o)
}

fn summary_normal(v: &mut Value, brief: bool) {
    remove(v, &["command"]); // the injected flags and temp paths are in `full`
    if let Some(Value::Object(f)) = v.get_mut("files") {
        if brief { f.shift_remove("compiled"); } else { cap(f, "compiled", 5); }
        if f.get("linked") == Some(&Value::Bool(false)) { f.shift_remove("linked"); }
    }
    if let Some(Value::Object(d)) = v.get_mut("diagnostics") { d.retain(|k, x| k != "records" && x.as_u64() != Some(0)); }
    if let Some(Value::Object(l)) = v.get_mut("linker") { l.retain(|_, x| x.as_u64() != Some(0)); }
    drop_defaults(v, &["sanitizer_reports"]);
}

fn fix_normal(v: &mut Value, brief: bool) {
    remove(v, &["written", "note"]);
    if let Some(Value::Array(fs)) = v.get_mut("files") {
        for f in fs.iter_mut() {
            // the alternatives carry what diff_with_unsafe adds
            remove(f, &["diff_with_unsafe"]);
            drop_defaults(f, &["safe_edits", "unsafe_edits", "conflicts"]);
        }
    }
    // the diagnostic record has the message and its code; each alternative keeps the note that tells which one it is
    if let Some(Value::Array(al)) = v.get_mut("alternatives") {
        for a in al.iter_mut() { remove(a, &["diagnostic", "code"]); if brief { remove(a, &["diff", "file", "edits"]); } }
    }
    // the diffs carry every edit that is in them; the list is only for the edits that are in none (skipped, unreadable files)
    if let Some(Value::Array(es)) = v.get_mut("edits") {
        es.retain(|e| !e["skipped"].is_null());
        for e in es.iter_mut() { remove(e, &["reason", "in_diff", "in_diff_with_unsafe", "diagnostic", "alternative"]); }
    }
    if brief { remove(v, &["edits"]); }
}

fn header_normal(v: &mut Value, brief: bool) {
    // the C0003 diagnostic just before it has the message and the place
    remove(v, &["message", "location"]);
    if let Some(m) = v.as_object_mut() { if cap(m, "searched", 8) { m.insert("more".into(), MORE.into()); } }
    if v["truncated"] == false { remove(v, &["truncated"]); }
    if brief { remove(v, &["searched", "packages", "near_misses", "included_from"]); if let Some(Value::Array(s)) = v.get_mut("suggestions") { s.truncate(1); } }
}

fn effective_normal(v: &mut Value, brief: bool) {
    remove(v, &["cc1_args", "no_input_given"]);
    for (a, b) in [("clang", "gcc_installation"), ("", "resource_dir")] {
        let slot = if a.is_empty() { Some(&mut v[b]) } else { v[a].get_mut(b) };
        if let Some(x) = slot { if let Some(p) = x.as_str() { *x = crate::paths::tidy(p).into(); } }
    }
    for k in ["search", "system", "quote_only", "from_dash_I"] {
        if let Some(Value::Array(a)) = v["include_dirs"].get_mut(k) { for x in a.iter_mut() { if let Some(p) = x.as_str() { *x = crate::paths::tidy(p).into(); } } }
    }
    if let Some(Value::Object(c)) = v.get_mut("clang") { c.shift_remove("installed_dir"); }
    if let Some(Value::Object(d)) = v.get_mut("defines") { d.shift_remove("all"); }
    // the system include directories are part of the search list
    let search: Vec<Value> = v["include_dirs"]["search"].as_array().cloned().unwrap_or_default();
    if let Some(Value::Array(sys)) = v["include_dirs"].get_mut("system") { sys.retain(|d| !search.contains(d)); }
    if let Some(Value::Object(l)) = v.get_mut("linker") { l.shift_remove("library_dirs"); l.shift_remove("libraries"); }
    if let Some(Value::Object(t)) = v.get_mut("target") { if brief { t.shift_remove("features"); t.shift_remove("tune_cpu"); } }
    if brief { remove(v, &["include_dirs", "linker", "resource_dir", "clang", "sysroot", "debug_info"]); }
}

fn whynot_normal(v: &mut Value, brief: bool) {
    if let Some(Value::Array(ev)) = v.get_mut("events") {
        for e in ev.iter_mut() {
            if let Some(Value::Array(rs)) = e.get_mut("reasons") {
                for r in rs.iter_mut() {
                    remove(r, &["pass"]);
                    // "loop not vectorized: call instruction cannot be vectorized": the outcome is already in `status`
                    if let Some(msg) = r["message"].as_str() { if let Some((_, tail)) = msg.split_once(": ") { if msg.starts_with("loop not ") { r["message"] = tail.into(); } } }
                }
            }
        }
    }
    if let Some(m) = v.as_object_mut() {
        let mut more = cap(m, "events", if brief { 0 } else { 12 });
        if brief { m.shift_remove("events"); }
        more |= cap(m, "suggestions", if brief { 1 } else { 3 });
        if more { m.insert("more".into(), MORE.into()); }
    }
}

fn time_normal(v: &mut Value, brief: bool) {
    remove(v, &["events", "phases_overlap"]);
    if brief {
        if let Some(Value::Object(p)) = v.get_mut("phases") { p.retain(|k, _| k == "frontend_ms" || k == "backend_ms"); }
        if let Some(Value::Object(s)) = v.get_mut("slowest") { for (_, x) in s.iter_mut() { if let Value::Array(a) = x { a.truncate(1); } } }
        remove(v, &["suggestions"]);
    }
}
