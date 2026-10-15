//! Agent-mode switches: RUSTY_AGENT=1 or --agent-mode, plus one flag per feature. Our flags are removed
//! from the argument list before clang sees it.
use crate::whynot::What;
use std::ffi::OsString;

#[derive(Default, Debug)]
pub struct Config {
    pub agent: bool,
    pub diag_json: bool,
    pub root_cause: bool,
    pub fix_preview: bool,
    pub explain_header: bool,
    pub build_summary: bool,
    pub stable: bool,
    pub agent_help: bool,
    pub time_report: Option<usize>,
    pub effective_flags: bool,
    pub why: Vec<(What, String)>,
    pub detail: crate::shape::Detail,
    pub detail_flag: bool,
}

pub struct Parsed { pub cfg: Config, pub args: Vec<OsString> }

pub fn parse(args: Vec<OsString>) -> Parsed {
    let env_on = std::env::var_os("RUSTY_AGENT").is_some_and(|v| v == "1");
    let mut cfg = Config { agent: env_on, ..Default::default() };
    // RUSTY_AGENT_DETAIL=brief|normal|full; the flag --agent-detail=... wins; an unknown value changes nothing
    if let Some(d) = std::env::var("RUSTY_AGENT_DETAIL").ok().and_then(|v| crate::shape::Detail::parse(&v)) { cfg.detail = d; }
    let mut diag_text = false; let mut no_root = false; let mut no_summary = false; let mut no_stable = false; let mut no_explain = false;
    let mut kept = Vec::with_capacity(args.len());
    let mut after_dd = false;
    for a in args {
        if !after_dd {
            match a.to_str() {
                Some("--") => after_dd = true,
                Some("--agent-mode") => { cfg.agent = true; continue; }
                Some("--no-agent-mode") => { cfg.agent = false; continue; }
                Some("--diagnostics-format=json") => { cfg.diag_json = true; continue; }
                Some("--diagnostics-format=text") => { diag_text = true; continue; }
                Some("--explain-missing-header") => { cfg.explain_header = true; continue; }
                Some("--no-explain-missing-header") => { no_explain = true; continue; }
                Some("--build-summary") => { cfg.build_summary = true; continue; }
                Some("--no-build-summary") => { no_summary = true; continue; }
                Some("--stable-diagnostics") => { cfg.stable = true; continue; }
                Some("--no-stable-diagnostics") => { no_stable = true; continue; }
                Some(t) if t.starts_with("--why-not-inlined=") => { cfg.why.push((What::Inlined, t["--why-not-inlined=".len()..].to_string())); continue; }
                Some(t) if t.starts_with("--why-not-vectorized=") => { cfg.why.push((What::Vectorized, t["--why-not-vectorized=".len()..].to_string())); continue; }
                Some(t) if t.starts_with("--why-not-unrolled=") => { cfg.why.push((What::Unrolled, t["--why-not-unrolled=".len()..].to_string())); continue; }
                Some("--effective-flags") => { cfg.effective_flags = true; continue; }
                Some("--compile-time-report") => { cfg.time_report = Some(5); continue; }
                Some(t) if t.starts_with("--compile-time-report=") => { cfg.time_report = Some(t["--compile-time-report=".len()..].parse().unwrap_or(5).max(1)); continue; }
                Some(t) if t.starts_with("--agent-detail=") => { if let Some(d) = crate::shape::Detail::parse(&t["--agent-detail=".len()..]) { cfg.detail = d; cfg.detail_flag = true; } continue; }
                Some("--agent-help") => { cfg.agent_help = true; continue; }
                Some("--fix-preview") => { cfg.fix_preview = true; continue; }
                Some("--root-cause") => { cfg.root_cause = true; continue; }
                Some("--no-root-cause") => { no_root = true; continue; }
                _ => {}
            }
        }
        kept.push(a);
    }
    if cfg.agent && !diag_text { cfg.diag_json = true; }
    if cfg.agent && !no_explain { cfg.explain_header = true; }
    if no_explain { cfg.explain_header = false; }
    if cfg.agent && !no_stable { cfg.stable = true; }
    if no_stable { cfg.stable = false; }
    if cfg.agent && !no_summary { cfg.build_summary = true; }
    if no_summary { cfg.build_summary = false; }
    if cfg.agent && !no_root { cfg.root_cause = true; }
    if cfg.root_cause { cfg.diag_json = true; }
    if no_root { cfg.root_cause = false; }
    if diag_text { cfg.diag_json = false; }
    Parsed { cfg, args: kept }
}

impl Config {
    /// true when anything needs the wrapper to stay alive after clang starts
    pub fn needs_driver(&self) -> bool { self.diag_json || self.agent_help || self.agent || self.fix_preview || self.explain_header || self.build_summary || self.stable || self.time_report.is_some() || self.effective_flags || !self.why.is_empty() }
}
