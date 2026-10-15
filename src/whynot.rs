//! Feature 6: why was a function not inlined / its loops not vectorized / not unrolled?
//! Data comes from clang's optimization record (-fsave-optimization-record, written to a temp file), not from
//! parsing -Rpass text, so function names and reasons are exact.
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum What { Inlined, Vectorized, Unrolled }

impl What {
    pub fn word(self) -> &'static str { match self { What::Inlined => "inlined", What::Vectorized => "vectorized", What::Unrolled => "unrolled" } }
}

#[derive(Debug, Clone, Default)]
pub struct Remark {
    pub kind: String, pub pass: String, pub name: String,
    pub file: Option<String>, pub line: u32, pub col: u32,
    pub function: String,
    pub args: Vec<(String, String)>,
}

impl Remark {
    pub fn message(&self) -> String { self.args.iter().map(|(_, v)| v.as_str()).collect() }
    pub fn arg(&self, key: &str) -> Option<&str> { self.args.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()) }
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') { v[1..v.len() - 1].replace("''", "'") }
    else if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') { v[1..v.len() - 1].replace("\\\"", "\"").replace("\\n", "\n").replace("\\\\", "\\") }
    else { v.to_string() }
}

fn debugloc(v: &str) -> Option<(String, u32, u32)> {
    // { File: w.c, Line: 5, Column: 106 }
    let inner = v.trim().trim_start_matches('{').trim_end_matches('}');
    let (mut file, mut line, mut col) = (None, 0, 0);
    // File may be quoted and contain commas; find the keys by name
    let fi = inner.find("File:")?; let li = inner.rfind(", Line:")?; let ci = inner.rfind(", Column:")?;
    if fi < li { file = Some(unquote(inner[fi + 5..li].trim())); }
    line = inner[li + 7..ci].trim().parse().ok().unwrap_or(line);
    col = inner[ci + 9..].trim().parse().ok().unwrap_or(col);
    Some((file?, line, col))
}

/// Parse an optimization record in clang's YAML (a stream of `--- !Kind` documents).
pub fn parse_record(text: &str) -> Vec<Remark> {
    let mut out = vec![]; let mut cur: Option<Remark> = None; let mut in_args = false;
    for l in text.lines() {
        if let Some(k) = l.strip_prefix("--- !") { if let Some(r) = cur.take() { out.push(r); } cur = Some(Remark { kind: k.trim().to_string(), ..Default::default() }); in_args = false; continue; }
        if l == "..." { if let Some(r) = cur.take() { out.push(r); } in_args = false; continue; }
        let Some(r) = cur.as_mut() else { continue };
        if let Some(rest) = l.strip_prefix("  - ") {
            if let Some((k, v)) = rest.split_once(':') { r.args.push((k.trim().to_string(), unquote(v.trim_start()))); }
            continue;
        }
        if l.starts_with("    ") && in_args { continue; } // an argument's own DebugLoc
        in_args = false;
        if let Some((k, v)) = l.split_once(':') {
            match k { "Pass" => r.pass = v.trim().to_string(), "Name" => r.name = v.trim().to_string(), "Function" => r.function = unquote(v),
                "DebugLoc" => if let Some((f, ln, c)) = debugloc(v) { r.file = Some(f); r.line = ln; r.col = c; },
                "Args" => in_args = true, _ => {} }
        }
    }
    if let Some(r) = cur.take() { out.push(r); }
    out
}

pub fn demangle(name: &str) -> String {
    if name.starts_with("_Z") { if let Ok(s) = cpp_demangle::Symbol::new(name) { if let Ok(d) = s.demangle() { return d; } } }
    name.to_string()
}

/// "ns::Foo<int>::bar(int)" matches NAME "bar", "Foo::bar", "ns::Foo<int>::bar" ...
pub fn name_matches(full: &str, want: &str) -> bool {
    let d = demangle(full);
    let mut base = String::new(); let mut depth = 0i32;
    for c in d.chars() { match c { '<' => depth += 1, '>' => depth -= 1, '(' if depth <= 0 => break, _ if depth <= 0 => base.push(c), _ => {} } }
    base == want || base.ends_with(&format!("::{want}")) || d == want || d.split('(').next().is_some_and(|b| b == want || b.ends_with(&format!("::{want}")))
}

/// Source span of function NAME from `clang -Xclang -ast-dump -Xclang -ast-dump-filter=NAME` output: (file, first line, last line)
pub fn spans_from_ast(dump: &str, want: &str) -> Vec<(String, u32, u32)> {
    let mut out = vec![]; let (mut file, mut line) = (String::new(), 0u32);
    let loc = |tok: &str, file: &mut String, line: &mut u32| -> Option<(u32, u32)> {
        // file:L:C | line:L:C | col:C
        let tok = tok.trim();
        if let Some(c) = tok.strip_prefix("col:") { return Some((*line, c.parse().ok()?)); }
        if let Some(r) = tok.strip_prefix("line:") { let (l, c) = r.split_once(':')?; *line = l.parse().ok()?; return Some((*line, c.parse().ok()?)); }
        let mut it = tok.rsplitn(3, ':'); let c = it.next()?.parse().ok()?; let l: u32 = it.next()?.parse().ok()?; let f = it.next()?; *file = f.to_string(); *line = l; Some((l, c))
    };
    for l in dump.lines() {
        let Some(i) = l.find('<') else { continue };
        let head = &l[..i]; if !(head.contains("FunctionDecl") || head.contains("CXXMethodDecl") || head.contains("CXXConstructorDecl")) { continue; }
        let Some(j) = l[i..].find('>') else { continue }; let range = &l[i + 1..i + j];
        let Some((a, b)) = range.split_once(", ") else { continue };
        let Some((sl, _)) = loc(a, &mut file, &mut line) else { continue };
        let f0 = file.clone();
        let Some((el, _)) = loc(b, &mut file, &mut line) else { continue };
        // the declared name is the word before the first quote after the location
        let after = &l[i + j + 1..]; let Some(q) = after.find(" '") else { continue };
        let before = after[..q].trim(); let name = before.rsplit(' ').next().unwrap_or("");
        let name = if name == "used" || name == "implicit" { before.split(' ').rev().nth(1).unwrap_or(name) } else { name };
        if name_matches(name, want) { out.push((f0, sl, el)); }
    }
    out
}

fn passes(what: What) -> &'static [&'static str] {
    match what { What::Inlined => &["inline"], What::Vectorized => &["loop-vectorize"], What::Unrolled => &["loop-unroll", "TTI"] }
}

fn in_spans(r: &Remark, spans: &[(String, u32, u32)]) -> bool {
    match &r.file { Some(f) => spans.iter().any(|(sf, a, b)| (sf == f || f.ends_with(sf.as_str()) || sf.ends_with(f.as_str())) && r.line >= *a && r.line <= *b), None => false }
}

fn loc_json(r: &Remark) -> Value { match &r.file { Some(f) => json!({"file": f, "line": r.line, "column": r.col}), None => Value::Null } }

fn hint(name: &str) -> Option<(&'static str, Option<&'static str>)> {
    Some(match name {
        "TooCostly" => ("the call is too costly for the inliner's threshold; mark the function `static inline` or `__attribute__((always_inline))`, or raise the threshold", None),
        "NoDefinition" => ("the definition is not visible in this translation unit; put it in a header, or compile with -flto so the linker can inline across files", Some("-flto")),
        "NeverInline" => ("the function can never be inlined here (noinline attribute, recursion, or similar, see the message)", None),
        "CantVectorizeLibcall" => ("a library call in the loop blocks vectorization; move it out of the loop, or for math functions try -fno-math-errno", None),
        "CantVectorizeInstruction" => ("an instruction in the loop has no vector form; see the loop's other reasons", None),
        "NonReductionValueUsedOutsideLoop" => ("a value computed in the loop is used after it in a way that is not a plain reduction; restructure so only a sum/min/max/product leaves the loop", None),
        "NoCFGForSelect" => ("a branch inside the loop cannot be turned into a select; make the body branch-free or compute both sides", None),
        "CantComputeNumberOfIterations" => ("the loop count is not known before the loop starts; use a simple counted loop", None),
        "UnsafeMemDep" | "UnsafeDep" => ("the compiler cannot prove the loop's memory accesses do not overlap; add `restrict` to the pointers or `#pragma clang loop vectorize(assume_safety)` if it is true", None),
        "CantReorderFPOps" => ("reordering floating-point additions would change the result; -ffast-math or `#pragma clang loop vectorize(enable)` allow it", None),
        "DontUnroll" => ("the target advises against unrolling this loop (see the message)", None),
        _ => return None,
    })
}

/// "1 loop in 'f' was not vectorized (1 was)", "2 loops in 'f' were not vectorized, none were"
fn loops_summary(bad: usize, ok: usize, want: &str, word: &str) -> String {
    let loops = |n: usize| if n == 1 { "1 loop".to_string() } else { format!("{n} loops") };
    let rest = match ok { 0 => String::new(), 1 => format!("; 1 other was {word}"), n => format!("; {n} others were {word}") };
    if bad == 1 { format!("{} in '{want}' was not {word}{rest}", loops(bad)) } else { format!("{} in '{want}' were not {word}{rest}", loops(bad)) }
}

/// Build the why-not record. `remarks` is the whole optimization record of the single translation unit,
/// `opt_level` the -O option given (None = none), `spans` the source span(s) of NAME.
pub fn record(what: What, want: &str, remarks: Option<&[Remark]>, opt_level: Option<&str>, spans: &[(String, u32, u32)], why_no_data: Option<&str>) -> Value {
    let mut o = Map::new();
    o.insert("schema".into(), json!(1)); o.insert("type".into(), json!("why-not"));
    o.insert("what".into(), json!(what.word())); o.insert("function".into(), json!(want));
    o.insert("optimization".into(), json!(opt_level));
    let mut events: Vec<Value> = vec![]; let mut suggestions: Vec<Value> = vec![];
    let mut outcome = "no-data"; let mut summary = String::new();
    let level_none = opt_level.is_none() || opt_level == Some("-O0");
    let Some(rs) = remarks else {
        o.insert("outcome".into(), json!("no-data"));
        o.insert("summary".into(), json!(why_no_data.unwrap_or("no optimization record was produced")));
        o.insert("events".into(), json!([])); o.insert("suggestions".into(), json!([]));
        return Value::Object(o);
    };
    match what {
        What::Inlined => {
            let (mut inlined, mut missed) = (0, 0); let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
            for r in rs.iter().filter(|r| r.pass == "inline") {
                let callee = r.arg("Callee").unwrap_or(""); let caller = r.arg("Caller").unwrap_or(&r.function);
                let (is_callee, is_caller) = (name_matches(callee, want), name_matches(caller, want));
                if !is_callee && !is_caller { continue; }
                let role = if is_callee { "callee" } else { "caller" };
                if is_callee { if r.kind == "Passed" { inlined += 1 } else { missed += 1; *reasons.entry(r.name.clone()).or_default() += 1; } }
                let mut details = Map::new();
                for k in ["Cost", "Threshold", "Callee", "Caller"] { if let Some(v) = r.arg(k) { details.insert(k.to_lowercase(), json!(if k == "Callee" || k == "Caller" { demangle(v) } else { v.to_string() })); } }
                events.push(json!({"role": role, "status": if r.kind == "Passed" { "inlined" } else { "not-inlined" }, "reason": r.name, "message": r.message(), "location": loc_json(r), "details": details}));
            }
            if missed > 0 {
                outcome = "not-optimized";
                let parts: Vec<String> = reasons.iter().map(|(k, n)| format!("{k} x{n}")).collect();
                let sites = |n: usize| if n == 1 { "1 call site".to_string() } else { format!("{n} call sites") };
                summary = format!("'{want}' was inlined at {} and not at {}: {}", sites(inlined), sites(missed), parts.join(", "));
                for k in reasons.keys() { if let Some((m, opt)) = hint(k) { suggestions.push(json!({"message": m, "option": opt})); } }
                // a cost-based refusal: show the numbers
                for e in &events { if e["reason"] == "TooCostly" { if let (Some(c), Some(t)) = (e["details"]["cost"].as_str(), e["details"]["threshold"].as_str()) { suggestions.push(json!({"message": format!("cost {c} against threshold {t} at {}:{}", e["location"]["file"].as_str().unwrap_or("?"), e["location"]["line"]), "option": null})); break; } } }
            } else if inlined > 0 {
                outcome = "optimized"; summary = if inlined == 1 { format!("'{want}' was inlined at its only call site; nothing to explain") } else { format!("'{want}' was inlined at all {inlined} call sites; nothing to explain") };
            }
        }
        What::Vectorized | What::Unrolled => {
            // group by loop (place); the same loop can appear once per inlined copy
            // remarks about one loop can point at the loop head or at the offending instruction: group by line
            let mut loops: BTreeMap<(String, u32), (bool, Vec<Remark>, usize)> = BTreeMap::new();
            for r in rs.iter().filter(|r| passes(what).contains(&r.pass.as_str())) {
                if !(name_matches(&r.function, want) || in_spans(r, spans)) { continue; }
                let Some(f) = &r.file else { continue };
                let e = loops.entry((f.clone(), r.line)).or_insert((false, vec![], 0));
                let done = r.kind == "Passed" && (r.name == "Vectorized" || r.name.contains("Unrolled") && !r.name.starts_with("Not") || r.name == "Interleaved");
                if done { e.0 = true; }
                if !e.1.iter().any(|x| x.name == r.name && x.message() == r.message()) { e.1.push(r.clone()); }
                e.2 += 1;
            }
            let (mut ok, mut bad) = (0, 0);
            for ((f, line), (done, rs2, _n)) in &loops {
                let col = rs2.iter().filter(|r| r.name == "MissedDetails" || r.kind == "Passed").map(|r| r.col).min().or_else(|| rs2.iter().map(|r| r.col).min()).unwrap_or(0);
                if *done { ok += 1; } else { bad += 1; }
                let reasons: Vec<Value> = rs2.iter().filter(|r| r.name != "MissedDetails").map(|r| json!({"reason": r.name, "message": r.message(), "pass": r.pass})).collect();
                events.push(json!({"role": "loop", "status": if *done { what.word().to_string() } else { format!("not-{}", what.word()) }, "location": {"file": f, "line": line, "column": col},
                                   "function": demangle(&rs2[0].function), "reasons": reasons}));
                if !*done { for r in rs2 { if let Some((m, opt)) = hint(&r.name) { let s = json!({"message": m, "option": opt}); if !suggestions.contains(&s) { suggestions.push(s); } } } }
            }
            if bad > 0 { outcome = "not-optimized"; summary = loops_summary(bad, ok, want, what.word()); }
            else if ok > 0 { outcome = "optimized"; summary = if ok == 1 { format!("the loop in '{want}' was {}", what.word()) } else { format!("all {ok} loops in '{want}' were {}", what.word()) }; }
        }
    }
    if events.is_empty() && outcome == "no-data" {
        let known = rs.iter().any(|r| name_matches(&r.function, want) || r.arg("Callee").is_some_and(|c| name_matches(c, want)));
        summary = if level_none { format!("no optimization level was given, so clang compiled at -O0 and the optimizer did not run; add -O2 to see why '{want}' is or is not {}", what.word()) }
        else if what != What::Inlined && !known && spans.is_empty() { format!("no function named '{want}' was found in this translation unit") }
        else if what == What::Inlined && !known { format!("'{want}' never appeared as a call target in the optimization record: it is not called here, was removed before inlining (an unused static function), or is only declared; with -flto the answer may differ") }
        else { format!("the optimizer recorded nothing about {} of '{want}': no loop there, or it was handled before this pass", what.word()) };
    }
    let total = events.len(); events.truncate(60);
    o.insert("outcome".into(), json!(outcome)); o.insert("summary".into(), json!(summary));
    o.insert("events".into(), json!(events)); o.insert("events_omitted".into(), json!(total.saturating_sub(60)));
    o.insert("suggestions".into(), json!(suggestions));
    Value::Object(o)
}
