#!/usr/bin/env python3
"""Feature 2: root cause first, follow-ons grouped, long backtraces compressed. Real compiles."""
import json, os, subprocess, sys
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures"); CLANG = "/usr/bin/clang"
env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}; env["RUSTY_AGENT_DETAIL"] = "full"
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:400]))
def recs(flags, src, extra=("-fsyntax-only",)):
    p = subprocess.run([RCC] + flags + list(extra) + [src], cwd=FX, capture_output=True, env=env)
    return [d for d in (json.loads(l) for l in p.stderr.decode().splitlines() if l.startswith('{"schema"')) if d["type"] != "summary"], p
def plain_count(src):
    p = subprocess.run([CLANG, "-fsyntax-only", "-fno-color-diagnostics", src], cwd=FX, capture_output=True)
    return p.stderr.decode().count(": error:") + p.stderr.decode().count(": warning:")

# names: 6 clang errors become 3 records (root: unknown type, undeclared f is grouped, 'missing' x3 -> 1)
r, p = recs(["--agent-mode"], "cascade_names.c")
total = len(r) + sum(x["suppressed"] for x in r)
check("names: records + suppressed equals the number of clang errors", total == plain_count("cascade_names.c") == 6, (len(r), total))
check("names: first record is the unknown type, the root cause", r[0]["code"] == "C0015" and "Foo" in r[0]["message"], r[0]["message"])
check("names: Foo declaration cascade grouped under it", {f["rule"] for f in r[0]["follow_on"]} == {"mentions-undeclared-name", "same-line-as-unknown-type"}, r[0]["follow_on"])
miss = [x for x in r if x["symbols"] == ["missing"]]
check("names: 'missing' reported once with 2 repeats", len(miss) == 1 and miss[0]["suppressed"] == 2 and all(f["rule"] == "same-error-repeated" for f in miss[0]["follow_on"]), miss)
check("names: follow-ons carry location and code", all(f["location"]["line"] in (2, 5, 6) and f["code"] for x in r for f in x["follow_on"]), r)
check("names: exit code 1 as clang", p.returncode == 1)

# syntax
r, p = recs(["--agent-mode"], "cascade_syntax.c")
check("syntax: later syntax error grouped under the first", len(r) == 1 and r[0]["suppressed"] == 1 and r[0]["follow_on"][0]["rule"] == "syntax-error-nearby", r)
check("syntax: the note 'to match this' stays", r[0]["notes"] and "to match this" in r[0]["notes"][0], r[0]["notes"])

# templates and overloads
r, p = recs(["--agent-mode"], "deep.cpp", ("-fsyntax-only",))
over = [x for x in r if x["code"] == "C0008"][0]; sa = [x for x in r if x["code"] == "C0018"][0]  # stable mode sorts by place
check("deep: overload candidates cut to 3 plus a count", over["notes_omitted"] == 3 and len(over["notes"]) == 4 and "3 more notes omitted" in over["notes"][-1], over["notes"])
check("deep: full text stays in rendered", over["rendered"].count("note: candidate function not viable") == 6, over["rendered"].count("candidate"))
check("deep: instantiation chain keeps first 2 and last 2", sa["notes_omitted"] == 1 and "Fact<3>" in sa["notes"][0] and "Fact<4>" in sa["notes"][1] and "more notes omitted" in sa["notes"][2] and "Fact<7>" in sa["notes"][-1], sa["notes"])
check("deep: rendered has all 5 frames", sa["rendered"].count("in instantiation of") == 5)

# alone and off
r, _ = recs(["--root-cause"], "cascade_names.c")
check("--root-cause alone works (and turns JSON on)", len(r) == 2 and "suppressed" in r[0], len(r))
r, _ = recs(["--diagnostics-format=json"], "cascade_names.c")
check("without --root-cause nothing is grouped", len(r) == 6 and "follow_on" not in r[0], len(r))
r, _ = recs(["--agent-mode", "--no-root-cause"], "cascade_names.c")
check("--no-root-cause turns it off inside --agent-mode", len(r) == 6)
# plain fixtures: nothing grouped, field present
r, _ = recs(["--agent-mode"], "errors3.c")
check("errors3: 4 roots, 0 suppressed", len(r) == 4 and all(x["suppressed"] == 0 and x["follow_on"] == [] for x in r), len(r))
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
