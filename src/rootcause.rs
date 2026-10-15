//! Feature 2: group cascading follow-on errors under the first cause, compress long note chains.
//! Nothing is dropped silently: followers are listed in the root record's `follow_on` with the rule that grouped them.
use crate::codes::{classify, Code};
use crate::diag::{Group, Sev};

pub struct Rec {
    pub group: Group,
    pub code: Code,
    pub followers: Vec<(Rec, &'static str)>,
}

pub fn make_rec(group: Group) -> Rec {
    let m = &group.main;
    let driver = m.loc.is_none() && m.rendered.starts_with("clang");
    let code = classify(m.sev, m.flag.as_deref(), &m.message, driver);
    Rec { group, code, followers: vec![] }
}

fn is_error(r: &Rec) -> bool { matches!(r.group.main.sev, Sev::Error | Sev::Fatal) }

fn quoted(msg: &str) -> Vec<&str> {
    let mut out = vec![]; let mut rest = msg;
    while let Some(a) = rest.find('\'') { let Some(b) = rest[a + 1..].find('\'') else { break }; out.push(&rest[a + 1..a + 1 + b]); rest = &rest[a + 2 + b..]; }
    out
}

const SAME_SYMBOL_CODES: [&str; 2] = ["C0001", "C0015"];
/// a later syntax error this close below the first one, in the same file, is treated as a follow-on
pub const SYNTAX_WINDOW: u32 = 10;

/// Returns the rule name if `r` is a follow-on of the root at `roots[i]`.
fn follows(root: &Rec, r: &Rec) -> Option<&'static str> {
    let (rm, m) = (&root.group.main, &r.group.main);
    // identical diagnostic at the same place (a header included twice, a template instantiated twice)
    if let (Some(a), Some(b)) = (&rm.loc, &m.loc) {
        if a.file == b.file && a.line == b.line && a.col == b.col && root.code.code == r.code.code && rm.message == m.message { return Some("same-place"); }
    }
    if !is_error(r) || !is_error(root) { return None; }
    // the same missing name reported again
    if root.code.code == r.code.code && SAME_SYMBOL_CODES.contains(&root.code.code) && rm.message == m.message { return Some("same-error-repeated"); }
    // a later error that mentions the name the root said was undeclared / unknown
    if SAME_SYMBOL_CODES.contains(&root.code.code) {
        if let Some(sym) = first_symbol(root) {
            if root.code.code != r.code.code || rm.message != m.message {
                if quoted(&m.message).iter().any(|q| *q == sym || q.starts_with(&format!("{sym}<")) || q.starts_with(&format!("{sym} "))) { return Some("mentions-undeclared-name"); }
            }
        }
    }
    // an unknown type name makes the declaration after it fail on the same line ("Foo f;" -> 'f' undeclared)
    if root.code.code == "C0015" && rm.message.starts_with("unknown type name") && r.code.code == "C0001" {
        if let (Some(a), Some(b)) = (&rm.loc, &m.loc) { if a.file == b.file && a.line == b.line { return Some("same-line-as-unknown-type"); } }
    }
    // a declaration whose type is unknown ("foo_t x = 0;") never declares x: every later use of x is a cascade of that one error
    if root.code.code == "C0015" && r.code.code == "C0001" && rm.message.starts_with("unknown type name") {
        if let (Some(ty), Some(used)) = (first_symbol(root), undeclared_name(&m.message)) {
            let names = declared_with_type(root, &ty);
            if names.iter().any(|n| *n == used) { return Some("declared-with-unknown-type"); }
        }
    }
    // syntax errors close together in one file
    if root.code.code == "C0002" && r.code.code == "C0002" {
        if let (Some(a), Some(b)) = (&rm.loc, &m.loc) {
            if a.file == b.file && b.line >= a.line && b.line - a.line <= SYNTAX_WINDOW { return Some("syntax-error-nearby"); }
        }
    }
    None
}

/// "use of undeclared identifier 'x'" -> x
fn undeclared_name(msg: &str) -> Option<String> { if msg.starts_with("use of undeclared identifier") { quoted(msg).first().map(|s| s.to_string()) } else { None } }

/// the source line clang printed under a diagnostic ("    2 | foo_t x = 0;")
fn source_lines(rendered: &str) -> Vec<&str> {
    rendered.lines().filter_map(|l| { let (a, b) = l.split_once(" | ")?; if a.trim().chars().all(|c| c.is_ascii_digit()) && !a.trim().is_empty() { Some(b) } else { None } }).collect()
}

/// names declared with type `ty` on a source line: the identifiers that follow the type word ("foo_t *p, x" gives p)
fn names_after_type(line: &str, ty: &str) -> Vec<String> {
    let mut out = vec![]; let b = line.as_bytes(); let mut from = 0;
    let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    while let Some(i) = line[from..].find(ty) {
        let start = from + i; let end = start + ty.len(); from = end;
        if start > 0 && is_id(b[start - 1]) || end < b.len() && is_id(b[end]) { continue; }
        let mut j = end;
        while j < b.len() && (b[j] == b' ' || b[j] == b'*' || b[j] == b'&' || b[j] == b'\t') { j += 1; }
        let k = j; while j < b.len() && is_id(b[j]) { j += 1; }
        if j > k && !b[k].is_ascii_digit() { let w = &line[k..j]; if w != "const" && w != "volatile" { out.push(w.to_string()); } }
    }
    out
}

/// names declared with the unknown type, on the root's line and on the lines of the diagnostics already folded under it
fn declared_with_type(root: &Rec, ty: &str) -> Vec<String> {
    let mut names = vec![];
    for r in std::iter::once(root).chain(root.followers.iter().map(|(f, _)| f)) {
        if !std::ptr::eq(r, root) && !quoted(&r.group.main.message).contains(&ty) { continue; }
        for l in source_lines(&r.group.main.rendered) { names.extend(names_after_type(l, ty)); }
    }
    names
}

fn first_symbol(r: &Rec) -> Option<String> { quoted(&r.group.main.message).first().map(|s| s.to_string()) }

/// Fold follow-ons into roots, in input order. `keep` receives the surviving roots only.
pub fn fold(recs: Vec<Rec>) -> Vec<Rec> {
    let mut roots: Vec<Rec> = vec![];
    for r in recs {
        let mut hit = None;
        for (i, root) in roots.iter().enumerate() { if let Some(rule) = follows(root, &r) { hit = Some((i, rule)); break; } }
        match hit { Some((i, rule)) => roots[i].followers.push((r, rule)), None => roots.push(r) }
    }
    roots
}

const FRAME_PREFIXES: [&str; 6] = ["in instantiation of", "while substituting", "while checking", "in instantiation of default argument", "expanded from macro", "during template argument deduction"];

fn is_frame(msg: &str) -> bool { FRAME_PREFIXES.iter().any(|p| msg.starts_with(p)) || msg.ends_with("requested here") }
fn is_candidate(msg: &str) -> bool { msg.starts_with("candidate ") }

/// Which notes to show in notes[]: runs of more than 4 backtrace frames keep the first 2 and last 2,
/// runs of more than 3 overload candidates keep the first 3. Returns (kept indexes, number omitted).
pub fn compress_notes(msgs: &[&str]) -> (Vec<usize>, usize) {
    let mut keep = vec![]; let mut omitted = 0; let n = msgs.len(); let mut i = 0;
    while i < n {
        let kind: Option<fn(&str) -> bool> = if is_frame(msgs[i]) { Some(is_frame) } else if is_candidate(msgs[i]) { Some(is_candidate) } else { None };
        match kind {
            None => { keep.push(i); i += 1; }
            Some(f) => {
                let mut j = i; while j < n && f(msgs[j]) { j += 1; }
                let run = j - i;
                let frames = f == is_frame as fn(&str) -> bool;
                let (head, tail, limit) = if frames { (2, 2, 4) } else { (3, 0, 3) };
                if run > limit { keep.extend(i..i + head); keep.extend(j - tail..j); omitted += run - head - tail; }
                else { keep.extend(i..j); }
                i = j;
            }
        }
    }
    (keep, omitted)
}
