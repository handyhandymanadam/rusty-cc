#!/usr/bin/env python3
"""agent-fixes 1: --fix-preview never merges mutually exclusive alternatives (a fix-it on a note is one alternative)."""
import json, os, shutil, subprocess, sys, tempfile
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures")
base = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:600]))
def recs(flags, src, detail="full", extra=()):
    e = dict(base, RUSTY_AGENT_DETAIL=detail)
    p = subprocess.run([RCC, "--agent-mode"] + flags + list(extra) + ["-Wall", "-fsyntax-only", src], cwd=FX, capture_output=True, env=e)
    return [json.loads(l) for l in p.stderr.decode().splitlines() if l.startswith('{"schema"')]
def of(r, t): return [x for x in r if x["type"] == t]

r = recs(["--fix-preview"], "assign_cond.c")
fv = of(r, "fix-preview")[0]; f = fv["files"][0]
check("assign_cond: no merged diff (diff_with_unsafe absent: two alternatives)", "diff_with_unsafe" not in f and f["diff"] == "", f)
alts = fv["alternatives"]
check("assign_cond: two alternatives listed separately", len(alts) == 2 and [a["id"] for a in alts] == [1, 2], alts)
check("alternative 1 = parentheses, alone: (v.a = 1)", "+    if ((v.a = 1)) return 1;" in alts[0]["diff"] and "==" not in alts[0]["diff"], alts[0]["diff"])
check("alternative 2 = '==', alone: v.a == 1, no double parentheses", "+    if (v.a == 1) return 1;" in alts[1]["diff"] and "((" not in alts[1]["diff"], alts[1]["diff"])
check("no diff anywhere contains the meaning-changing mix ((v.a == 1))", "((v.a == 1))" not in json.dumps(fv))
d = tempfile.mkdtemp()
for a in alts:
    shutil.copy(os.path.join(FX, "assign_cond.c"), d + "/assign_cond.c"); open(d + "/x.diff", "w").write(a["diff"])
    res = subprocess.run(["patch", "-p1", "-s", "-i", "x.diff"], cwd=d, capture_output=True)
    ok = subprocess.run(["/usr/bin/clang", "-Wall", "-Werror", "-fsyntax-only", d + "/assign_cond.c"], capture_output=True)
    check(f"alternative {a['id']} patches cleanly and the result compiles with -Wall -Werror", res.returncode == 0 and ok.returncode == 0, (res.stderr, ok.stderr))
check("edits carry their alternative id", sorted(e["alternative"] for e in fv["edits"]) == [1, 1, 2], fv["edits"])
check("the two edits of the parentheses alternative share an id", [e["text"] for e in fv["edits"] if e["alternative"] == 1] == ["(", ")"])
dg = of(r, "diagnostic")[0]
check("the diagnostic's fixits say which alternative they belong to", sorted(x["alternative"] for x in dg["fixits"]) == [1, 1, 2], dg["fixits"])

# a diagnostic with ONE alternative keeps working as before (fixable.c: parentheses around && in ||)
r = recs(["--fix-preview"], "fixable.c"); fv = of(r, "fix-preview")[0]; f = fv["files"][0]
check("fixable: one alternative is unambiguous, still in diff_with_unsafe", "(b && c)" in f["diff_with_unsafe"] and "(b && c)" not in f["diff"])
check("fixable: and also listed as an alternative", len(fv["alternatives"]) == 1 and "(b && c)" in fv["alternatives"][0]["diff"])
# normal detail: alternatives kept, redundant copies gone
r = recs(["--fix-preview"], "assign_cond.c", "normal"); fv = of(r, "fix-preview")[0]
check("normal: alternatives kept, no per-edit list repeating them", len(fv["alternatives"]) == 2 and "edits" not in fv, list(fv))
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
