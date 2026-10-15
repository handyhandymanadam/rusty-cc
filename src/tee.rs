//! RUSTY_AGENT_OUT=<path> and RUSTY_AGENT_FD=<n>: a second copy of every JSON record, appended to a file or to an already-open descriptor, one object per line, one write per record.
//! Only when agent mode is on. The file is created if missing but its folder is not; an empty path is refused. A bad channel is a warning record, never a failure.
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};

/// the files that get a copy: the RUSTY_AGENT_FD descriptor (re-opened through /proc, so no unsafe) and the RUSTY_AGENT_OUT file
static SINKS: OnceLock<Mutex<Vec<(bool, File)>>> = OnceLock::new();

/// opens the records channels; returns one (code, text) per channel that is not possible (the caller writes each as a diagnostic record, C0500 for the file, C0501 for the descriptor).
/// A bad channel never fails the job; the other channel and standard error still get the records.
pub fn init(agent_on: bool) -> Vec<(&'static str, String)> {
    let mut problems = Vec::new();
    if !agent_on { return problems; }
    let mut files = Vec::new();
    // the descriptor first: a file opened below could otherwise take the number a caller means by a closed descriptor
    if let Some(v) = std::env::var_os("RUSTY_AGENT_FD") {
        match open_fd(&v.to_string_lossy()) {
            Ok(f) => files.push((true, f)),
            Err(why) => problems.push(("C0501", format!("RUSTY_AGENT_FD={}: {why}: no records on that descriptor", v.to_string_lossy()))),
        }
    }
    if let Some(p) = std::env::var_os("RUSTY_AGENT_OUT") {
        if p.is_empty() { problems.push(("C0500", "RUSTY_AGENT_OUT is empty: no records file, records go to standard error only".into())); }
        else {
            match OpenOptions::new().create(true).append(true).open(&p) {
                Ok(f) => files.push((false, f)),
                Err(e) => problems.push(("C0500", format!("RUSTY_AGENT_OUT={} cannot be opened ({}): no records file, records go to standard error only", p.to_string_lossy(), err_text(&e)))),
            }
        }
    }
    if !files.is_empty() { let _ = SINKS.set(Mutex::new(files)); }
    problems
}

/// a descriptor the caller left open, as a new append handle on the same file, pipe or terminal (the caller's descriptor is not touched)
fn open_fd(value: &str) -> Result<File, String> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) { return Err("not a decimal number".into()); }
    let n: u64 = value.parse().map_err(|_| "number too large".to_string())?;
    if n < 3 { return Err(format!("{n} is below 3 (0, 1 and 2 are standard input, output and error)")); }
    // Linux's fdinfo says how the descriptor was opened: a read-only one is refused
    match std::fs::read_to_string(format!("/proc/self/fdinfo/{n}")) {
        Ok(t) => {
            let flags = t.lines().find_map(|l| l.strip_prefix("flags:")).and_then(|v| u32::from_str_radix(v.trim(), 8).ok());
            if flags.is_some_and(|f| f & 3 == 0) { return Err(format!("descriptor {n} is open for reading only")); }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(format!("descriptor {n} is not open")),
        Err(_) => {}
    }
    OpenOptions::new().append(true).open(format!("/proc/self/fd/{n}")).map_err(|e| format!("descriptor {n} cannot be written ({})", err_text(&e)))
}

/// the error without Rust's " (os error N)" suffix
fn err_text(e: &std::io::Error) -> String { let s = e.to_string(); match s.find(" (os error") { Some(i) => s[..i].to_string(), None => s } }

/// a file is open
pub fn active() -> bool { SINKS.get().is_some() }

/// the record that reports a problem with the records file itself
pub fn problem_record(code: &str, msg: &str) -> serde_json::Value {
    serde_json::json!({"schema": 1, "type": "diagnostic", "code": code, "severity": "warning", "message": msg, "flag": null, "notes": [], "suggestions": [], "locations": [], "fixits": []})
}

/// one record (no trailing newline) to each open channel, as one write per record
pub fn line(s: &str) {
    if let Some(m) = SINKS.get() {
        if let Ok(mut fs) = m.lock() { let mut b = Vec::with_capacity(s.len() + 1); b.extend_from_slice(s.as_bytes()); b.push(b'\n'); for (_, f) in fs.iter_mut() { let _ = f.write_all(&b); } }
    }
}

/// opens the channels and writes one warning record per bad one, to standard error and to the channels that work; returns whether any channel is open
pub fn setup(agent_on: bool, init_shape: impl FnOnce(bool)) {
    let problems = init(agent_on);
    init_shape(active());
    for (code, msg) in problems { crate::agent::emit(&mut std::io::stderr(), &problem_record(code, &msg)); }
}

/// a child process (the compiler, a test program) must not write the channels itself: this process copies its records
pub fn scrub(cmd: &mut std::process::Command) -> &mut std::process::Command { cmd.env_remove("RUSTY_AGENT_OUT").env_remove("RUSTY_AGENT_FD") }

/// which open channels hold every record, for the pointer in a summary
pub fn where_all() -> &'static str {
    let Some(m) = SINKS.get() else { return "" };
    let fs = m.lock().unwrap_or_else(|e| e.into_inner());
    match (fs.iter().any(|(fd, _)| !fd), fs.iter().any(|(fd, _)| *fd)) {
        (true, true) => "all records are in the RUSTY_AGENT_OUT file and the RUSTY_AGENT_FD descriptor",
        (false, true) => "all records are in the RUSTY_AGENT_FD descriptor",
        _ => "all records are in the RUSTY_AGENT_OUT file",
    }
}
