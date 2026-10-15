//! Feature 10: `rusty-cc run-san -- PROGRAM ARGS`: run a program built with -fsanitize=..., turn the sanitizer
//! reports on its standard error into shared-format diagnostic records (code C0400). Everything else the program
//! writes passes through unchanged; the exit code is the program's.
use serde_json::{json, Map, Value};
use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Default)]
pub struct Frame { pub index: u32, pub function: String, pub file: Option<String>, pub line: u32, pub column: u32, pub module: Option<String>, pub offset: Option<String> }

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub sanitizer: String, pub kind: String, pub headline: String,
    pub access: Option<(String, u64)>, pub address: Option<String>,
    pub region: Option<(u64, String, u64)>,
    pub stacks: Vec<(String, Vec<Frame>)>, // (role, frames)
    pub summary: Option<String>,
    /// UBSan names the place on its first line; its stack (print_stacktrace=1) follows
    pub place: Option<Frame>,
    pub rendered: String,
}

pub enum Item { Report(Report), Text(String) }

fn parse_frame(l: &str) -> Option<Frame> {
    let t = l.trim_start(); let rest = t.strip_prefix('#')?;
    let (idx, rest) = rest.split_once(' ')?; let index: u32 = idx.parse().ok()?;
    let rest = rest.trim_start();
    let rest = rest.split_once(' ').map(|(_, r)| r).unwrap_or(rest); // skip the address
    let rest = rest.strip_prefix("in ").unwrap_or(rest);
    let rest = match rest.rfind(" (BuildId:") { Some(i) => &rest[..i], None => rest };
    let mut f = Frame { index, ..Default::default() };
    if rest.ends_with(')') {
        if let Some(i) = rest.rfind(" (") { f.function = rest[..i].to_string(); let m = &rest[i + 2..rest.len() - 1];
            match m.rsplit_once('+') { Some((md, off)) => { f.module = Some(md.to_string()); f.offset = Some(off.to_string()); } None => f.module = Some(m.to_string()) }
            return Some(f); }
        f.function = rest.to_string(); return Some(f);
    }
    // function file:line:col  (the function name may contain spaces; the place is the last word)
    if let Some((func, place)) = rest.rsplit_once(' ') {
        let mut it = place.rsplitn(3, ':'); let (a, b, c) = (it.next(), it.next(), it.next());
        match (c, b, a) {
            (Some(file), Some(l), Some(col)) if l.parse::<u32>().is_ok() && col.parse::<u32>().is_ok() => { f.function = func.to_string(); f.file = Some(file.to_string()); f.line = l.parse().unwrap(); f.column = col.parse().unwrap(); }
            (None, Some(file), Some(l)) if l.parse::<u32>().is_ok() => { f.function = func.to_string(); f.file = Some(file.to_string()); f.line = l.parse().unwrap(); }
            _ => f.function = rest.to_string(),
        }
    } else { f.function = rest.to_string(); }
    Some(f)
}

fn is_user_frame(f: &Frame) -> bool {
    match &f.file { Some(p) => !(p.starts_with("/usr/") || p.starts_with("/lib") || p.contains("compiler-rt") || p.contains("/lib/clang/")), None => false }
}

/// "heap-use-after-free on address 0x7c.. at pc ..." -> kind and address
fn asan_headline(rest: &str, r: &mut Report) {
    r.kind = if rest.starts_with("attempting double-free") { "double-free".into() } else if rest.starts_with("attempting free on address which was not malloc") { "bad-free".into() } else { rest.split(' ').next().unwrap_or("").to_string() };
    if let Some(i) = rest.find(" on address ") { r.address = rest[i + 12..].split(' ').next().map(|s| s.to_string()); }
}

pub fn parse(text: &str) -> Vec<Item> {
    let mut out = vec![]; let mut cur: Option<Report> = None; let mut after_summary = false; let mut stack_role = String::new(); let mut sep: Option<String> = None;
    let flush = |cur: &mut Option<Report>, out: &mut Vec<Item>| { if let Some(r) = cur.take() { out.push(Item::Report(r)); } };
    for raw in text.split_inclusive('\n') {
        let l = raw.trim_end_matches('\n');
        // the ===== line that precedes an AddressSanitizer report belongs to it
        if l == "=================================================================" {
            if let Some(sp) = sep.take() { out.push(Item::Text(sp)); }
            flush(&mut cur, &mut out); after_summary = false;
            // the empty line LeakSanitizer prints before it is part of the report too
            let blank = matches!(out.last(), Some(Item::Text(t)) if t == "\n") && { out.pop(); true };
            sep = Some(if blank { format!("\n{raw}") } else { raw.to_string() }); continue;
        }
        if let Some(r) = cur.as_mut() {
            // still inside a report?
            let is_frame = l.trim_start().starts_with('#') && parse_frame(l).is_some();
            let noise = after_summary && (l.is_empty() || l.starts_with(' ') || l.starts_with("=>") || l.starts_with("Shadow") || l.starts_with("  "));
            let aborting = l.starts_with("==") && l.contains("ABORTING");
            if l.starts_with("SUMMARY: ") { r.summary = Some(l["SUMMARY: ".len()..].to_string()); r.rendered.push_str(raw); after_summary = true; continue; }
            if aborting { r.rendered.push_str(raw); flush(&mut cur, &mut out); after_summary = false; continue; }
            if after_summary && !noise { flush(&mut cur, &mut out); after_summary = false; }
            else if after_summary { r.rendered.push_str(raw); continue; }
            else if is_frame { if let Some(fr) = parse_frame(l) { if r.stacks.is_empty() { r.stacks.push(("access".into(), vec![])); } if let Some(s) = r.stacks.last_mut() { s.1.push(fr); } } r.rendered.push_str(raw); continue; }
            else if l.is_empty() {
                // UBSan without SUMMARY: a blank line ends nothing by itself; ASan has blanks between sections
                r.rendered.push_str(raw); continue;
            } else if cur.as_ref().is_some() && !l.starts_with("==") {
                let r = cur.as_mut().unwrap();
                if let Some(rest) = l.strip_prefix("READ of size ").or_else(|| l.strip_prefix("WRITE of size ")) {
                    let n: u64 = rest.split(' ').next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    r.access = Some((if l.starts_with("READ") { "READ" } else { "WRITE" }.to_string(), n));
                    r.stacks.push(("access".into(), vec![]));
                } else if l.contains(" is located ") && l.contains("-byte region") {
                    // 0x.. is located 0 bytes after 16-byte region [..)
                    if let Some(i) = l.find(" is located ") { let w: Vec<&str> = l[i + 12..].split(' ').collect();
                        if w.len() >= 4 { let off = w[0].parse().unwrap_or(0); let rel = if w[2] == "inside" { "inside".to_string() } else { w[2].to_string() };
                            let size = w.iter().find_map(|x| x.strip_suffix("-byte")).and_then(|x| x.parse().ok()).unwrap_or(0); r.region = Some((off, rel, size)); } }
                } else if l.starts_with("allocated by") || l.starts_with("previously allocated by") { r.stacks.push(("allocated".into(), vec![])); stack_role = "allocated".into(); }
                else if l.starts_with("freed by") || l.starts_with("previously freed by") { r.stacks.push(("freed".into(), vec![])); stack_role = "freed".into(); }
                else if l.starts_with("Direct leak of") || l.starts_with("Indirect leak of") { r.stacks.push((l.trim_end_matches(':').to_string(), vec![])); }
                else if l.starts_with("Previous ") || l.starts_with("Location is") || l.starts_with("Mutex ") { r.stacks.push((l.trim_end_matches(':').to_string(), vec![])); }
                let _ = &stack_role;
                r.rendered.push_str(raw); continue;
            } else if cur.is_some() { let r = cur.as_mut().unwrap(); r.rendered.push_str(raw); continue; }
        }
        // a new report?
        if let Some(rest) = l.strip_prefix("==").and_then(|s| s.split_once("==ERROR: ")).map(|(_, r)| r) {
            flush(&mut cur, &mut out); after_summary = false;
            let (name, rest2) = rest.split_once(": ").unwrap_or((rest, ""));
            let mut r = Report { sanitizer: name.to_string(), headline: rest.to_string(), ..Default::default() };
            if name == "LeakSanitizer" { r.kind = "memory-leak".into(); } else { asan_headline(rest2, &mut r); }
            if let Some(sp) = sep.take() { r.rendered.push_str(&sp); }
            r.rendered.push_str(raw); cur = Some(r); continue;
        }
        if let Some(rest) = l.strip_prefix("WARNING: ThreadSanitizer: ") {
            flush(&mut cur, &mut out); after_summary = false;
            let kind = rest.split(" (pid=").next().unwrap_or(rest);
            cur = Some(Report { sanitizer: "ThreadSanitizer".into(), kind: kind.replace(' ', "-"), headline: rest.to_string(), rendered: raw.to_string(), ..Default::default() }); continue;
        }
        if let Some((place, msg)) = l.split_once(": runtime error: ") {
            flush(&mut cur, &mut out); after_summary = false;
            let mut r = Report { sanitizer: "UndefinedBehaviorSanitizer".into(), headline: msg.to_string(), rendered: raw.to_string(), ..Default::default() };
            r.kind = msg.split(':').next().unwrap_or(msg).replace(' ', "-");
            let mut it = place.rsplitn(3, ':'); let (c, ln, f) = (it.next(), it.next(), it.next());
            if let (Some(f), Some(ln), Some(c)) = (f, ln, c) { if let (Ok(l2), Ok(c2)) = (ln.parse::<u32>(), c.parse::<u32>()) { r.place = Some(Frame { index: 0, function: String::new(), file: Some(f.to_string()), line: l2, column: c2, ..Default::default() }); } }
            r.stacks.push(("access".into(), vec![]));
            cur = Some(r); continue;
        }
        flush(&mut cur, &mut out);
        if let Some(sp) = sep.take() { out.push(Item::Text(sp)); }
        out.push(Item::Text(raw.to_string()));
    }
    if let Some(sp) = sep.take() { out.push(Item::Text(sp)); }
    flush(&mut cur, &mut out);
    out
}

fn hint(r: &Report) -> Option<&'static str> {
    Some(match r.kind.as_str() {
        "heap-buffer-overflow" | "stack-buffer-overflow" | "global-buffer-overflow" => "an access went past the end (or before the start) of the object: check the index or loop bound against the region size in the notes",
        "heap-use-after-free" => "the memory was used after free(); the `freed` and `allocated` frames show where; keep the owner alive longer or clear the pointer when freeing",
        "stack-use-after-return" | "stack-use-after-scope" => "a pointer to a local variable outlived the variable; return a copy or allocate it on the heap",
        "attempting-double-free" | "double-free" => "the same pointer was freed twice; free once, then set it to NULL",
        "memory-leak" => "memory from the `leak` frames was never freed; free it on every exit path, or keep a pointer to it",
        "data-race" => "two threads touched the same memory without a lock; protect it with a mutex or use an atomic",
        "signed-integer-overflow" => "signed overflow is undefined behaviour; use a wider or unsigned type, or check before the operation",
        "load-of-null-pointer" | "store-to-null-pointer" | "member-access-within-null-pointer" => "a null pointer was dereferenced; check it before use",
        "division-by-zero" => "divisor was zero; check it before dividing",
        "shift-exponent" => "the shift count was negative or too large for the type",
        _ => {
            if r.kind.starts_with("index") || r.headline.contains("out of bounds") { "an array index was out of bounds; check it against the array size" } else { return None }
        }
    })
}

fn frame_json(f: &Frame) -> Value { json!({"index": f.index, "function": f.function, "file": f.file, "line": f.line, "column": f.column, "module": f.module, "offset": f.offset}) }

/// the sanitizer prints the path the program was compiled with (often absolute, sometimes with `..`); records name files the way the
/// compiler records do: relative to the current directory when they lie under it
fn relative(f: &Frame) -> Frame {
    let cwd = std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned());
    Frame { file: f.file.as_ref().map(|p| crate::paths::short(p, cwd.as_deref())), ..f.clone() }
}

pub fn record(r: &Report) -> Value {
    let r2 = Report { place: r.place.as_ref().map(relative), stacks: r.stacks.iter().map(|(role, fr)| (role.clone(), fr.iter().map(relative).collect())).collect(),
                      sanitizer: r.sanitizer.clone(), kind: r.kind.clone(), headline: r.headline.clone(), access: r.access.clone(), address: r.address.clone(),
                      region: r.region.clone(), summary: r.summary.clone(), rendered: r.rendered.clone() };
    record_paths(&r2)
}

fn record_paths(r: &Report) -> Value {
    let mut locations = vec![]; let mut files: Vec<String> = vec![]; let mut notes: Vec<String> = vec![]; let mut symbols: Vec<String> = vec![];
    let fmt = |f: &Frame| match &f.file { Some(p) => format!("{} ({}:{}:{})", f.function, p, f.line, f.column), None => f.function.clone() };
    if let Some(f) = &r.place {
        locations.push(json!({"file": f.file, "line": f.line, "column": f.column, "end_line": f.line, "end_column": f.column, "role": "primary"}));
        if let Some(p) = &f.file { files.push(p.clone()); }
    }
    for (role, frames) in &r.stacks {
        let user: Vec<&Frame> = frames.iter().filter(|f| is_user_frame(f)).collect();
        let r_name = match role.as_str() { "access" => "primary", "allocated" => "allocated", "freed" => "freed", _ => "frame" };
        if let Some(f) = user.first().filter(|_| !(r.place.is_some() && role == "access")) {
            locations.push(json!({"file": f.file, "line": f.line, "column": f.column, "end_line": f.line, "end_column": f.column, "role": r_name}));
            if let Some(p) = &f.file { if !files.contains(p) { files.push(p.clone()); } }
            if role == "access" && !f.function.is_empty() { symbols.push(f.function.clone()); }
        }
        if let (Some(f), true) = (user.first(), r.place.is_some() && role == "access") { if !f.function.is_empty() { symbols.push(f.function.clone()); } }
        // the first frame of the access stack is the primary location already
        let shown: Vec<&&Frame> = user.iter().skip(if role == "access" { 1 } else { 0 }).collect();
        if !shown.is_empty() { notes.push(format!("{}: {}", match role.as_str() { "allocated" => "allocated by", "freed" => "freed by", "access" => "called from", o => o }, shown.iter().map(|f| fmt(f)).collect::<Vec<_>>().join(" <- "))); }
    }
    if let Some((off, rel, size)) = &r.region { notes.insert(0, format!("the address is {off} bytes {rel} a {size}-byte region")); }
    let mut msg = format!("{}: {}", r.sanitizer, r.kind.replace('-', " "));
    if let Some((a, n)) = &r.access { msg.push_str(&format!(": {a} of size {n}")); if let Some(ad) = &r.address { msg.push_str(&format!(" at {ad}")); } }
    if r.sanitizer == "UndefinedBehaviorSanitizer" { msg = format!("{}: {}", r.sanitizer, r.headline); }
    let mut o = Map::new();
    o.insert("schema".into(), json!(1)); o.insert("type".into(), json!("diagnostic")); o.insert("code".into(), json!("C0400")); o.insert("severity".into(), json!("error"));
    o.insert("message".into(), json!(msg)); o.insert("files".into(), json!(files)); o.insert("symbols".into(), json!(symbols)); o.insert("notes".into(), json!(notes));
    o.insert("suggestions".into(), json!(hint(r).map(|h| vec![json!({"message": h, "option": null})]).unwrap_or_default()));
    o.insert("locations".into(), json!(locations)); o.insert("flag".into(), Value::Null); o.insert("fixits".into(), json!([]));
    o.insert("sanitizer".into(), json!({"name": r.sanitizer, "kind": r.kind, "access": r.access.as_ref().map(|a| a.0.clone()), "size": r.access.as_ref().map(|a| a.1),
        "address": r.address, "region": r.region.as_ref().map(|(o, rel, s)| json!({"offset": o, "relation": rel, "size": s})), "summary": r.summary,
        "stacks": r.stacks.iter().map(|(role, fr)| json!({"role": role, "frames": fr.iter().map(frame_json).collect::<Vec<_>>()})).collect::<Vec<_>>()}));
    o.insert("rendered".into(), json!(r.rendered));
    Value::Object(o)
}

fn merge_env(var: &str, ours: &str) -> String { match std::env::var(var) { Ok(v) if !v.is_empty() => format!("{ours}:{v}"), _ => ours.to_string() } }

/// `args` is everything after "run-san"
pub fn run(args: &[std::ffi::OsString]) -> i32 {
    let prog_args: &[std::ffi::OsString] = if args.first().is_some_and(|a| a == "--") { &args[1..] } else { args };
    let Some(prog) = prog_args.first() else { eprintln!("rusty-cc run-san: usage: rusty-cc run-san -- PROGRAM [ARGS...]"); return 2; };
    let agent_on = std::env::var_os("RUSTY_AGENT").is_some_and(|v| v == "1");
    let detail = std::env::var("RUSTY_AGENT_DETAIL").ok().and_then(|v| crate::shape::Detail::parse(&v)).unwrap_or_default();
    crate::tee::setup(agent_on, |file| crate::shape::init(detail, false, false, file));
    let started = std::time::Instant::now();
    let mut cmd = Command::new(prog);
    cmd.args(&prog_args[1..]).env("RUSTY_AGENT", "1").stderr(Stdio::piped())
        .env("ASAN_OPTIONS", merge_env("ASAN_OPTIONS", "color=never")).env("UBSAN_OPTIONS", merge_env("UBSAN_OPTIONS", "print_stacktrace=1:color=never"))
        .env("TSAN_OPTIONS", merge_env("TSAN_OPTIONS", "color=never")).env("MSAN_OPTIONS", merge_env("MSAN_OPTIONS", "color=never")).env("LSAN_OPTIONS", merge_env("LSAN_OPTIONS", "color=never"));
    crate::tee::scrub(&mut cmd);
    let mut child = match cmd.spawn() { Ok(c) => c, Err(e) => { eprintln!("rusty-cc run-san: cannot run {}: {e}", prog.to_string_lossy()); return 127; } };
    let mut buf = Vec::new(); let _ = child.stderr.take().unwrap().read_to_end(&mut buf);
    let status = child.wait().expect("wait");
    let code = status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
    let text = String::from_utf8_lossy(&buf);
    let mut err = std::io::stderr().lock(); let mut n = 0;
    for it in parse(&text) { match it { Item::Report(r) => { n += 1; crate::agent::emit(&mut err, &record(&r)); } Item::Text(t) => { let _ = err.write_all(t.as_bytes()); } } }
    let cmdline: Vec<String> = prog_args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
    crate::agent::emit(&mut err, &json!({"schema": 1, "type": "summary", "tool": "rusty-cc", "mode": "run-san", "exit": code, "elapsed_ms": started.elapsed().as_millis() as u64, "command": cmdline, "sanitizer_reports": n}));
    code
}
