//! Feature 11: self-description. One table drives --agent-help (JSON) and the text added to --help in agent mode.
use serde_json::{json, Value};

/// (option, argument, feature, in the --agent-mode umbrella, what it does)
pub const OPTIONS: &[(&str, &str, &str, bool, &str)] = &[
    ("--agent-mode", "", "umbrella", false, "turn agent mode on (same as RUSTY_AGENT=1); switches on the features marked yes below"),
    ("--no-agent-mode", "", "umbrella", false, "turn agent mode off again"),
    ("--agent-detail=LEVEL", "LEVEL", "detail", false, "how much each record says: brief, normal (default in agent mode) or full; also RUSTY_AGENT_DETAIL=LEVEL, the flag wins"),
    ("--agent-help", "", "self-description", false, "print this description as one JSON object on standard output and exit"),
    ("--diagnostics-format=json", "", "diagnostics", true, "every compiler diagnostic as one JSON record on standard error; =text turns it off"),
    ("--root-cause", "", "root-cause", true, "group follow-on errors under the first cause, compress long note chains"),
    ("--no-root-cause", "", "root-cause", false, "turn root-cause grouping off inside agent mode"),
    ("--fix-preview", "", "fix-preview", false, "show clang's machine fix-its as unified diffs in a record; writes nothing"),
    ("--explain-missing-header", "", "missing-header", true, "explain each 'file not found': search dirs, candidates, exact -I, pkg-config package"),
    ("--no-explain-missing-header", "", "missing-header", false, "turn the header explainer off inside agent mode"),
    ("--build-summary", "", "summary", true, "a summary record last: exit code, elapsed ms, files, counts, the command run"),
    ("--no-build-summary", "", "summary", false, "turn the summary off inside agent mode"),
    ("--stable-diagnostics", "", "stable", true, "deterministic output: LC_ALL=C, sorted by place, temp names fixed"),
    ("--no-stable-diagnostics", "", "stable", false, "turn the stable mode off inside agent mode"),
    ("--why-not-inlined=NAME", "NAME", "why-not", false, "why was function NAME not inlined (from the optimization record; needs -O1 or higher)"),
    ("--why-not-vectorized=NAME", "NAME", "why-not", false, "why were the loops of NAME not vectorized"),
    ("--why-not-unrolled=NAME", "NAME", "why-not", false, "why were the loops of NAME not unrolled"),
    ("--effective-flags", "", "effective-flags", false, "target, sysroot, include dirs, defines, language, linker as one record"),
    ("--compile-time-report[=N]", "N", "compile-time", false, "slowest N (default 5) headers, templates and functions from -ftime-trace"),
    ("run-san", "-- PROGRAM ARGS", "sanitizer", false, "subcommand: run PROGRAM and turn its ASan/UBSan report into diagnostic records"),
];

pub const RECORDS: &[(&str, &str)] = &[
    ("diagnostic", "schema type code severity message [files symbols: full only] notes suggestions locations flag fixits{... alternative?} [rendered: full only] [werror include_stack suppressed follow_on notes_omitted more]; at normal and brief every empty field is absent"),
    ("remark", "same fields as diagnostic (an -Rpass optimization remark)"),
    ("fix-preview", "schema type dry_run written files{file safe_edits unsafe_edits conflicts diff diff_with_unsafe} alternatives{id diagnostic code note file edits diff} edits{alternative? file start end text safety reason code diagnostic in_diff in_diff_with_unsafe skipped} note. Alternatives (fix-its on notes) exclude each other and are never merged; diff_with_unsafe adds only a diagnostic's single alternative"),
    ("missing-header", "schema type code header form message location included_from searched truncated found packages near_misses suggestions"),
    ("summary", "schema type tool exit elapsed_ms command(full only) files{compiled linked} diagnostics{errors fatal warnings remarks records suppressed} linker{errors warnings} records_omitted?(size budget); elapsed_ms is not in stable mode"),
    ("why-not", "schema type what function optimization outcome summary events events_omitted suggestions"),
    ("effective-flags", "schema type clang target sysroot resource_dir language optimization debug_info position_independent include_dirs defines linker cc1_args no_input_given"),
    ("compile-time", "schema type file total_ms phases phases_overlap slowest{headers templates functions} events suggestions"),
    ("diagnostic (run-san)", "the diagnostic fields with code C0400 (paths relative to the current directory) plus sanitizer{name kind access size address region summary stacks[{role frames[{index function file line column module offset}]}]}"),
    ("help", "this object (--agent-help)"),
];

pub const CODES: &[(&str, &str, &str)] = &[
    ("C0001", "error", "use of undeclared identifier or label"), ("C0002", "error", "syntax error (expected ..., unterminated ...)"),
    ("C0003", "fatal", "header file not found"), ("C0004", "error", "implicit function declaration"),
    ("C0005", "error|warning", "type, pointer or integer conversion"), ("C0006", "warning", "unused variable, parameter, function, value"),
    ("C0007", "warning", "format string problem"), ("C0008", "error", "no matching function, constructor or overload"),
    ("C0009", "error", "linker command failed (the linker's own records come first, prefix L)"), ("C0010", "warning", "comparison problem (sign, tautology)"),
    ("C0011", "error|warning", "bad, unknown or unused command-line option"), ("C0012", "error", "input file missing or no input files"),
    ("C0013", "warning", "deprecated"), ("C0014", "warning", "uninitialized variable"), ("C0015", "error", "unknown type name, no such member or template"),
    ("C0016", "warning", "return type problem (missing return, address of local)"), ("C0017", "error", "redefinition or conflicting declaration"),
    ("C0018", "error", "static assertion failed"), ("C0100", "error", "any other error"), ("C0200", "warning", "any other warning, remark or note"),
    ("C0300", "fatal", "any other fatal error"), ("C0400", "error", "sanitizer finding (run-san)"), ("C0500", "warning", "agent-mode setup problem (RUSTY_AGENT_OUT cannot be used)"), ("C0501", "warning", "agent-mode setup problem (RUSTY_AGENT_FD cannot be used)"),
];

pub const ROOT_CAUSE_RULES: &[(&str, &str)] = &[
    ("same-place", "identical diagnostic at the same place"), ("same-error-repeated", "the same undeclared name or unknown type again"),
    ("mentions-undeclared-name", "a later error names an undeclared name or unknown type"), ("same-line-as-unknown-type", "'Foo f;' with Foo unknown makes f undeclared"),
    ("declared-with-unknown-type", "'foo_t x = 0;' with foo_t unknown never declares x: later uses of x are the same error"),
    ("syntax-error-nearby", "another syntax error within 10 lines in the same file"),
];

pub fn agent_help() -> Value {
    json!({"schema": 1, "type": "help", "tool": "rusty-cc", "version": env!("CARGO_PKG_VERSION"),
        "switch": {"env": "RUSTY_AGENT=1", "flag": "--agent-mode", "other_values_change_nothing": true},
        "umbrella_features": ["diagnostics", "root-cause", "missing-header", "summary", "stable"],
        "records_file": {"env": "RUSTY_AGENT_OUT=<path>", "what": "in agent mode every JSON record is also appended to this file, one per line, flushed per record; the folder must exist; empty path refused; a bad path is a warning record C0500, never a failure"},
        "records_fd": {"env": "RUSTY_AGENT_FD=<n>", "what": "in agent mode every JSON record is also appended to the already-open descriptor n (3 or more, open for writing: a pipe, file or terminal the caller left open), one per line, one write per record; with RUSTY_AGENT_OUT both get the records; a bad, closed or read-only descriptor is a warning record C0501, never a failure; the compiler and test programs do not inherit the variable; ignored when agent mode is off"},
        "detail": {"env": "RUSTY_AGENT_DETAIL=brief|normal|full", "flag": "--agent-detail=LEVEL", "default": "normal",
                   "brief": "code, message, one place, one fix, counts",
                   "normal": "what is needed to act: no rendered text, no files/symbols, no empty or zero fields (an absent field means empty, zero or null), nothing said twice, lists capped with <list>_omitted counts, the summary without the command line; about 12 KB soft and 20 KB hard of records per command (the summary's records_omitted says what was cut; RUSTY_AGENT_OUT or RUSTY_AGENT_FD keeps everything)",
                   "full": "every field as the first version wrote them (apart from the fixes listed in AGENT-MODE.md)"},
        "records_on": "standard error, one JSON object per line, schema and type first; standard output and build outputs are unchanged",
        "options": OPTIONS.iter().map(|(n, a, f, u, d)| json!({"option": n, "argument": a, "feature": f, "in_agent_mode": u, "description": d})).collect::<Vec<_>>(),
        "record_types": RECORDS.iter().map(|(t, f)| json!({"type": t, "fields": f})).collect::<Vec<_>>(),
        "codes": CODES.iter().map(|(c, s, m)| json!({"code": c, "severity": s, "meaning": m})).collect::<Vec<_>>(),
        "root_cause_rules": ROOT_CAUSE_RULES.iter().map(|(r, d)| json!({"rule": r, "meaning": d})).collect::<Vec<_>>(),
        "exit_code": "the exit code of clang (128+signal if clang was killed)",
        "docs": "AGENT-MODE.md"})
}

/// text appended to clang's --help in agent mode
pub fn help_text() -> String {
    let mut s = String::from("\nrusty-cc agent mode (rusty-cc --agent-help prints this as JSON):\n  agent mode is on with --agent-mode or RUSTY_AGENT=1; records are JSON lines on standard error.\n");
    for (n, a, _, u, d) in OPTIONS.iter() { s.push_str(&format!("  {:<30} {}{}\n", if a.is_empty() || n.contains('=') || n.contains('[') { n.to_string() } else { format!("{n} {a}") }, d, if *u { " [on in --agent-mode]" } else { "" })); }
    s.push_str("  records: diagnostic fix-preview missing-header summary why-not effective-flags compile-time; codes C0001..C0400.\n");
    s
}

/// capabilities line printed under --version in agent mode
pub fn capabilities() -> String {
    format!("agent features: {}", ["diagnostics", "root-cause", "summary", "stable", "fix-preview", "missing-header", "why-not", "effective-flags", "compile-time", "run-san"].join(" "))
}
