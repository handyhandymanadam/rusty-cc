#!/usr/bin/env python3
"""Feature 11: --agent-help, --help text and the --version lines; the description must match what the tool really does."""
import json, os, subprocess, sys, glob
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures"); CLANG = "/usr/bin/clang"
env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}; env["RUSTY_AGENT_DETAIL"] = "full"
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:500]))
def run(args, e=env, cwd=FX): return subprocess.run([RCC] + args, cwd=cwd, capture_output=True, env=e)

p = run(["--agent-help"]); h = json.loads(p.stdout)
check("--agent-help: one JSON object on stdout, exit 0, nothing on stderr", p.returncode == 0 and p.stderr == b"" and h["schema"] == 1 and h["type"] == "help" and h["tool"] == "rusty-cc", p.stdout[:100])
check("--agent-help works with agent mode on too", json.loads(run(["--agent-help"], dict(env, RUSTY_AGENT="1")).stdout) == h)
check("switch documented", h["switch"]["env"] == "RUSTY_AGENT=1" and h["switch"]["flag"] == "--agent-mode")
# every documented option is really consumed (clang never sees it)
for o in h["options"]:
    name = o["option"]
    if name == "run-san": continue
    arg = name.replace("[=N]", "").replace("=NAME", "=main").replace("=LEVEL", "=normal")
    q = run([arg, "-fsyntax-only", "clean.c"]) if name != "--agent-help" else None
    if q is None: continue
    check(f"option {name} is consumed by the wrapper", b"unknown argument" not in q.stderr and q.returncode == 0, q.stderr)
# the umbrella list in the help equals what --agent-mode switches on
on = {o["feature"] for o in h["options"] if o["in_agent_mode"]}
check("umbrella features match the options marked in_agent_mode", on == set(h["umbrella_features"]) == {"diagnostics", "root-cause", "missing-header", "summary", "stable"}, on)
p = run(["--agent-mode", "-c", "errors3.c", "-o", "/dev/null"]); types = {json.loads(l)["type"] for l in p.stderr.decode().splitlines() if l.startswith("{")}
check("--agent-mode really produces the umbrella record types", types == {"diagnostic", "summary"}, types)  # errors3 has no missing header
# record types and codes seen in real runs are all documented
seen_types, seen_codes = set(), set()
def collect(args, cwd=FX):
    q = run(args, cwd=cwd)
    for l in q.stderr.decode().splitlines():
        if l.startswith('{"schema"'):
            d = json.loads(l); seen_types.add(d["type"])
            if d["type"] == "diagnostic" and d["code"].startswith("C"): seen_codes.add(d["code"])
    return q
for f in sorted(glob.glob(FX + "/*.c") + glob.glob(FX + "/*.cpp")):
    n = os.path.basename(f)
    if n.startswith("san_") or n in ("ct.cpp",): continue
    collect(["--agent-mode", "--fix-preview", "-Wall", "-Wextra", "-fsyntax-only", n])
collect(["--explain-missing-header", "-fsyntax-only", "src/missing_none.c"], FX + "/hdr")
collect(["--why-not-inlined=small", "-O2", "-c", "opt.c", "-o", "/dev/null"]); collect(["--effective-flags", "-c", "clean.c", "-o", "/dev/null"])
collect(["--compile-time-report", "-c", "clean.c", "-o", "/dev/null"]); collect(["--agent-mode", "-fno-such-flag", "-c", "clean.c"]); collect(["--agent-mode", "-c", "nope.c"]); collect(["--agent-mode", "linkfail.c", "-o", "/dev/null"])
documented_types = {r["type"].split(" ")[0] for r in h["record_types"]}
check("every record type seen in real runs is documented", seen_types <= documented_types, seen_types - documented_types)
documented_codes = {c["code"] for c in h["codes"]}
check("every C code seen in real runs is documented", seen_codes <= documented_codes, seen_codes - documented_codes)
check("seen at least 8 distinct codes (the test saw real variety)", len(seen_codes) >= 8, seen_codes)
check("every documented root-cause rule exists in the doc file", all(r["rule"] in open(os.path.join(root, "AGENT-MODE.md")).read() for r in h["root_cause_rules"]))
check("every documented option is in AGENT-MODE.md", all(o["option"].split("=")[0].split("[")[0] in open(os.path.join(root, "AGENT-MODE.md")).read() for o in h["options"] if o["option"] != "--agent-help") )
# --version and --help
plain = subprocess.run([CLANG, "--version"], capture_output=True).stdout
check("plain --version is clang's bytes", run(["--version"]).stdout == plain)
v = run(["--agent-mode", "--version"]).stdout.decode().splitlines()
check("agent --version: clang lines, then 'agent mode: yes, schema 1', then features", v[:len(plain.decode().splitlines())] == plain.decode().splitlines() and v[-2] == "agent mode: yes, schema 1" and v[-1].startswith("agent features: ") and "run-san" in v[-1], v[-3:])
check("RUSTY_AGENT=1 --version equal to the flag", run(["--version"], dict(env, RUSTY_AGENT="1")).stdout.decode().splitlines() == v)
ph = subprocess.run([CLANG, "--help"], capture_output=True)
check("plain --help is clang's bytes (stdout, stderr, exit)", (lambda q: q.stdout == ph.stdout and q.stderr == ph.stderr and q.returncode == ph.returncode)(run(["--help"])))
ah = run(["--agent-mode", "--help"])
check("agent --help: clang's help first, then our section listing every option", ah.stdout.startswith(ph.stdout) and all(o["option"].split("=")[0].split("[")[0] in ah.stdout[len(ph.stdout):].decode() for o in h["options"]), ah.stdout[len(ph.stdout):][:200])
check("agent --help exit code 0", ah.returncode == 0)
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
