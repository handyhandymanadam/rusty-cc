#!/usr/bin/env python3
"""Feature 5: the end-of-build summary record, on real compiles."""
import json, os, re, subprocess, sys
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures"); CLANG = "/usr/bin/clang"
env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}; env["RUSTY_AGENT_DETAIL"] = "full"
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:500]))
def run(flags, args):
    p = subprocess.run([RCC] + flags + args, cwd=FX, capture_output=True, env=env)
    return [json.loads(l) for l in p.stderr.decode().splitlines() if l.startswith('{"schema"')], p
def clang_counts(args):
    t = subprocess.run([CLANG, "-fno-color-diagnostics"] + args, cwd=FX, capture_output=True).stderr.decode()
    w = sum(int(m) for m in re.findall(r"(\d+) warnings? ", t.split("\n")[-2] if "generated" in t else "")) if False else None
    return t

r, p = run(["--agent-mode"], ["-c", "errors3.c", "-o", "/dev/null"]); s = r[-1]
check("summary is the last record", s["type"] == "summary" and s["schema"] == 1 and s["tool"] == "rusty-cc", [x["type"] for x in r])
check("exit equals process exit", s["exit"] == p.returncode == 1)
check("counts equal clang's own '4 errors generated.'", s["diagnostics"]["errors"] == 4 and "4 errors generated" in clang_counts(["-c", "errors3.c", "-o", "/dev/null"]), s["diagnostics"])
check("files compiled named, no link", s["files"] == {"compiled": ["errors3.c"], "linked": False}, s["files"])
check("stable mode (in the umbrella): no elapsed_ms, a clock value", "elapsed_ms" not in s, s)
r2, p2 = run(["--agent-mode", "--no-stable-diagnostics"], ["-c", "errors3.c", "-o", "/dev/null"]); s2 = r2[-1]
check("elapsed is a non-negative integer (without stable)", isinstance(s2["elapsed_ms"], int) and s2["elapsed_ms"] >= 0)
check("command is the clang command actually run (with our flags)", s["command"][0] == "/usr/bin/clang" and s["command"][1:4] == ["-c", "errors3.c", "-o"] and "-fdiagnostics-parseable-fixits" in s["command"], s["command"])

r, p = run(["--agent-mode"], ["-Wall", "-Wextra", "-c", "warn.c", "-o", "/dev/null"]); s = r[-1]
check("warnings counted (5), exit 0", s["diagnostics"]["warnings"] == 5 and s["diagnostics"]["errors"] == 0 and s["exit"] == 0)

r, p = run(["--agent-mode"], ["-fsyntax-only", "cascade_names.c"]); s = r[-1]
check("root cause: totals still equal clang's 6 errors, records fewer", s["diagnostics"]["errors"] == 6 and s["diagnostics"]["records"] == 2 and s["diagnostics"]["suppressed"] == 4, s["diagnostics"])

r, p = run(["--agent-mode"], ["linkfail.c", "-o", "/tmp/rcc-lf.out"]); s = r[-1]
check("link failure: linker errors counted from its JSON, link step seen", s["linker"]["errors"] == 1 and s["files"]["linked"] is True and s["exit"] == 1 and s["files"]["compiled"] == ["linkfail.c"], s)

r, p = run(["--agent-mode"], ["clean.c", "warn.c", "-o", "/tmp/rcc-two.out"]); s = r[-1]
check("two inputs both listed", s["files"]["compiled"] == ["clean.c", "warn.c"], s["files"])

r, p = run(["--agent-mode"], ["-c", "clean.c", "-o", "/dev/null"])
check("clean compile: only the summary record", [x["type"] for x in r] == ["summary"] and r[0]["exit"] == 0)
# stdout of the program/tool untouched
p = subprocess.run([RCC, "--agent-mode", "-E", "clean.c"], cwd=FX, capture_output=True, env=env)
check("stdout of -E untouched", p.stdout == subprocess.run([CLANG, "-E", "clean.c"], cwd=FX, capture_output=True).stdout)
# alone and off
p = subprocess.run([RCC, "--build-summary", "-c", "errors3.c", "-o", "/dev/null"], cwd=FX, capture_output=True, env=env)
txt = p.stderr.decode(); plain = subprocess.run([CLANG, "-fno-color-diagnostics", "-c", "errors3.c", "-o", "/dev/null"], cwd=FX, capture_output=True).stderr.decode()
check("--build-summary alone: clang text then the summary", txt.startswith(plain) and json.loads(txt[len(plain):])["diagnostics"]["errors"] == 4, txt[-200:])
r, p = run(["--agent-mode", "--no-build-summary"], ["-c", "errors3.c", "-o", "/dev/null"])
check("--no-build-summary", all(x["type"] != "summary" for x in r))
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
