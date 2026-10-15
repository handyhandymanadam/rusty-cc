//! Parser for clang's text diagnostics (clang format, no colour, with -fdiagnostics-parseable-fixits and
//! -fdiagnostics-print-source-range-info). SARIF was tried first and rejected.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sev { Fatal, Error, Warning, Note, Remark }

impl Sev {
    pub fn word(self) -> &'static str {
        match self { Sev::Fatal => "fatal", Sev::Error => "error", Sev::Warning => "warning", Sev::Note => "note", Sev::Remark => "remark" }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Loc { pub file: String, pub line: u32, pub col: u32, pub end_line: u32, pub end_col: u32 }

#[derive(Clone, Debug)]
pub struct Fixit { pub file: String, pub sl: u32, pub sc: u32, pub el: u32, pub ec: u32, pub text: String }

#[derive(Clone, Debug)]
pub struct Block {
    pub sev: Sev,
    pub loc: Option<Loc>,
    pub message: String,
    pub flag: Option<String>,
    pub werror: bool,
    /// text exactly as clang printed it, without our added range annotation and fix-it lines
    pub rendered: String,
    pub fixits: Vec<Fixit>,
}

/// One main diagnostic with the notes that follow it, plus the "In file included from" lines before it.
#[derive(Clone, Debug)]
pub struct Group { pub includes: Vec<String>, pub main: Block, pub notes: Vec<Block> }

/// What the parser hands over, in input order.
#[derive(Debug)]
pub enum Item {
    Group(Group),
    /// "N errors generated." style line: (warnings, errors)
    Counts(u32, u32, String),
    /// anything else (linker JSON, -v output, unknown text), unchanged, with its newline if it had one
    Text(String),
}

fn parse_u32(s: &str) -> Option<u32> { if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) { None } else { s.parse().ok() } }

/// "L:C" -> (L, C)
fn lc(s: &str) -> Option<(u32, u32)> { let (l, c) = s.split_once(':')?; Some((parse_u32(l)?, parse_u32(c)?)) }

/// "{L:C-L:C}" ranges that follow the column, returns (text before them, ranges)
fn split_ranges(prefix: &str) -> (&str, Vec<(u32, u32, u32, u32)>) {
    let Some(i) = prefix.find(":{") else { return (prefix, vec![]) };
    let mut out = vec![];
    for part in prefix[i + 1..].split("}{") {
        let part = part.trim_start_matches('{').trim_end_matches('}');
        if let Some((a, b)) = part.split_once('-') {
            if let (Some((l1, c1)), Some((l2, c2))) = (lc(a), lc(b)) { out.push((l1, c1, l2, c2)); }
        }
    }
    (&prefix[..i], out)
}

fn parse_location(prefix: &str) -> Option<Option<Loc>> {
    let (head, ranges) = split_ranges(prefix);
    let mut loc = Loc::default();
    // file:L:C, file:L, or a special place like <command line>
    let parts: Vec<&str> = head.rsplitn(3, ':').collect();
    let located = if parts.len() == 3 && parse_u32(parts[0]).is_some() && parse_u32(parts[1]).is_some() {
        loc.file = parts[2].to_string(); loc.line = parse_u32(parts[1])?; loc.col = parse_u32(parts[0])?; true
    } else if parts.len() >= 2 && parse_u32(parts[0]).is_some() {
        loc.file = head[..head.len() - parts[0].len() - 1].to_string(); loc.line = parse_u32(parts[0])?; true
    } else { false };
    if !located { return if head.starts_with('<') && head.ends_with('>') { Some(None) } else { None }; }
    loc.end_line = loc.line; loc.end_col = loc.col;
    // the first range that contains the caret is the primary range
    for (l1, c1, l2, c2) in ranges {
        if (l1, c1) <= (loc.line, loc.col) && (loc.line, loc.col) <= (l2, c2) { loc.end_line = l2; loc.end_col = c2; break; }
    }
    Some(Some(loc))
}

const MARKERS: [(&str, Sev); 5] = [(": fatal error: ", Sev::Fatal), (": error: ", Sev::Error), (": warning: ", Sev::Warning), (": note: ", Sev::Note), (": remark: ", Sev::Remark)];

/// Split a trailing " [-Wflag]" or " [-Werror,-Wflag]"
fn split_flag(msg: &str) -> (String, Option<String>, bool) {
    if let Some(stripped) = msg.strip_suffix(']') {
        if let Some(i) = stripped.rfind(" [-") {
            let inner = &stripped[i + 2..];
            if inner.bytes().all(|b| b.is_ascii_alphanumeric() || b"-,_=+".contains(&b)) {
                let mut werror = false; let mut flag = None;
                for f in inner.split(',') { if f == "-Werror" { werror = true } else if flag.is_none() { flag = Some(f.to_string()) } }
                if flag.is_some() || werror { return (stripped[..i].to_string(), flag, werror); }
            }
        }
    }
    (msg.to_string(), None, false)
}

/// A header line "file:L:C:{ranges}: error: message [-Wflag]" or "clang: error: message".
/// Returns the block with `rendered` holding the header as clang printed it (ranges removed).
pub fn parse_header(line: &str) -> Option<Block> {
    // driver diagnostics have no file: "clang: error: ...", "clang++: warning: ..."
    if line.starts_with("clang") {
        if let Some(colon) = line.find(": ") {
            let name = &line[..colon];
            if name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-+._".contains(&b)) {
                for (m, sev) in MARKERS {
                    if let Some(rest) = line[colon..].strip_prefix(m) {
                        let (message, flag, werror) = split_flag(rest);
                        return Some(Block { sev, loc: None, message, flag, werror, rendered: format!("{line}\n"), fixits: vec![] });
                    }
                }
            }
        }
    }
    let mut best: Option<(usize, Sev)> = None;
    for (m, sev) in MARKERS { if let Some(p) = line.find(m) { if best.map_or(true, |(bp, _)| p < bp) { best = Some((p, sev)); } } }
    let (p, sev) = best?;
    let marker_len = MARKERS.iter().find(|(_, s)| *s == sev).unwrap().0.len();
    let prefix = &line[..p];
    let loc = parse_location(prefix)?;
    let (message, flag, werror) = split_flag(&line[p + marker_len..]);
    let (head, _) = split_ranges(prefix);
    let rendered = format!("{head}{}{}\n", &line[p..p + marker_len], &line[p + marker_len..]);
    Some(Block { sev, loc, message, flag, werror, rendered, fixits: vec![] })
}

/// fix-it:"file":{4:13-4:13}:";"
pub fn parse_fixit(line: &str) -> Option<Fixit> {
    let rest = line.strip_prefix("fix-it:\"")?;
    // the file name ends at the first "\":{" (names with that sequence are not supported)
    let i = rest.find("\":{")?;
    let file = rest[..i].to_string();
    let rest = &rest[i + 3..];
    let j = rest.find("}:\"")?;
    let (a, b) = rest[..j].split_once('-')?;
    let ((sl, sc), (el, ec)) = (lc(a)?, lc(b)?);
    let body = rest[j + 3..].strip_suffix('"')?;
    let bytes = body.as_bytes(); let mut out = Vec::new(); let mut k = 0;
    while k < bytes.len() {
        if bytes[k] == b'\\' && k + 1 < bytes.len() {
            if bytes.len() >= k + 4 && bytes[k + 1..k + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
                let v = u8::from_str_radix(&body[k + 1..k + 4], 8).ok()?; out.push(v); k += 4; continue;
            }
            out.push(bytes[k + 1]); k += 2; continue;
        }
        out.push(bytes[k]); k += 1;
    }
    Some(Fixit { file, sl, sc, el, ec, text: String::from_utf8_lossy(&out).into_owned() })
}

fn is_include_line(l: &str) -> bool { l.starts_with("In file included from ") || l.starts_with("In module ") || (l.starts_with(' ') && l.trim_start().starts_with("from ") && l.ends_with(':')) || l.starts_with("In file included from") }

/// source snippet and caret lines: "    3 |  code", "      |  ^~~", "      |  ;"
fn is_snippet(l: &str) -> bool {
    let t = l.trim_start();
    if let Some(i) = t.find('|') { let before = t[..i].trim(); return (before.is_empty() || before.bytes().all(|b| b.is_ascii_digit())) && (i == 0 || t.as_bytes()[i - 1] == b' ') && l.starts_with(' '); }
    false
}

/// "3 errors generated.", "1 warning generated.", "2 warnings and 1 error generated."
fn parse_counts(l: &str) -> Option<(u32, u32)> {
    let rest = l.strip_suffix(" generated.")?;
    let (mut w, mut e) = (0, 0);
    for part in rest.split(" and ") {
        let (n, what) = part.split_once(' ')?;
        let n = parse_u32(n)?;
        match what { "warning" | "warnings" => w = n, "error" | "errors" => e = n, _ => return None }
    }
    Some((w, e))
}

pub struct Parser { group: Option<Group>, pending_includes: Vec<String>, pending_text: String, out: Vec<Item> }

impl Parser {
    pub fn new() -> Self { Parser { group: None, pending_includes: vec![], pending_text: String::new(), out: vec![] } }

    fn flush(&mut self) { if let Some(g) = self.group.take() { self.out.push(Item::Group(g)); } }

    fn flush_pending_text(&mut self) {
        if !self.pending_text.is_empty() { let t = std::mem::take(&mut self.pending_text); self.pending_includes.clear(); self.out.push(Item::Text(t)); }
    }

    fn last_block(&mut self) -> Option<&mut Block> { let g = self.group.as_mut()?; Some(g.notes.last_mut().unwrap_or(&mut g.main)) }

    pub fn line(&mut self, raw: &str) {
        let l = raw.strip_suffix('\n').unwrap_or(raw);
        let l = l.strip_suffix('\r').unwrap_or(l);
        if is_include_line(l) {
            // belongs to the next header (a main diagnostic or a note in another file)
            self.pending_includes.push(l.to_string()); self.pending_text.push_str(raw); return;
        }
        if let Some(b) = parse_header(l) {
            if b.sev != Sev::Note { self.flush(); }
            let includes = std::mem::take(&mut self.pending_includes); self.pending_text.clear();
            let mut b = b;
            if !includes.is_empty() { let mut r = includes.join("\n"); r.push('\n'); r.push_str(&b.rendered); b.rendered = r; }
            if b.sev == Sev::Note {
                if let Some(g) = self.group.as_mut() { g.notes.push(b); return; }
                // a note with nothing before it: keep as its own group so it is not lost
            }
            self.group = Some(Group { includes, main: b, notes: vec![] });
            return;
        }
        if l.starts_with("fix-it:") {
            if let Some(f) = parse_fixit(l) { if let Some(b) = self.last_block() { b.fixits.push(f); return; } }
        }
        if let Some((w, e)) = parse_counts(l) { self.flush(); self.flush_pending_text(); self.out.push(Item::Counts(w, e, raw.to_string())); return; }
        if self.group.is_some() && is_snippet(l) {
            // -fdiagnostics-print-source-range-info makes clang print one extra space after the gutter bar; undo it
            let fixed = match l.find('|') { Some(i) if l.as_bytes().get(i + 2) == Some(&b' ') => format!("{}{}", &l[..i + 2], &l[i + 3..]), _ => l.to_string() };
            let raw = &format!("{fixed}\n");
            let b = self.last_block().unwrap(); b.rendered.push_str(raw); if !raw.ends_with('\n') { b.rendered.push('\n'); } return; }
        self.flush(); self.flush_pending_text();
        self.out.push(Item::Text(raw.to_string()));
    }

    pub fn finish(mut self) -> Vec<Item> { self.flush(); self.flush_pending_text(); self.out }
}

/// Parse a whole stderr capture.
pub fn parse(text: &str) -> Vec<Item> {
    let mut p = Parser::new();
    for l in text.split_inclusive('\n') { p.line(l); }
    p.finish()
}
