//! Agent mode: run clang with a few extra diagnostic flags, read its stderr, write shared-format records.
use crate::fixpreview::Edits;
use crate::rootcause::{self, Rec};
use crate::config::Config;
use crate::diag::{self, Block, Fixit, Item, Loc, Sev};
use serde_json::{json, Map, Value};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

/// Flags we add so clang's text is easy to parse. They change only how diagnostics are printed.
const EXTRA_FLAGS: [&str; 8] = [
    "-fno-color-diagnostics", "-fdiagnostics-format=clang", "-fdiagnostics-parseable-fixits",
    "-fdiagnostics-print-source-range-info", "-fdiagnostics-show-option", "-fshow-column", "-fshow-source-location", "-fmessage-length=0",
];

fn loc_json(l: &Loc, role: &str) -> Value {
    json!({"file": l.file, "line": l.line, "column": l.col, "end_line": l.end_line, "end_column": l.end_col, "role": role})
}

/// byte offset of (line, column) in the file, columns count bytes from 1 as clang does
fn offset_of(lines: &[usize], line: u32, col: u32) -> Option<usize> {
    let start = *lines.get(line.checked_sub(1)? as usize)?;
    Some(start + col.checked_sub(1)? as usize)
}

fn line_starts(data: &[u8]) -> Vec<usize> {
    let mut v = vec![0]; for (i, b) in data.iter().enumerate() { if *b == b'\n' { v.push(i + 1); } } v
}

fn fixit_json(f: &Fixit, cache: &mut std::collections::HashMap<String, Option<Vec<usize>>>) -> Value {
    let starts = cache.entry(f.file.clone()).or_insert_with(|| std::fs::read(&f.file).ok().map(|d| line_starts(&d)));
    let (so, eo) = match starts { Some(s) => (offset_of(s, f.sl, f.sc), offset_of(s, f.el, f.ec)), None => (None, None) };
    json!({"file": f.file,
           "start": {"line": f.sl, "column": f.sc, "offset": so},
           "end": {"line": f.el, "column": f.ec, "offset": eo},
           "text": f.text})
}

fn first_quoted(msg: &str) -> Option<String> {
    let a = msg.find('\'')?; let b = msg[a + 1..].find('\'')?; Some(msg[a + 1..a + 1 + b].to_string())
}

fn fix_message(f: &Fixit) -> String {
    let at = format!("{}:{}:{}", f.file, f.sl, f.sc);
    if (f.sl, f.sc) == (f.el, f.ec) { format!("insert '{}' at {at}", f.text) }
    else if f.text.is_empty() { format!("remove the text at {at}-{}:{}", f.el, f.ec) }
    else { format!("replace the text at {at}-{}:{} with '{}'", f.el, f.ec, f.text) }
}

pub type Cache = std::collections::HashMap<String, Option<Vec<usize>>>;

pub fn group_record(rec: &Rec, root_cause: bool, cache: &mut Cache) -> Value {
    let g = &rec.group; let code = &rec.code;
    let m = &g.main;
    let mut files: Vec<String> = vec![];
    let mut locations = vec![];
    let push_file = |f: &str, files: &mut Vec<String>| if !f.is_empty() && !files.iter().any(|x| x == f) { files.push(f.to_string()) };
    if let Some(l) = &m.loc { push_file(&l.file, &mut files); locations.push(loc_json(l, "primary")); }
    let mut notes: Vec<String> = vec![];
    let mut note_texts: Vec<String> = vec![];
    let mut note_locs: Vec<Option<Value>> = vec![];
    for n in &g.notes {
        match &n.loc {
            Some(l) => { push_file(&l.file, &mut files); note_locs.push(Some(loc_json(l, "note"))); note_texts.push(format!("{}:{}:{}: {}", l.file, l.line, l.col, n.message)); }
            None => { note_locs.push(None); note_texts.push(n.message.clone()); }
        }
    }
    let mut omitted = 0;
    let mut kept_notes: Vec<usize> = (0..note_texts.len()).collect();
    if root_cause {
        let msgs: Vec<&str> = g.notes.iter().map(|n| n.message.as_str()).collect();
        let (keep, om) = rootcause::compress_notes(&msgs);
        omitted = om;
        let mut last = None;
        for &i in &keep {
            if let Some(l) = last { if i != l + 1 { notes.push(format!("... {} more notes omitted", i - l - 1)); } }
            notes.push(note_texts[i].clone()); last = Some(i);
        }
        if let Some(l) = last { if l + 1 < note_texts.len() { notes.push(format!("... {} more notes omitted", note_texts.len() - l - 1)); } }
        kept_notes = keep;
    } else { notes = note_texts; }
    // the locations of the notes that are shown, in step with them (`notes_omitted` counts the others)
    for (i, nl) in note_locs.into_iter().enumerate() { if let Some(l) = nl { if kept_notes.contains(&i) { locations.push(l); } } }
    let all: Vec<&Block> = std::iter::once(m).chain(g.notes.iter()).collect();
    let mut fixits = vec![]; let mut suggestions = vec![];
    // a fix-it on a note is one alternative: fix-its with the same `alternative` number go together, different numbers exclude each other
    let mut alt = 0;
    for (bi, b) in all.iter().enumerate() {
        if bi > 0 && !b.fixits.is_empty() { alt += 1; }
        for f in &b.fixits {
            let mut j = fixit_json(f, cache);
            if bi > 0 { j["alternative"] = json!(alt); }
            fixits.push(j); suggestions.push(json!({"message": fix_message(f), "option": null}));
        }
    }
    let symbols: Vec<String> = if code.symbol_from_quotes { first_quoted(&m.message).into_iter().collect() } else { vec![] };
    let rendered: String = all.iter().map(|b| b.rendered.as_str()).collect();
    let severity = match m.sev { Sev::Fatal => "fatal", Sev::Error => "error", _ => "warning" };
    let ty = if m.sev == Sev::Remark { "remark" } else { "diagnostic" };
    let mut o = Map::new();
    o.insert("schema".into(), json!(1)); o.insert("type".into(), json!(ty));
    o.insert("code".into(), json!(code.code)); o.insert("severity".into(), json!(severity));
    o.insert("message".into(), json!(m.message)); o.insert("files".into(), json!(files));
    o.insert("symbols".into(), json!(symbols)); o.insert("notes".into(), json!(notes));
    o.insert("suggestions".into(), json!(suggestions)); o.insert("locations".into(), Value::Array(locations));
    o.insert("flag".into(), json!(m.flag)); o.insert("fixits".into(), Value::Array(fixits));
    if m.werror { o.insert("werror".into(), json!(true)); }
    if !g.includes.is_empty() { o.insert("include_stack".into(), json!(g.includes)); }
    if root_cause {
        o.insert("notes_omitted".into(), json!(omitted));
        o.insert("suppressed".into(), json!(rec.followers.len()));
        let fo: Vec<Value> = rec.followers.iter().map(|(f, rule)| {
            let fm = &f.group.main;
            json!({"code": f.code.code, "severity": match fm.sev { Sev::Fatal => "fatal", Sev::Error => "error", _ => "warning" },
                   "message": fm.message, "flag": fm.flag, "location": fm.loc.as_ref().map(|l| loc_json(l, "primary")), "rule": rule})
        }).collect();
        o.insert("follow_on".into(), Value::Array(fo));
    }
    o.insert("rendered".into(), json!(rendered));
    Value::Object(o)
}

/// text on standard output; a reader that went away (`| head`) ends the output quietly instead of a panic
pub fn stdout_text(s: &str) {
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(s.as_bytes());
    let _ = o.flush();
}

/// one record: shaped by the detail level, held to the size budget, copied to the records file
pub fn emit(out: &mut impl Write, v: &Value) { crate::shape::write(out, v); }

/// Run clang with agent flags; returns the exit code to use.
pub fn run(cfg: &Config, clang: &OsString, arg0: &str, args: &[OsString]) -> i32 {
    if cfg.agent && args.iter().any(|a| a == "--help") {
        // clang's help, then ours: in agent mode only (a plain --help stays clang's bytes)
        let mut cmd = Command::new(clang); cmd.arg0(arg0).args(args); crate::tee::scrub(&mut cmd);
        let code = match cmd.status() { Ok(s) => s.code().unwrap_or(1), Err(e) => { eprintln!("rusty-cc: cannot run clang: {e}"); return 127; } };
        stdout_text(&crate::help::help_text());
        return code;
    }
    if cfg.agent && args.iter().any(|a| a == "--version") {
        // clang's own text unchanged, then the capabilities line
        let mut cmd = Command::new(clang); cmd.arg0(arg0).args(args); crate::tee::scrub(&mut cmd);
        let code = match cmd.status() { Ok(s) => s.code().unwrap_or(1), Err(e) => { eprintln!("rusty-cc: cannot run clang: {e}"); return 127; } };
        stdout_text(&format!("agent mode: yes, schema 1\n{}\n", crate::help::capabilities()));
        return code;
    }
    if !cfg.diag_json && cfg.time_report.is_none() && !cfg.effective_flags && !cfg.fix_preview && !cfg.explain_header && !cfg.build_summary && !cfg.stable && cfg.why.is_empty() {
        // agent mode on, but records off: clang's text as it is, linker still told to use agent mode
        let err = std::os::unix::process::CommandExt::exec(Command::new(clang).arg0(arg0).args(args).env("RUSTY_AGENT", "1"));
        eprintln!("rusty-cc: cannot run clang: {err}");
        return 127;
    }
    if cfg.effective_flags && !crate::effective::has_input(clang, arg0, args) {
        // a question about flags only: nothing to compile, so clang's "no input files" is not raised
        let mut err = std::io::stderr().lock();
        emit(&mut err, &crate::effective::record(clang, arg0, args));
        return 0;
    }
    let mut cmd = Command::new(clang);
    cmd.arg0(arg0);
    // our flags go before a bare "--" so clang still reads them as options
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let mut full: Vec<OsString> = args[..split].to_vec();
    full.extend(EXTRA_FLAGS.iter().map(OsString::from));
    // the optimization record goes to a private temp directory, never next to the user's outputs
    let opt_dir = if cfg.why.is_empty() { None } else { Some(std::env::temp_dir().join(format!("rusty-cc-{}", std::process::id()))) };
    if let Some(d) = &opt_dir {
        let _ = std::fs::create_dir_all(d);
        full.push("-fsave-optimization-record".into());
        full.push(format!("-foptimization-record-file={}", d.join("opt.yaml").display()).into());
    }
    let trace_dir = cfg.time_report.map(|_| std::env::temp_dir().join(format!("rusty-cc-trace-{}", std::process::id())));
    if let Some(d) = &trace_dir {
        let _ = std::fs::create_dir_all(d);
        full.push(format!("-ftime-trace={}", d.display()).into());
        if !args.iter().any(|a| a.to_str().is_some_and(|a| a.starts_with("-ftime-trace-granularity="))) { full.push("-ftime-trace-granularity=100".into()); }
    }
    full.extend(args[split..].iter().cloned());
    cmd.args(&full);
    cmd.env("RUSTY_AGENT", "1");
    // the linker's records pass through this process, which copies them to the records file: the child must not write the file too
    crate::tee::scrub(&mut cmd);
    if cfg.detail_flag { cmd.env("RUSTY_AGENT_DETAIL", match cfg.detail { crate::shape::Detail::Brief => "brief", crate::shape::Detail::Normal => "normal", crate::shape::Detail::Full => "full" }); }
    if cfg.stable { cmd.env("LC_ALL", "C").env("LANG", "C").env_remove("LANGUAGE"); }
    cmd.stderr(Stdio::piped());
    let started = std::time::Instant::now();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => { eprintln!("rusty-cc: cannot run clang: {e}"); return 127; }
    };
    let mut buf = Vec::new();
    let _ = child.stderr.take().unwrap().read_to_end(&mut buf);
    let status = child.wait().expect("wait for clang");
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let code = status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
    let text = String::from_utf8_lossy(&buf);
    let mut err = std::io::stderr().lock();
    let mut cache = Cache::new();
    // stage 1: parse, 2: classify, 3: collect fix-its, 4: (opt-in) fold cascades, 5: write
    enum Out { Rec(Rec), Text(String), Boundary }
    let mut outs: Vec<Out> = vec![];
    let mut edits = Edits::new(); let mut diag_no = 0usize;
    for item in diag::parse(&text) {
        match item {
            Item::Group(g) => {
                let rec = rootcause::make_rec(g);
                if cfg.fix_preview {
                    let all = std::iter::once((&rec.group.main, true, None)).chain(rec.group.notes.iter().enumerate().map(|(i, n)| (n, false, Some(i))));
                    for (b, safe, note) in all { for f in &b.fixits { edits.add(f, safe, rec.code.code, &b.message, diag_no, note.map(|i| diag_no * 1000 + i), &rec.group.main.message); } }
                    diag_no += 1;
                }
                outs.push(Out::Rec(rec));
            }
            Item::Counts(_, _, raw) => { if cfg.diag_json { outs.push(Out::Boundary); } else { outs.push(Out::Text(raw)); } }
            Item::Text(t) => outs.push(Out::Text(t)),
        }
    }
    // runs of records between text lines / end of one translation unit
    if cfg.root_cause || cfg.stable {
        let mut done = vec![]; let mut run = vec![];
        let finish = |run: &mut Vec<Rec>, done: &mut Vec<Out>| {
            let mut v = std::mem::take(run);
            if cfg.root_cause { v = rootcause::fold(v); }
            if cfg.stable { v.sort_by(|a, b| rec_key(a).cmp(&rec_key(b))); }
            done.extend(v.into_iter().map(Out::Rec));
        };
        for o in outs {
            match o { Out::Rec(r) => run.push(r), other => { finish(&mut run, &mut done); done.push(other); } }
        }
        finish(&mut run, &mut done);
        outs = done;
    }
    let hctx = crate::header::Ctx { clang, arg0, args };
    let mut stats = Stats::default();
    let mut explained: Vec<String> = vec![]; let mut late: Vec<Value> = vec![];
    for o in &outs {
        match o {
            Out::Rec(r) => { stats.count(r); }
            _ => {}
        }
        match o {
            Out::Rec(r) if cfg.diag_json => { let v = group_record(r, cfg.root_cause, &mut cache); emit(&mut err, &v); }
            // records off: print clang's text again, as it printed it
            Out::Rec(r) => { let _ = err.write_all(std::iter::once(&r.group.main).chain(r.group.notes.iter()).map(|b| b.rendered.as_str()).collect::<String>().as_bytes()); }
            Out::Text(t) => { stats.text_line(t); let t = if cfg.stable { normalize_tmp(t) } else { t.clone() }; let _ = err.write_all(t.as_bytes()); if t.starts_with("{\"schema\"") { crate::tee::line(t.trim_end_matches('\n')); } }
            Out::Boundary => {}
        }
        if let (true, Out::Rec(r)) = (cfg.explain_header, o) {
            if r.code.code == "C0003" {
                if let Some(name) = crate::header::missing_name(&r.group.main.message) {
                    if !explained.contains(&name) {
                        explained.push(name.clone());
                        let m = &r.group.main;
                        let loc = m.loc.as_ref().map(|l| loc_json(l, "primary"));
                        let v = crate::header::explain(&hctx, &name, crate::header::include_form(&m.rendered), m.loc.as_ref().map(|l| l.file.as_str()), loc.as_ref(), &m.message);
                        // JSON mode: right after its diagnostic; text mode: after clang's whole text, so that stays contiguous
                        if cfg.diag_json { emit(&mut err, &v); } else { late.push(v); }
                    }
                }
            }
        }
    }
    for v in &late { emit(&mut err, v); }
    if cfg.fix_preview { if let Some(v) = crate::fixpreview::record(&edits) { emit(&mut err, &v); } }
    if let Some(d) = &opt_dir {
        let yaml = std::fs::read_to_string(d.join("opt.yaml")).ok();
        let _ = std::fs::remove_dir_all(d);
        let level = args.iter().rev().find_map(|a| { let s = a.to_str()?; if s == "-O" { Some("-O1".to_string()) } else if s.len() <= 6 && s.starts_with("-O") && !s.starts_with("-Op") { Some(s.to_string()) } else { None } });
        let n_tu = jobs(clang, arg0, args).0.len();
        let compiled = !args.iter().any(|a| a == "-fsyntax-only" || a == "-E" || a == "-M" || a == "-MM");
        let why_none = if n_tu > 1 { Some("the optimization record is one file per command: ask about one source file at a time") }
            else if !compiled { Some("this command does not generate code (-fsyntax-only, -E or -M), so the optimizer did not run") }
            else if code != 0 && yaml.is_none() { Some("the compile failed before optimization") } else { None };
        let remarks = if why_none.is_some() { None } else { yaml.as_deref().map(crate::whynot::parse_record) };
        for (what, name) in &cfg.why {
            let spans = if *what != crate::whynot::What::Inlined && remarks.is_some() {
                let mut dump_args = crate::header::probe_args(args);
                dump_args.extend(["-fsyntax-only".into(), "-Xclang".into(), "-ast-dump".into(), "-Xclang".into(), format!("-ast-dump-filter={name}").into(), "-fno-color-diagnostics".into()]);
                match crate::tee::scrub(Command::new(clang).arg0(arg0).args(&dump_args).stdin(Stdio::null()).stderr(Stdio::null())).output() {
                    Ok(o) => crate::whynot::spans_from_ast(&String::from_utf8_lossy(&o.stdout), name), Err(_) => vec![] }
            } else { vec![] };
            let v = crate::whynot::record(*what, name, remarks.as_deref(), level.as_deref(), &spans, why_none);
            emit(&mut err, &v);
        }
    }
    if let (Some(d), Some(top)) = (&trace_dir, cfg.time_report) {
        let mut files: Vec<_> = std::fs::read_dir(d).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect()).unwrap_or_default();
        files.sort();
        let sources = jobs(clang, arg0, args).0;
        if files.is_empty() { emit(&mut err, &json!({"schema": 1, "type": "compile-time", "file": null, "note": "clang wrote no trace: nothing was compiled (or the compile failed before the frontend)"})); }
        for f in files {
            let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            // the trace is named after the output (or the input); with one source there is nothing to match
            let input = if sources.len() == 1 { sources[0].clone() } else { sources.iter().find(|a| std::path::Path::new(a).file_stem().is_some_and(|s| stem.starts_with(&*s.to_string_lossy()))).cloned().unwrap_or(stem.clone()) };
            if let Ok(text) = std::fs::read_to_string(&f) { match crate::timetrace::report(&text, &input, top) { Some(v) => emit(&mut err, &v), None => emit(&mut err, &json!({"schema": 1, "type": "compile-time", "file": input, "note": "the trace could not be read"})) } }
        }
        let _ = std::fs::remove_dir_all(d);
    }
    if cfg.effective_flags { emit(&mut err, &crate::effective::record(clang, arg0, args)); }
    if cfg.build_summary {
        let mut command: Vec<String> = vec![clang.to_string_lossy().into_owned()];
        // the private temp directories (optimization record, time trace) are deleted before the record is read: name them, not their path
        let privates: Vec<String> = [&opt_dir, &trace_dir].into_iter().flatten().map(|d| d.display().to_string()).collect();
        command.extend(full.iter().map(|a| { let mut t = a.to_string_lossy().into_owned(); for d in &privates { t = t.replace(d.as_str(), "<private temp dir>"); } t }));
        emit(&mut err, &stats.summary(code, elapsed_ms, command, jobs(clang, arg0, args)));
    }
    code
}

trait Arg0 { fn arg0(&mut self, a: &str) -> &mut Self; }
impl Arg0 for Command { fn arg0(&mut self, a: &str) -> &mut Self { std::os::unix::process::CommandExt::arg0(self, a); self } }

#[derive(Default)]
struct Stats { errors: u32, fatal: u32, warnings: u32, remarks: u32, suppressed: u32, records: u32, link_errors: u32, link_warnings: u32 }

impl Stats {
    fn count(&mut self, r: &Rec) {
        for g in std::iter::once(r).chain(r.followers.iter().map(|(f, _)| f)) {
            match g.group.main.sev { Sev::Fatal => self.fatal += 1, Sev::Error => self.errors += 1, Sev::Remark => self.remarks += 1, _ => self.warnings += 1 }
        }
        self.suppressed += r.followers.len() as u32; self.records += 1;
    }
    /// records of the linker (or any other rusty tool) that pass through are counted too
    fn text_line(&mut self, t: &str) {
        if !t.starts_with("{\"schema\"") { return; }
        if let Ok(v) = serde_json::from_str::<Value>(t) {
            if v["type"] == "diagnostic" { match v["severity"].as_str() { Some("error") | Some("fatal") => self.link_errors += 1, Some("warning") => self.link_warnings += 1, _ => {} } }
        }
    }
    fn summary(&self, exit: i32, elapsed_ms: u64, command: Vec<String>, jobs: (Vec<String>, bool)) -> Value {
        json!({"schema": 1, "type": "summary", "tool": "rusty-cc", "exit": exit, "elapsed_ms": elapsed_ms, "command": command,
               "files": {"compiled": jobs.0, "linked": jobs.1},
               "diagnostics": {"errors": self.errors + self.fatal, "fatal": self.fatal, "warnings": self.warnings, "remarks": self.remarks, "records": self.records, "suppressed": self.suppressed},
               "linker": {"errors": self.link_errors, "warnings": self.link_warnings}})
    }
}

/// What the driver would run, from `clang -###` with the same arguments: source files compiled and whether a link step exists.
fn jobs(clang: &OsString, arg0: &str, args: &[OsString]) -> (Vec<String>, bool) {
    let Ok(o) = crate::tee::scrub(Command::new(clang).arg0(arg0).args(args).arg("-###").stdin(Stdio::null()).stdout(Stdio::null())).output() else { return (vec![], false) };
    let text = String::from_utf8_lossy(&o.stderr);
    let (mut files, mut linked) = (vec![], false);
    for l in text.lines() {
        let words = shell_words(l);
        if words.iter().any(|w| w == "-cc1") {
            if let Some(i) = words.iter().position(|w| w == "-main-file-name") { if let Some(f) = words.get(i + 1) {
                // the real path is the last word that is not an option; main-file-name is the base name
                let path = words.iter().rev().find(|w| !w.starts_with('-') && w.ends_with(f.as_str())).cloned().unwrap_or(f.clone());
                if !files.contains(&path) { files.push(path); }
            } }
        } else if words.first().is_some_and(|w| { let b = w.rsplit('/').next().unwrap_or(w); b == "ld" || b.starts_with("ld.") || b == "mold" || b.contains("ld") && b != "clang" }) && words.iter().any(|w| w == "-o") { linked = true; }
    }
    (files, linked)
}

/// split a `-###` line: words in double quotes with backslash escapes
pub fn shell_words(l: &str) -> Vec<String> {
    let mut out = vec![]; let mut cur = String::new(); let mut inq = false; let mut it = l.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' => { if inq { out.push(std::mem::take(&mut cur)); } inq = !inq; }
            '\\' if inq => { if let Some(n) = it.next() { cur.push(n); } }
            _ if inq => cur.push(c),
            _ => {}
        }
    }
    out
}

/// Sort key for the stable order: located diagnostics by place, then driver diagnostics (no place) last.
fn rec_key(r: &Rec) -> (bool, String, u32, u32, &'static str, String) {
    let m = &r.group.main;
    match &m.loc { Some(l) => (false, l.file.clone(), l.line, l.col, r.code.code, m.message.clone()), None => (true, String::new(), 0, 0, r.code.code, m.message.clone()) }
}

/// clang names temporary files NAME-xxxxxx.EXT with random hex; the stable mode writes XXXXXX instead.
pub fn normalize_tmp(s: &str) -> String {
    const EXT: [&str; 7] = [".o", ".i", ".ii", ".c", ".cpp", ".sh", ".s"];
    let b = s.as_bytes(); let mut out = String::with_capacity(s.len()); let mut i = 0; let mut last = 0;
    while i < b.len() {
        if b[i] == b'-' && i + 7 <= b.len() && b[i + 1..i + 7].iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c)) {
            let rest = &s[i + 7..];
            if EXT.iter().any(|e| rest.starts_with(e) && !rest[e.len()..].starts_with(|c: char| c.is_ascii_alphanumeric())) {
                out.push_str(&s[last..i + 1]); out.push_str("XXXXXX"); last = i + 7; i += 7; continue;
            }
        }
        i += 1;
    }
    out.push_str(&s[last..]); out
}
