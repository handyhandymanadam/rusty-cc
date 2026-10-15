#!/usr/bin/env python3
"""Feature 3: --fix-preview shows machine fix-its as unified diffs, never writes, safe/unsafe as clang does."""
import hashlib, json, os, shutil, subprocess, sys, tempfile
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures"); CLANG = "/usr/bin/clang"
env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}; env["RUSTY_AGENT_DETAIL"] = "full"
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:500]))
def sha(f): return hashlib.sha256(open(f, "rb").read()).hexdigest()
def hashes(d): return {f: sha(os.path.join(d, f)) for f in sorted(os.listdir(d)) if os.path.isfile(os.path.join(d, f))}
def fp(flags, src, cwd=FX):
    p = subprocess.run([RCC] + flags + ["-Wall", "-fsyntax-only", src], cwd=cwd, capture_output=True, env=env)
    return [d for d in (json.loads(l) for l in p.stderr.decode().splitlines() if l.startswith('{"schema"')) if d["type"] != "summary"], p
def apply_diff(d, rel, diff):
    open(os.path.join(d, "x.diff"), "w").write(diff)
    return subprocess.run(["patch", "-p1", "-s", "-i", "x.diff"], cwd=d, capture_output=True)

before = hashes(FX)
r, p = fp(["--fix-preview"], "fixable.c")
check("fixable: exit code like clang (errors present -> 1)", p.returncode == 1)
fv = [x for x in r if x["type"] == "fix-preview"]
check("one fix-preview record, dry_run, nothing written", len(fv) == 1 and fv[0]["dry_run"] and fv[0]["written"] is False and fv[0]["schema"] == 1)
fv = fv[0]; f = fv["files"][0]
check("fixable: safe edits counted (format, semicolon, ->), unsafe (parentheses) counted", f["safe_edits"] == 3 and f["unsafe_edits"] == 2, f)
check("fixable: safe/unsafe flags per edit", sorted({(e["safety"]) for e in fv["edits"]}) == ["safe", "unsafe"] and sum(e["safety"] == "unsafe" for e in fv["edits"]) == 2)
check("fixable: unsafe ones belong to the parentheses note", all("'&&'" in e["diagnostic"] or "parentheses" in e["diagnostic"] for e in fv["edits"] if e["safety"] == "unsafe"), fv["edits"])
# the safe diff must equal what clang's own -fixit does on a copy
d = tempfile.mkdtemp(); shutil.copy(os.path.join(FX, "fixable.c"), d + "/fixable.c"); shutil.copy(d + "/fixable.c", d + "/orig.c")
subprocess.run([CLANG, "-Xclang", "-fixit", "-Wall", "-fsyntax-only", "fixable.c"], cwd=d, capture_output=True)
clang_fixed = open(d + "/fixable.c").read()
d2 = tempfile.mkdtemp(); shutil.copy(os.path.join(FX, "fixable.c"), d2 + "/fixable.c")
res = apply_diff(d2, "fixable.c", f["diff"])
check("patch applies cleanly", res.returncode == 0, res.stderr)
check("our safe diff gives exactly clang -fixit's result", open(d2 + "/fixable.c").read() == clang_fixed)
check("diff_with_unsafe present and different (adds parentheses)", "diff_with_unsafe" in f and "(b && c)" in f["diff_with_unsafe"] and "(b && c)" not in f["diff"], f.get("diff_with_unsafe"))
check("diff has a/ b/ headers", f["diff"].startswith("--- a/fixable.c\n+++ b/fixable.c\n"), f["diff"][:60])
check("source files unchanged after the run", hashes(FX) == before)

# mixed: clang -fixit applies nothing (unfixable error) but the preview still shows the fixable ones
r, p = fp(["--fix-preview"], "mixed.c"); fv = [x for x in r if x["type"] == "fix-preview"][0]
d3 = tempfile.mkdtemp(); shutil.copy(os.path.join(FX, "mixed.c"), d3 + "/mixed.c")
subprocess.run([CLANG, "-Xclang", "-fixit", "-Wall", "-fsyntax-only", "mixed.c"], cwd=d3, capture_output=True)
check("mixed: clang -fixit alone writes nothing here, preview still has safe edits", open(d3 + "/mixed.c").read() == open(os.path.join(FX, "mixed.c")).read() and fv["files"][0]["safe_edits"] == 3, fv["files"][0]["safe_edits"])
# alone: diagnostics stay as clang's text; with agent mode both records
check("--fix-preview alone: clang text printed as before", subprocess.run([RCC, "--fix-preview", "-Wall", "-fsyntax-only", "fixable.c"], cwd=FX, capture_output=True, env=env).stderr.decode().startswith("fixable.c:3:16: warning: '&&' within '||'"))
txt = subprocess.run([RCC, "--fix-preview", "-Wall", "-fsyntax-only", "fixable.c"], cwd=FX, capture_output=True, env=env).stderr.decode()
plain = subprocess.run([CLANG, "-fno-color-diagnostics", "-Wall", "-fsyntax-only", "fixable.c"], cwd=FX, capture_output=True).stderr.decode()
check("--fix-preview alone: text equals plain clang plus the record line", txt.startswith(plain) and txt[len(plain):].startswith('{"schema":1,"type":"fix-preview"'), txt[len(plain):][:80])
r, p = fp(["--agent-mode", "--fix-preview"], "fixable.c")
check("agent mode + --fix-preview: diagnostics records then the preview last", r[-1]["type"] == "fix-preview" and all(x["type"] == "diagnostic" for x in r[:-1]) and len(r) >= 3)
r, p = fp(["--agent-mode"], "fixable.c")
check("agent mode alone has no fix-preview record", all(x["type"] != "fix-preview" for x in r))
# clean: no record
r, p = fp(["--fix-preview"], "clean.c")
check("clean file: no record", r == [] and p.returncode == 0)
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
