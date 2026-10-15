//! Feature 3: apply clang's machine fix-its in memory and show the result as unified diffs. Never writes a file.
//! Safe = a fix-it on an error or warning (what clang's own -fixit applies). Unsafe = a fix-it on a note: clang
//! shows it but -fixit leaves it alone (checked against `clang -Xclang -fixit` while building this).
use crate::diag::Fixit;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// `alt` is `Some(id)` for a fix-it that sits on a note: clang offers it as one alternative way to resolve the
/// diagnostic (all fix-its of one note belong together, different notes of one diagnostic exclude each other).
/// `diag` is the diagnostic they belong to (an index), `message` the message of the block the fix-it sits on.
pub struct Edit { pub fix: Fixit, pub safe: bool, pub code: &'static str, pub message: String, pub diag: usize, pub alt: Option<usize>, pub diag_message: String }

pub struct Edits { pub list: Vec<Edit> }

impl Edits {
    pub fn new() -> Self { Edits { list: vec![] } }
    pub fn add(&mut self, fix: &Fixit, safe: bool, code: &'static str, message: &str, diag: usize, alt: Option<usize>, diag_message: &str) {
        // the same fix-it reported twice (a header seen from two places) counts once
        if self.list.iter().any(|e| e.fix.file == fix.file && (e.fix.sl, e.fix.sc, e.fix.el, e.fix.ec) == (fix.sl, fix.sc, fix.el, fix.ec) && e.fix.text == fix.text) { return; }
        self.list.push(Edit { fix: fix.clone(), safe, code, message: message.to_string(), diag, alt, diag_message: diag_message.to_string() });
    }
}

fn offsets(data: &[u8]) -> Vec<usize> { let mut v = vec![0]; for (i, b) in data.iter().enumerate() { if *b == b'\n' { v.push(i + 1); } } v }
fn off(starts: &[usize], line: u32, col: u32, len: usize) -> Option<usize> {
    let o = *starts.get(line.checked_sub(1)? as usize)? + col.checked_sub(1)? as usize;
    if o <= len { Some(o) } else { None }
}

/// Apply `idx` edits (already resolved to byte ranges) in order; overlapping ones are skipped.
fn apply(data: &[u8], edits: &[(usize, usize, &str, usize)]) -> (Vec<u8>, Vec<usize>) {
    let mut sorted: Vec<&(usize, usize, &str, usize)> = edits.iter().collect();
    sorted.sort_by_key(|e| (e.0, e.1, e.3));
    let mut out = Vec::with_capacity(data.len()); let mut pos = 0; let mut skipped = vec![];
    for e in sorted {
        if e.0 < pos { skipped.push(e.3); continue; }
        out.extend_from_slice(&data[pos..e.0]); out.extend_from_slice(e.2.as_bytes()); pos = e.1;
    }
    out.extend_from_slice(&data[pos..]);
    (out, skipped)
}

fn unified(file: &str, old: &[u8], new: &[u8]) -> Option<String> {
    let (o, n) = (std::str::from_utf8(old).ok()?, std::str::from_utf8(new).ok()?);
    Some(similar::TextDiff::from_lines(o, n).unified_diff().context_radius(3).header(&format!("a/{file}"), &format!("b/{file}")).to_string())
}

pub fn record(edits: &Edits) -> Option<Value> {
    if edits.list.is_empty() { return None; }
    // alternatives: number them in order of appearance; a diagnostic with exactly one is unambiguous (its edits join diff_with_unsafe),
    // a diagnostic with several never has them merged: each gets its own diff
    let mut alt_ids: Vec<usize> = vec![];
    for e in &edits.list { if let Some(a) = e.alt { if !alt_ids.contains(&a) { alt_ids.push(a); } } }
    let alt_no = |a: usize| alt_ids.iter().position(|x| *x == a).map(|p| p + 1).unwrap_or(0);
    let per_diag = |d: usize| { let mut v: Vec<usize> = edits.list.iter().filter(|e| e.diag == d).filter_map(|e| e.alt).collect(); v.sort(); v.dedup(); v.len() };
    let unambiguous = |e: &Edit| e.alt.is_some() && per_diag(e.diag) == 1;
    let mut by_file: BTreeMap<&str, Vec<(usize, &Edit)>> = BTreeMap::new();
    for (i, e) in edits.list.iter().enumerate() { by_file.entry(e.fix.file.as_str()).or_default().push((i, e)); }
    let mut files = vec![]; let mut alternatives: Vec<(usize, Value)> = vec![]; let mut edit_json: Vec<Option<Value>> = vec![None; edits.list.len()];
    for (file, list) in by_file {
        let Ok(data) = std::fs::read(file) else {
            for (i, e) in &list { edit_json[*i] = Some(edit_value(e, None, None, false, false, Some("file could not be read"), e.alt.map(alt_no))); }
            files.push(json!({"file": file, "error": "file could not be read", "safe_edits": 0, "unsafe_edits": 0}));
            continue;
        };
        let starts = offsets(&data);
        let mut resolved: Vec<(usize, usize, &str, usize, bool)> = vec![]; // start, end, text, index, safe
        for (i, e) in &list {
            match (off(&starts, e.fix.sl, e.fix.sc, data.len()), off(&starts, e.fix.el, e.fix.ec, data.len())) {
                (Some(s), Some(t)) if s <= t => resolved.push((s, t, e.fix.text.as_str(), *i, e.safe)),
                _ => edit_json[*i] = Some(edit_value(e, None, None, false, false, Some("position is outside the file"), e.alt.map(alt_no))),
            }
        }
        let safe: Vec<(usize, usize, &str, usize)> = resolved.iter().filter(|r| r.4).map(|r| (r.0, r.1, r.2, r.3)).collect();
        // safe edits plus the alternatives that are the only way to resolve their diagnostic
        let with_one: Vec<(usize, usize, &str, usize)> = resolved.iter().filter(|r| r.4 || unambiguous(&edits.list[r.3])).map(|r| (r.0, r.1, r.2, r.3)).collect();
        let (safe_out, safe_skip) = apply(&data, &safe);
        let (all_out, all_skip) = apply(&data, &with_one);
        let (mut n_safe, mut n_unsafe, mut conflicts) = (0, 0, 0);
        for r in &resolved {
            let e = &edits.list[r.3];
            let in_safe = r.4 && !safe_skip.contains(&r.3);
            let in_all = with_one.iter().any(|x| x.3 == r.3) && !all_skip.contains(&r.3);
            if in_safe { n_safe += 1; }
            if !r.4 { n_unsafe += 1; }
            let skipped = (r.4 && safe_skip.contains(&r.3)) || (!r.4 && with_one.iter().any(|x| x.3 == r.3) && all_skip.contains(&r.3));
            if skipped { conflicts += 1; }
            edit_json[r.3] = Some(edit_value(e, Some(r.0), Some(r.1), in_safe, in_all, if skipped { Some("overlaps another edit") } else { None }, e.alt.map(alt_no)));
        }
        let mut f = json!({"file": file, "safe_edits": n_safe, "unsafe_edits": n_unsafe, "conflicts": conflicts});
        f["diff"] = json!(unified(file, &data, &safe_out));
        if with_one.len() > safe.len() { f["diff_with_unsafe"] = json!(unified(file, &data, &all_out)); }
        files.push(f);
        // each alternative alone, on the original file
        let mut seen: Vec<usize> = vec![];
        for r in resolved.iter().filter(|r| !r.4) {
            let Some(a) = edits.list[r.3].alt else { continue };
            if seen.contains(&a) { continue; } seen.push(a);
            let mine: Vec<(usize, usize, &str, usize)> = resolved.iter().filter(|x| edits.list[x.3].alt == Some(a)).map(|x| (x.0, x.1, x.2, x.3)).collect();
            let (out, _) = apply(&data, &mine);
            let e = &edits.list[r.3];
            alternatives.push((alt_no(a), json!({"id": alt_no(a), "diagnostic": e.diag_message, "code": e.code, "note": e.message, "file": file, "edits": mine.len(), "diff": unified(file, &data, &out)})));
        }
    }
    alternatives.sort_by_key(|a| a.0);
    let mut rec = json!({"schema": 1, "type": "fix-preview", "dry_run": true, "written": false, "files": files});
    rec["alternatives"] = Value::Array(alternatives.into_iter().map(|a| a.1).collect());
    rec["edits"] = Value::Array(edit_json.into_iter().flatten().collect());
    rec["note"] = json!("nothing was written; apply a diff from the project root with `patch -p1`; alternatives exclude each other: apply at most one per diagnostic");
    Some(rec)
}

fn edit_value(e: &Edit, so: Option<usize>, eo: Option<usize>, applied: bool, applied_all: bool, why: Option<&str>, alt: Option<usize>) -> Value {
    json!({"alternative": alt, "file": e.fix.file,
           "start": {"line": e.fix.sl, "column": e.fix.sc, "offset": so},
           "end": {"line": e.fix.el, "column": e.fix.ec, "offset": eo},
           "text": e.fix.text, "safety": if e.safe { "safe" } else { "unsafe" },
           "reason": if e.safe { "fix-it on an error or warning (clang -fixit applies it)" } else { "fix-it on a note (clang shows it, -fixit does not apply it)" },
           "code": e.code, "diagnostic": e.message, "in_diff": applied, "in_diff_with_unsafe": applied_all, "skipped": why})
}
