#!/usr/bin/env python3
"""Feature 4: --explain-missing-header on real failing includes; every suggested option is tried for real."""
import json, os, subprocess, sys
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures/hdr"); CLANG = "/usr/bin/clang"
env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}; env["RUSTY_AGENT_DETAIL"] = "full"
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:500]))
def run(flags, src):
    p = subprocess.run([RCC] + flags + ["-fsyntax-only", src], cwd=FX, capture_output=True, env=env)
    return [d for d in (json.loads(l) for l in p.stderr.decode().splitlines() if l.startswith('{"schema"')) if d["type"] != "summary"], p
def works(opt, src):
    return subprocess.run([CLANG, opt, "-fsyntax-only", src], cwd=FX, capture_output=True).returncode == 0

r, p = run(["--explain-missing-header"], "src/missing_project.c")
mh = [x for x in r if x["type"] == "missing-header"]
check("project: exit code 1 as clang", p.returncode == 1)
check("project: one missing-header record, shared fields", len(mh) == 1 and mh[0]["schema"] == 1 and mh[0]["code"] == "C0003" and mh[0]["header"] == "util.h" and mh[0]["form"] == "quote", mh)
m = mh[0]
check("project: searched dirs include /usr/include", "/usr/include" in m["searched"], m["searched"])
check("project: found in the project with -Iinc", m["found"][0]["kind"] == "project" and m["found"][0]["option"] == "-Iinc", m["found"])
check("project: first suggestion has the exact option", m["suggestions"][0]["option"] == "-Iinc")
check("project: the suggested option really fixes the first include", subprocess.run([CLANG, "-Iinc", "-fsyntax-only", "src/missing_project.c"], cwd=FX, capture_output=True).returncode == 0)
check("project: location of the failing include", m["location"]["file"] == "src/missing_project.c" and m["location"]["line"] == 1, m["location"])

r, p = run(["--explain-missing-header"], "src/missing_pkg.c"); m = [x for x in r if x["type"] == "missing-header"][0]
check("pkg: Python.h angle form", m["form"] == "angle" and m["header"] == "Python.h")
check("pkg: pkg-config package found with its cflags", any(pk["name"].startswith("python") and "-I/usr/include/python3" in pk["cflags"] for pk in m["packages"]), m["packages"])
check("pkg: suggested -I compiles", all(works(s["option"], "src/missing_pkg.c") for s in m["suggestions"] if s["option"]), m["suggestions"])
check("pkg: rpm owner named for the system file", "rpm" in m["found"][0], m["found"][0])

r, p = run(["--explain-missing-header"], "src/missing_near.c"); m = [x for x in r if x["type"] == "missing-header"][0]
check("near: case difference ranks first", m["found"] == [] and m["near_misses"][0]["path"].endswith("inc/util.h") and m["near_misses"][0]["distance"] == 0, m["near_misses"])
check("near: its suggestion option works with the right spelling", subprocess.run([CLANG, "-Iinc", "-fsyntax-only", "-xc", "-"], cwd=FX, input=b'#include "util.h"\n', capture_output=True).returncode == 0)
r, p = run(["--explain-missing-header"], "src/missing_near2.c"); m = [x for x in r if x["type"] == "missing-header"][0]
check("near2: stdioo.h -> stdio.h first", m["near_misses"][0]["include_as"] == "stdio.h", m["near_misses"])

r, p = run(["--explain-missing-header"], "src/missing_angle.c"); m = [x for x in r if x["type"] == "missing-header"][0]
check("angle: suggests quotes for a header next to the including file", any('write #include "local.h"' in s["message"] for s in m["suggestions"]), m["suggestions"])

r, p = run(["--explain-missing-header"], "src/missing_none.c"); m = [x for x in r if x["type"] == "missing-header"][0]
check("none: says nothing found and names the dnf command, runs nothing", m["found"] == [] and m["near_misses"] == [] and "dnf provides" in m["suggestions"][0]["message"] and m["suggestions"][0]["option"] is None, m["suggestions"])

# alone: clang's text first, then the record; with --agent-mode the diagnostic record comes first
txt = subprocess.run([RCC, "--explain-missing-header", "-fsyntax-only", "src/missing_none.c"], cwd=FX, capture_output=True, env=env).stderr.decode()
plain = subprocess.run([CLANG, "-fno-color-diagnostics", "-fsyntax-only", "src/missing_none.c"], cwd=FX, capture_output=True).stderr.decode()
check("alone: text equals plain clang, then the record", txt.startswith(plain) and '"type":"missing-header"' in txt[len(plain):], txt[:200])
r, p = run(["--agent-mode", "--explain-missing-header"], "src/missing_none.c")
check("agent mode + flag: diagnostic C0003 then missing-header", [x["type"] for x in r] == ["diagnostic", "missing-header"] and r[0]["code"] == "C0003" and r[0]["severity"] == "fatal", [x["type"] for x in r])
r, p = run(["--agent-mode"], "src/missing_none.c")
check("agent mode alone now explains too (umbrella)", [x["type"] for x in r] == ["diagnostic", "missing-header"] and r[1]["suggestions"], [x["type"] for x in r])
r, p = run(["--agent-mode", "--no-explain-missing-header"], "src/missing_none.c")
check("--no-explain-missing-header turns it off", [x["type"] for x in r] == ["diagnostic"])
# overhead only paid on failure: a clean compile with the flag prints nothing
p = subprocess.run([RCC, "--explain-missing-header", "-fsyntax-only", "../clean.c"], cwd=FX, capture_output=True, env=env)
check("clean compile: no output", p.stderr == b"" and p.returncode == 0)
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
