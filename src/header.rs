//! Feature 4: explain a "file not found" for an #include: where clang looked, where the header exists,
//! the exact -I to add, the pkg-config package, near-miss names. Runs only on demand, after the compile failed.
use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// the whole file search stops after this long; the record says `truncated`
const SEARCH_BUDGET: Duration = Duration::from_millis(1500);

pub struct Ctx<'a> { pub clang: &'a OsString, pub arg0: &'a str, pub args: &'a [OsString] }

const HEADER_EXT: [&str; 9] = ["h", "hh", "hpp", "hxx", "H", "inc", "def", "tcc", "ipp"];
const MAX_NEAR: usize = 3;

fn header_like(p: &Path) -> bool { match p.extension() { Some(e) => HEADER_EXT.iter().any(|x| e == *x), None => true } }

/// All header-like files below `root`, depth limited, hidden directories and build outputs skipped.
fn walk(root: &Path, max_depth: usize, cap: usize, out: &mut Vec<PathBuf>, deadline: Instant, cut: &mut bool) {
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            if out.len() >= cap { return; }
            if Instant::now() >= deadline { *cut = true; return; }
            let Ok(ft) = e.file_type() else { continue };
            let path = e.path();
            if ft.is_dir() {
                let name = e.file_name(); let n = name.to_string_lossy();
                if n.starts_with('.') || n == "target" || n == "node_modules" { continue; }
                if depth + 1 <= max_depth { stack.push((path, depth + 1)); }
            } else if (ft.is_file() || ft.is_symlink()) && header_like(&path) { out.push(path); }
        }
    }
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() { cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + (a[i - 1] != b[j - 1]) as usize); }
        prev = cur;
    }
    prev[b.len()]
}

/// the directories clang searches, from `-E -v` run with the failing command line
/// the command line without options that choose outputs, to ask clang questions about the same inputs
pub fn probe_args(args: &[OsString]) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec![]; let mut skip = false;
    for x in args {
        if skip { skip = false; continue; }
        match x.to_str() {
            Some("-o") | Some("-MF") | Some("-MT") | Some("-MQ") | Some("-MJ") => { skip = true; }
            Some("-c") | Some("-S") | Some("-emit-llvm") | Some("-fsyntax-only") | Some("-MD") | Some("-MMD") | Some("-M") | Some("-MM") => {}
            _ => a.push(x.clone()),
        }
    }
    a
}

pub fn search_dirs(ctx: &Ctx) -> Vec<String> {
    let a = probe_args(ctx.args);
    let out = crate::tee::scrub(Command::new(ctx.clang).arg0(ctx.arg0).args(&a).args(["-E", "-v", "-fno-color-diagnostics"]).stdout(Stdio::null()).stderr(Stdio::piped()).stdin(Stdio::null())).output();
    let Ok(out) = out else { return vec![] };
    let text = String::from_utf8_lossy(&out.stderr);
    let mut dirs = vec![]; let mut inside = false;
    for l in text.lines() {
        if l.contains("search starts here:") { inside = true; continue; }
        if l.starts_with("End of search list.") { break; }
        if inside && l.starts_with(' ') {
            let d = l.trim(); if d.ends_with("(framework directory)") { continue; }
            let d = d.trim_end_matches(" (headermap)");
            let norm = std::fs::canonicalize(d).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| d.to_string());
            if !dirs.contains(&norm) { dirs.push(norm); }
        }
    }
    dirs
}

fn pc_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::env::var("PKG_CONFIG_PATH").unwrap_or_default().split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect();
    for d in ["/usr/lib64/pkgconfig", "/usr/lib/pkgconfig", "/usr/share/pkgconfig", "/usr/local/lib/pkgconfig", "/usr/local/lib64/pkgconfig", "/usr/lib/x86_64-linux-gnu/pkgconfig"] { v.push(PathBuf::from(d)); }
    v
}

/// pkg-config packages whose --cflags-only-I contain -I<prefix>
fn packages_for(prefix: &str) -> Vec<Value> {
    let Some(last) = Path::new(prefix).file_name().map(|s| s.to_string_lossy().into_owned()) else { return vec![] };
    let mut out = vec![];
    for d in pc_dirs() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut names: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "pc")).collect();
        names.sort();
        for pc in names {
            let Ok(text) = std::fs::read_to_string(&pc) else { continue };
            if !text.contains(&last) { continue; }
            let stem = pc.file_stem().unwrap().to_string_lossy().into_owned();
            let Ok(o) = Command::new("pkg-config").args(["--cflags-only-I", &stem]).output() else { continue };
            let flags = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if flags.split_whitespace().any(|f| f.strip_prefix("-I") == Some(prefix)) && !out.iter().any(|v: &Value| v["name"] == stem) {
                out.push(json!({"name": stem, "pc_file": pc.to_string_lossy(), "cflags": flags, "option": format!("-I{prefix}")}));
            }
        }
    }
    out
}

fn rpm_owner(path: &str) -> Option<String> {
    let o = Command::new("rpm").args(["-qf", "--qf", "%{NAME}-%{VERSION}-%{RELEASE}", path]).stderr(Stdio::null()).output().ok()?;
    if o.status.success() { Some(String::from_utf8_lossy(&o.stdout).trim().to_string()) } else { None }
}

/// angle or quote form of the include, from the source line clang printed
pub fn include_form(rendered: &str) -> Option<bool> {
    for l in rendered.lines() {
        if let Some(i) = l.find('|') { let src = l[i + 1..].trim_start(); if let Some(r) = src.strip_prefix('#') {
            let r = r.trim_start(); if let Some(r) = r.strip_prefix("include").or_else(|| r.strip_prefix("include_next")).or_else(|| r.strip_prefix("import")) {
                let r = r.trim_start(); if r.starts_with('<') { return Some(true); } if r.starts_with('"') { return Some(false); } } } }
    }
    None
}

pub fn explain(ctx: &Ctx, name: &str, angle: Option<bool>, from_file: Option<&str>, loc: Option<&Value>, message: &str) -> Value {
    let searched = search_dirs(ctx);
    let cwd = std::env::current_dir().unwrap_or_default();
    let deadline = Instant::now() + SEARCH_BUDGET; let mut cut = false;
    let mut files: Vec<PathBuf> = vec![];
    walk(&cwd, 8, 60_000, &mut files, deadline, &mut cut);
    let n_project = files.len();
    for r in ["/usr/include", "/usr/local/include"] { walk(Path::new(r), 4, 400_000, &mut files, deadline, &mut cut); }
    for d in &searched { if !d.starts_with("/usr/include") && !d.starts_with("/usr/local/include") && !Path::new(d).starts_with(&cwd) { walk(Path::new(d), 4, 100_000, &mut files, deadline, &mut cut); } }
    let suffix = format!("/{name}");
    let mut found: Vec<Value> = vec![]; let mut suggestions: Vec<Value> = vec![];
    let mut packages: Vec<Value> = vec![]; let mut seen_prefix: Vec<String> = vec![];
    let from_dir = from_file.and_then(|f| Path::new(f).parent().map(|p| std::fs::canonicalize(if p.as_os_str().is_empty() { Path::new(".") } else { p }).unwrap_or_else(|_| p.to_path_buf())));
    for (i, f) in files.iter().enumerate() {
        let s = f.to_string_lossy();
        if !s.ends_with(&suffix) { continue; }
        let full = std::fs::canonicalize(f).unwrap_or_else(|_| f.clone());
        let prefix = f.to_string_lossy()[..s.len() - suffix.len()].to_string();
        let prefix_abs = std::fs::canonicalize(&prefix).map(|p| p.to_string_lossy().into_owned()).unwrap_or(prefix.clone());
        if seen_prefix.contains(&prefix_abs) { continue; } seen_prefix.push(prefix_abs.clone());
        let kind = if i < n_project { "project" } else { "system" };
        let mut entry = json!({"path": full.to_string_lossy(), "kind": kind, "add_include_dir": prefix_abs, "option": format!("-I{}", rel_or_abs(&prefix_abs, &cwd))});
        if kind == "system" { if let Some(o) = rpm_owner(&full.to_string_lossy()) { entry["rpm"] = json!(o); } }
        if searched.contains(&prefix_abs) { entry["already_searched"] = json!(true); }
        else {
            let opt = format!("-I{}", rel_or_abs(&prefix_abs, &cwd));
            // a project file is the likelier answer; system copies are listed in `found` but not suggested then
            if kind == "project" || !files[..n_project].iter().any(|p| p.to_string_lossy().ends_with(&suffix)) {
                suggestions.push(json!({"message": format!("add {opt} so that '{name}' resolves to {}", full.display()), "option": opt}));
                if kind == "system" {
                    let ps = packages_for(&prefix_abs);
                    if !ps.is_empty() {
                        let names: Vec<&str> = ps.iter().filter_map(|p| p["name"].as_str()).collect();
                        suggestions.push(json!({"message": format!("pkg-config packages that give {opt}: {} (use `pkg-config --cflags <name>`)", names.join(", ")), "option": opt}));
                    }
                    packages.extend(ps);
                }
            }
        }
        if angle == Some(true) && from_dir.as_ref().is_some_and(|d| d.to_string_lossy() == prefix_abs) {
            suggestions.push(json!({"message": format!("'{name}' is next to the including file: write #include \"{name}\" instead of <{name}>"), "option": null}));
        }
        found.push(entry);
    }
    // near misses: same name apart from case, extension or up to 2 letters
    let want_base = Path::new(name).file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let want_stem = Path::new(&want_base).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut near: Vec<(usize, bool, bool, usize, String, String)> = vec![]; // distance, system, unreachable, depth, path, include_as
    if !want_base.is_empty() {
        for (i, f) in files.iter().enumerate() {
            let Some(base) = f.file_name().map(|s| s.to_string_lossy().to_lowercase()) else { continue };
            if base == want_base && f.to_string_lossy().ends_with(&suffix) { continue; }
            if base.len().abs_diff(want_base.len()) > 2 { continue; }
            let stem = Path::new(&base).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let mut d = edit_distance(&base, &want_base);
            if stem == want_stem { d = d.min(1); }
            let case_only = d == 0 || base == want_base; // same name in other case: the best kind of near miss
            if case_only { d = 0; }
            let limit = if want_base.len() <= 7 { 1 } else { 2 };
            if d > limit { continue; }
            let full = std::fs::canonicalize(f).unwrap_or_else(|_| f.clone()).to_string_lossy().into_owned();
            let reach = searched.iter().filter(|dir| full.starts_with(&format!("{dir}/"))).max_by_key(|d| d.len()).map(|dir| full[dir.len() + 1..].to_string());
            let depth = reach.as_deref().unwrap_or(&full).matches('/').count();
            near.push((d, i >= n_project, reach.is_none(), depth, full, reach.unwrap_or_default()));
        }
    }
    if !found.is_empty() { near.clear(); } // an exact match makes near misses noise
    near.sort(); near.dedup_by(|a, b| a.4 == b.4); near.truncate(MAX_NEAR);
    let near_json: Vec<Value> = near.iter().map(|(d, _sys, unreachable, _depth, path, reach)| {
        let mut v = json!({"path": path, "distance": d});
        if !unreachable { v["include_as"] = json!(reach); suggestions.push(json!({"message": format!("did you mean #include <{reach}>? ({path})"), "option": null})); }
        else {
            let dir = Path::new(path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let base = Path::new(path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let opt = format!("-I{}", rel_or_abs(&dir, &cwd));
            v["include_as"] = json!(base); v["option"] = json!(opt);
            suggestions.push(json!({"message": format!("did you mean '{base}'? found {path}, which is not on the include path; spell it that way and add {opt}"), "option": opt}));
        }
        v
    }).collect();
    if found.is_empty() && near.is_empty() {
        suggestions.push(json!({"message": format!("no file named '{name}' found in the project, /usr/include or /usr/local/include; if a package provides it, `dnf provides '*/{name}'` names it (not run: needs repository data)"), "option": null}));
    }
    json!({"schema": 1, "type": "missing-header", "code": "C0003", "header": name,
           "form": match angle { Some(true) => "angle", Some(false) => "quote", None => "unknown" },
           "message": message, "location": loc, "included_from": from_file,
           "searched": searched, "truncated": cut, "found": found, "packages": packages, "near_misses": near_json, "suggestions": suggestions})
}

fn rel_or_abs(p: &str, cwd: &Path) -> String {
    match Path::new(p).strip_prefix(cwd) { Ok(r) if r.as_os_str().is_empty() => ".".into(), Ok(r) => r.to_string_lossy().into_owned(), Err(_) => p.to_string() }
}

/// the header name from "'foo.h' file not found" (also "'foo.h' file not found, did you mean ...")
pub fn missing_name(msg: &str) -> Option<String> {
    let rest = msg.strip_prefix('\'')?; let i = rest.find("' file not found")?; Some(rest[..i].to_string())
}
