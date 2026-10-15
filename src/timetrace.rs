//! Feature 8: the slowest headers, templates and functions of a compile, from clang's -ftime-trace (JSON).
//! The trace goes to a private temp directory; it never lands next to the user's outputs.
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Deserialize)]
struct Args { detail: Option<String> }

#[derive(Deserialize)]
struct Ev {
    ph: String, name: String,
    ts: Option<u64>, dur: Option<u64>, tid: Option<u64>, id: Option<serde_json::Number>,
    args: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Trace { #[serde(rename = "traceEvents")] events: Vec<Ev> }

fn detail(e: &Ev) -> String {
    match &e.args { Some(v) => serde_json::from_value::<Args>(v.clone()).ok().and_then(|a| a.detail).unwrap_or_default(), None => String::new() }
}

fn ms(us: u64) -> f64 { (us as f64 / 10.0).round() / 100.0 }

/// Nest intervals (start, end, key) by containment; returns (key, inclusive, self) per interval.
fn self_times<K: Clone>(mut iv: Vec<(u64, u64, K)>) -> Vec<(K, u64, u64)> {
    iv.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    let mut out: Vec<(K, u64, u64)> = Vec::with_capacity(iv.len());
    let mut stack: Vec<(u64, usize)> = vec![]; // (end, index into out)
    for (s, e, k) in iv {
        while let Some(&(end, _)) = stack.last() { if s >= end { stack.pop(); } else { break; } }
        let incl = e.saturating_sub(s);
        if let Some(&(_, pi)) = stack.last() { out[pi].2 = out[pi].2.saturating_sub(incl); }
        out.push((k, incl, incl));
        stack.push((e, out.len() - 1));
    }
    out
}

pub fn report(json_text: &str, input: &str, top: usize) -> Option<Value> {
    let t: Trace = serde_json::from_str(json_text).ok()?;
    let ev = &t.events;
    // phases
    // the phases that matter; they overlap (the frontend contains header parsing and template instantiation)
    let mut phases = serde_json::Map::new();
    for (trace_name, key) in [("Frontend", "frontend_ms"), ("Backend", "backend_ms"), ("Source", "header_parse_ms"), ("InstantiateClass", "instantiate_class_ms"),
                              ("InstantiateFunction", "instantiate_function_ms"), ("OptModule", "optimize_ms"), ("CodeGenPasses", "codegen_ms")] {
        if let Some(e) = ev.iter().find(|e| e.ph == "X" && e.name == format!("Total {trace_name}")) { phases.insert(key.into(), json!(ms(e.dur.unwrap_or(0)))); }
    }
    let total = ev.iter().find(|e| e.name == "Total ExecuteCompiler").and_then(|e| e.dur).unwrap_or(0);
    // headers: pair the async b/e events of cat Source by id
    let mut open: HashMap<String, (u64, String)> = HashMap::new(); let mut ivs: Vec<(u64, u64, String)> = vec![];
    for e in ev.iter().filter(|e| e.name == "Source") {
        let id = e.id.as_ref().map(|n| n.to_string()).unwrap_or_default();
        match e.ph.as_str() { "b" => { open.insert(id, (e.ts.unwrap_or(0), detail(e))); }
            "e" => { if let Some((s, d)) = open.remove(&id) { ivs.push((s, e.ts.unwrap_or(s), d)); } } _ => {} }
    }
    let mut headers: HashMap<String, (u64, u64, u32)> = HashMap::new();
    for (path, incl, own) in self_times(ivs) { let h = headers.entry(path).or_default(); h.0 += incl; h.1 += own; h.2 += 1; }
    let mut hv: Vec<_> = headers.into_iter().collect(); hv.sort_by(|a, b| b.1.1.cmp(&a.1.1).then(a.0.cmp(&b.0)));
    // templates and functions: complete events per thread, self time by containment
    let mut by_tid: HashMap<u64, Vec<(u64, u64, (String, String))>> = HashMap::new();
    for e in ev.iter().filter(|e| e.ph == "X" && e.dur.is_some() && e.ts.is_some()) {
        by_tid.entry(e.tid.unwrap_or(0)).or_default().push((e.ts.unwrap(), e.ts.unwrap() + e.dur.unwrap(), (e.name.clone(), detail(e))));
    }
    let mut templ: HashMap<(String, String), (u64, u64, u32)> = HashMap::new();
    let mut funcs: HashMap<String, (u64, u64, u32)> = HashMap::new(); // name -> (opt, codegen, count)
    for (_, iv) in by_tid {
        for ((name, det), incl, own) in self_times(iv) {
            match name.as_str() {
                "InstantiateClass" | "InstantiateFunction" => { let t = templ.entry((name, det)).or_default(); t.0 += incl; t.1 += own; t.2 += 1; }
                "OptFunction" => { let f = funcs.entry(det).or_default(); f.0 += incl; f.2 += 1; }
                "CodeGen Function" => { let f = funcs.entry(det).or_default(); f.1 += incl; }
                _ => {}
            }
        }
    }
    let mut tv: Vec<_> = templ.into_iter().collect(); tv.sort_by(|a, b| b.1.1.cmp(&a.1.1).then(a.0.cmp(&b.0)));
    let mut fv: Vec<_> = funcs.into_iter().collect(); fv.sort_by(|a, b| (b.1.0 + b.1.1).cmp(&(a.1.0 + a.1.1)).then(a.0.cmp(&b.0)));
    let headers_j: Vec<Value> = hv.iter().take(top).map(|(p, (i, s, n))| json!({"path": crate::paths::tidy(p), "self_ms": ms(*s), "inclusive_ms": ms(*i), "count": n})).collect();
    let templ_j: Vec<Value> = tv.iter().take(top).map(|((k, d), (i, s, n))| json!({"name": d, "kind": k, "self_ms": ms(*s), "inclusive_ms": ms(*i), "count": n})).collect();
    let funcs_j: Vec<Value> = fv.iter().take(top).map(|(n, (o, c, k))| json!({"name": crate::whynot::demangle(n), "optimize_ms": ms(*o), "codegen_ms": ms(*c), "total_ms": ms(*o + *c), "count": k})).collect();
    let mut suggestions: Vec<Value> = vec![];
    if let Some((p, (i, _, _))) = hv.first() {
        if total > 0 && *i * 4 >= total { suggestions.push(json!({"message": format!("{p} takes {:.0}% of the compile ({} ms parsing, with what it includes): a precompiled header (-include-pch) or a lighter include would cut it", *i as f64 * 100.0 / total as f64, ms(*i)), "option": null})); }
    }
    Some(json!({"schema": 1, "type": "compile-time", "file": input, "total_ms": ms(total), "phases": phases, "phases_overlap": true,
                "slowest": {"headers": headers_j, "templates": templ_j, "functions": funcs_j},
                "events": ev.len(), "suggestions": suggestions}))
}
