//! Feature 7: the flags clang really uses, as one JSON record, from `-###`, `-v` and `-E -dM`.
use crate::agent::shell_words;
use crate::header::probe_args;
use serde_json::{json, Map, Value};
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

fn clang_out(clang: &OsString, arg0: &str, args: &[OsString], extra: &[&str], want_stdout: bool) -> String {
    let o = crate::tee::scrub(Command::new(clang).arg0(arg0).args(args).args(extra).stdin(Stdio::null())).output();
    match o { Ok(o) => String::from_utf8_lossy(if want_stdout { &o.stdout } else { &o.stderr }).into_owned(), Err(_) => String::new() }
}

fn values_after<'a>(words: &'a [String], key: &str) -> Vec<&'a str> {
    words.windows(2).filter(|w| w[0] == key).map(|w| w[1].as_str()).collect()
}

fn first_line_value(text: &str, prefix: &str) -> Option<String> { text.lines().find_map(|l| l.strip_prefix(prefix).map(|r| r.trim().to_string())) }

fn run_version(path: &str) -> Option<String> {
    let o = Command::new(path).arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    String::from_utf8_lossy(&o.stdout).lines().next().map(|s| s.to_string())
}

/// true when the command line names something to compile or link
pub fn has_input(clang: &OsString, arg0: &str, args: &[OsString]) -> bool {
    let t = clang_out(clang, arg0, args, &["-###"], false);
    t.lines().any(|l| l.contains("\"-cc1\"") || l.contains("\"-o\""))
}

pub fn record(clang: &OsString, arg0: &str, args: &[OsString]) -> Value {
    let probe = probe_args(args);
    // with no input file, ask about an empty C file so the query still has something to describe
    let mut probe = probe;
    let no_input = !has_input(clang, arg0, &probe);
    if no_input { probe.extend(["-x".into(), "c".into(), "/dev/null".into()]); }
    let hash = clang_out(clang, arg0, &probe, &["-###"], false);
    let verbose = clang_out(clang, arg0, &probe, &["-E", "-v", "-o", "/dev/null"], false);
    let dm = clang_out(clang, arg0, &probe, &["-E", "-dM", "-o", "-"], true);

    let mut o = Map::new();
    o.insert("schema".into(), json!(1)); o.insert("type".into(), json!("effective-flags"));
    // clang itself
    let version = hash.lines().next().unwrap_or("").to_string();
    let mut clang_obj = json!({"version": version, "path": clang.to_string_lossy(), "installed_dir": first_line_value(&hash, "InstalledDir:"),
        "config_file": first_line_value(&hash, "Configuration file:"), "gcc_installation": first_line_value(&verbose, "Selected GCC installation:")});
    clang_obj["default_target"] = json!(first_line_value(&hash, "Target:"));
    o.insert("clang".into(), clang_obj);
    // jobs from -###
    let mut cc1: Vec<String> = vec![]; let mut link: Vec<String> = vec![];
    for l in hash.lines() {
        let w = shell_words(l);
        if w.iter().any(|x| x == "-cc1") && cc1.is_empty() { cc1 = w; }
        else if w.iter().any(|x| x == "-o") && !w.iter().any(|x| x == "-cc1") && !w.is_empty() && w[0].starts_with('/') { link = w; }
    }
    let triple = values_after(&cc1, "-triple").first().map(|s| s.to_string());
    let cpu = values_after(&cc1, "-target-cpu").first().map(|s| s.to_string());
    let features: Vec<&str> = values_after(&cc1, "-target-feature");
    o.insert("target".into(), json!({"triple": triple, "cpu": cpu, "tune_cpu": values_after(&cc1, "-tune-cpu").first(), "features": features}));
    let sysroot = values_after(&cc1, "-isysroot").first().map(|s| s.to_string());
    o.insert("sysroot".into(), json!(sysroot));
    o.insert("resource_dir".into(), json!(values_after(&cc1, "-resource-dir").first()));
    let lang = values_after(&cc1, "-x").first().map(|s| s.to_string());
    let std_flag = cc1.iter().find_map(|w| w.strip_prefix("-std=")).map(|s| s.to_string());
    let opt = cc1.iter().find(|w| w.starts_with("-O") && w.len() <= 6).cloned();
    o.insert("language".into(), json!({"x": lang, "std": std_flag}));
    o.insert("optimization".into(), json!(opt));
    o.insert("debug_info".into(), json!(cc1.iter().find_map(|w| w.strip_prefix("-debug-info-kind=")).map(|s| s.to_string())));
    o.insert("position_independent".into(), json!(values_after(&cc1, "-mrelocation-model").first().map(|s| *s != "static")));
    // include directories, in search order, from -v
    let (mut quote, mut angle, mut section) = (vec![], vec![], 0);
    for l in verbose.lines() {
        if l.starts_with("#include \"...\"") { section = 1; continue; }
        if l.starts_with("#include <...>") { section = 2; continue; }
        if l.starts_with("End of search list.") { break; }
        if section > 0 && l.starts_with(' ') { let d = l.trim(); if section == 1 { quote.push(d.to_string()); } else { angle.push(d.to_string()); } }
    }
    let user_i: Vec<&str> = values_after(&cc1, "-I");
    o.insert("include_dirs".into(), json!({"quote_only": quote, "search": angle, "from_dash_I": user_i,
        "system": values_after(&cc1, "-internal-isystem").into_iter().chain(values_after(&cc1, "-internal-externc-isystem")).collect::<Vec<_>>()}));
    // defines
    let user: Vec<String> = { let mut v = vec![]; let mut it = args.iter().filter_map(|a| a.to_str()).peekable();
        while let Some(a) = it.next() { if a == "-D" { if let Some(n) = it.next() { v.push(n.to_string()); } } else if let Some(r) = a.strip_prefix("-D") { v.push(r.to_string()); } } v };
    let mut predefined = Map::new();
    for l in dm.lines() { if let Some(r) = l.strip_prefix("#define ") { let (n, v) = r.split_once(' ').unwrap_or((r, "")); predefined.insert(n.to_string(), json!(v)); } }
    o.insert("defines".into(), json!({"user": user, "predefined_count": predefined.len(), "all": predefined}));
    // linker
    let linker = link.first().cloned();
    let resolved = linker.as_ref().and_then(|p| std::fs::canonicalize(p).ok()).map(|p| p.to_string_lossy().into_owned());
    let lk_ver = linker.as_ref().and_then(|p| run_version(p));
    let use_ld = args.iter().rev().filter_map(|a| a.to_str()).find_map(|a| a.strip_prefix("-fuse-ld=")).map(|s| s.to_string());
    o.insert("linker".into(), json!({"path": linker, "resolved": resolved, "version": lk_ver, "fuse_ld": use_ld,
        "library_dirs": link.iter().filter_map(|w| w.strip_prefix("-L")).collect::<Vec<_>>(),
        "libraries": link.iter().filter_map(|w| w.strip_prefix("-l")).collect::<Vec<_>>(),
        "dynamic_linker": values_after(&link, "-dynamic-linker").first(), "sysroot": link.iter().find_map(|w| w.strip_prefix("--sysroot="))}));
    o.insert("cc1_args".into(), json!(cc1));
    o.insert("no_input_given".into(), json!(no_input));
    Value::Object(o)
}
