//! rusty-cc: thin driver in front of clang. Without agent mode it is a pure exec.
mod agent;
mod codes;
mod config;
mod diag;
mod effective;
mod fixpreview;
mod header;
mod paths;
mod help;
mod rootcause;
mod shape;
mod sanitizer;
mod tee;
mod timetrace;
mod whynot;

use std::env;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

const DEFAULT_CLANG: &str = "/usr/bin/clang";

/// Name clang's driver should see as argv[0]: it picks C++ mode from a "++" suffix.
fn driver_name(invoked: &OsStr) -> &'static str {
    let base = Path::new(invoked).file_name().unwrap_or(invoked);
    if base.as_bytes().windows(2).any(|w| w == b"++") { "clang++" } else { "clang" }
}

fn clang_path() -> OsString {
    env::var_os("RUSTY_CC_CLANG").filter(|v| !v.is_empty()).unwrap_or_else(|| DEFAULT_CLANG.into())
}

fn main() {
    let mut args = env::args_os();
    let invoked = args.next().unwrap_or_default();
    let mut args = args.peekable();
    if args.peek().is_some_and(|a| a == "run-san") { args.next(); let rest: Vec<OsString> = args.collect(); std::process::exit(sanitizer::run(&rest)); }
    let parsed = config::parse(args.collect());
    if parsed.cfg.agent_help { agent::stdout_text(&format!("{}\n", help::agent_help())); std::process::exit(0); }
    if parsed.cfg.agent {
        let c = &parsed.cfg;
        tee::setup(true, |file| shape::init(c.detail, c.stable, c.fix_preview, file));
    }
    let rest = parsed.args;
    if parsed.cfg.needs_driver() {
        std::process::exit(agent::run(&parsed.cfg, &clang_path(), driver_name(&invoked), &rest));
    }
    // Default path: nothing else is looked at. exec keeps stdio, environment, signals, exit code.
    let err = Command::new(clang_path()).arg0(driver_name(&invoked)).args(&rest).exec();
    eprintln!("rusty-cc: cannot run clang: {err}");
    std::process::exit(127);
}
