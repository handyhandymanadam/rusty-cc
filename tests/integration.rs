//! Runs the python test scripts (real clang compiles, JSON parsed in python) against the release binary.
use std::process::Command;

fn script(name: &str) {
    let root = env!("CARGO_MANIFEST_DIR");
    let st = Command::new("python3").arg("-I").arg(format!("{root}/tests/{name}")).env("RUSTY_CC_BIN", env!("CARGO_BIN_EXE_rusty-cc")).status().expect("python3");
    assert!(st.success(), "{name} failed");
}

#[test] fn passthrough() { script("passthrough.py"); }
#[test] fn diagnostics() { script("agent_diag.py"); }
#[test] fn root_cause() { script("agent_rootcause.py"); }
#[test] fn fix_preview() { script("agent_fixpreview.py"); }
#[test] fn header_explainer() { script("agent_header.py"); }
#[test] fn summary() { script("agent_summary.py"); }
#[test] fn stable() { script("agent_stable.py"); }
#[test] fn why_not() { script("agent_whynot.py"); }
#[test] fn effective_flags() { script("agent_effective.py"); }
#[test] fn compile_time() { script("agent_timetrace.py"); }
#[test] fn sanitizer() { script("agent_sanitizer.py"); }
#[test] fn help() { script("agent_help.py"); }
#[test] fn followup() { script("agent_followup.py"); }
#[test] fn fixes_misc() { script("agent_fixes.py"); }
#[test] fn agent_fd() { script("agent_fd.py"); }
#[test] fn detail_levels() { script("agent_detail.py"); }
#[test] fn fixes_fixpreview() { script("agent_fixes_fixpreview.py"); }
