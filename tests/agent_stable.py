#!/usr/bin/env python3
"""Feature 9: --stable-diagnostics, deterministic records."""
import json, os, re, stat, subprocess, sys, tempfile
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures"); CLANG = "/usr/bin/clang"
env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}; env["RUSTY_AGENT_DETAIL"] = "full"
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:500]))
def run(flags, args, e=env):
    return subprocess.run([RCC] + flags + args, cwd=FX, capture_output=True, env=e)
def mask(b): return re.sub(rb'"elapsed_ms":\d+', b'"elapsed_ms":0', b)
def recs(p): return [json.loads(l) for l in p.stderr.decode().splitlines() if l.startswith('{"schema"')]

# 1. two runs, byte-identical apart from elapsed_ms (compile with errors, warnings, template notes and a link failure)
for name, args in [("errors", ["-c", "errors3.c", "-o", "/dev/null"]), ("template", ["-c", "tmpl.cpp", "-o", "/dev/null"]), ("warnings", ["-Wall", "-Wextra", "-c", "warn.c", "-o", "/dev/null"]), ("link", ["linkfail.c", "-o", "/tmp/rcc-st.out"])]:
    a = run(["--agent-mode"], args); b = run(["--agent-mode"], args)
    check(f"{name}: two runs identical except elapsed_ms", mask(a.stderr) == mask(b.stderr) and a.returncode == b.returncode, (a.stderr[-200:], b.stderr[-200:]))
# 2. link failure: random temp object name is fixed
a = run(["--agent-mode"], ["linkfail.c", "-o", "/tmp/rcc-st.out"])
check("link: temp object name normalised to XXXXXX", b"linkfail-XXXXXX.o" in a.stderr and not re.search(rb"linkfail-[0-9a-f]{6}\.o", a.stderr), a.stderr[:300])
a = run(["--agent-mode", "--no-stable-diagnostics"], ["linkfail.c", "-o", "/tmp/rcc-st.out"]); b = run(["--agent-mode", "--no-stable-diagnostics"], ["linkfail.c", "-o", "/tmp/rcc-st.out"])
check("link: without the stable mode the names differ between runs (control)", mask(a.stderr) != mask(b.stderr))
# 3. order: by place
a = recs(run(["--agent-mode"], ["-Wall", "-Wextra", "-c", "warn.c", "-o", "/dev/null"]))
lines = [x["locations"][0]["line"] for x in a if x["type"] == "diagnostic"]
check("warnings sorted by line in stable mode", lines == sorted(lines) and len(lines) == 5, lines)
b = recs(run(["--agent-mode", "--no-stable-diagnostics"], ["-Wall", "-Wextra", "-c", "warn.c", "-o", "/dev/null"]))
lines2 = [x["locations"][0]["line"] for x in b if x["type"] == "diagnostic"]
check("clang's own order is not sorted (control)", lines2 != sorted(lines2), lines2)
# 4. driver diagnostics (no place) come last in their run
a = recs(run(["--agent-mode"], ["linkfail.c", "-o", "/tmp/rcc-st.out"]))
check("link: clang's driver error stays after the linker's record", [x.get("code") for x in a if x["type"] == "diagnostic"] == ["L0001", "C0009"], [x.get("code") for x in a])
# 5. two translation units keep input order
a = recs(run(["--agent-mode"], ["-fsyntax-only", "warn.c", "errors3.c"]))
files = [x["locations"][0]["file"] for x in a if x["type"] == "diagnostic"]
check("two TUs: first input's records first", files == ["warn.c"] * files.count("warn.c") + ["errors3.c"] * files.count("errors3.c") and "warn.c" in files, files)
# 6. child sees LC_ALL=C and no LANGUAGE
fake = tempfile.mkdtemp() + "/fakeclang"; open(fake, "w").write('#!/bin/sh\necho "LC_ALL=$LC_ALL LANG=$LANG LANGUAGE=${LANGUAGE-unset}" >&2\n'); os.chmod(fake, 0o755)
e = dict(env, RUSTY_CC_CLANG=fake, LC_ALL="de_DE.UTF-8", LANG="de_DE.UTF-8", LANGUAGE="de")
p = run(["--agent-mode"], ["-c", "x.c"], e)
check("stable mode: clang runs with LC_ALL=C LANG=C and LANGUAGE unset", b"LC_ALL=C LANG=C LANGUAGE=unset" in p.stderr, p.stderr)
p = run(["--agent-mode", "--no-stable-diagnostics"], ["-c", "x.c"], e)
check("without stable mode the locale is left alone", b"LC_ALL=de_DE.UTF-8" in p.stderr, p.stderr)
# 7. --stable-diagnostics alone: clang's text, deterministic order not applied to plain text, temp names fixed in JSON lines
p = run(["--stable-diagnostics"], ["-c", "errors3.c", "-o", "/dev/null"])
plain = subprocess.run([CLANG, "-fno-color-diagnostics", "-c", "errors3.c", "-o", "/dev/null"], cwd=FX, capture_output=True).stderr
check("--stable-diagnostics alone: clang's text unchanged", p.stderr == plain)
# 8. no timestamps: only elapsed_ms is a time field
a = run(["--agent-mode"], ["-c", "errors3.c", "-o", "/dev/null"])
check("no date or time of day in any record", not re.search(rb"\d{4}-\d\d-\d\d|\d\d:\d\d:\d\d", a.stderr.replace(b"errors3.c:", b"")), a.stderr[-200:])
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
