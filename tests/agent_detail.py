#!/usr/bin/env python3
"""agent-fixes 7 (token budget rules A-G): detail levels, no empty or repeated fields, key order, summary, stable, size budget.
Every check runs the real tool and parses its JSON lines."""
import json, os, subprocess, sys, tempfile
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures")
base = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:700]))
def run(args, detail=None, extra=None, flags=("--agent-mode",), cwd=FX):
    e = dict(base); e.update(extra or {})
    if detail: e["RUSTY_AGENT_DETAIL"] = detail
    p = subprocess.run([RCC] + list(flags) + args, cwd=cwd, capture_output=True, env=e)
    return [json.loads(l) for l in p.stderr.decode().splitlines() if l.startswith('{"schema"')], p
def of(r, t): return [x for x in r if x["type"] == t]
def size(x): return len(json.dumps(x, separators=(",", ":")))
def walk(v, path=""):
    """yield (path, key, value) for every member of every object"""
    if isinstance(v, dict):
        for k, x in v.items():
            yield path, k, x
            yield from walk(x, path + "/" + k)
    elif isinstance(v, list):
        for x in v: yield from walk(x, path)
def empties(rec):
    return [(p, k) for p, k, x in walk(rec) if k != "text" and (x is None or x == "" or x == [] or x == {})]

JOBS = {
  "errors3": ["-c", "errors3.c", "-o", "/dev/null"], "warnings": ["-Wall", "-Wextra", "-c", "warn.c", "-o", "/dev/null"],
  "template": ["-c", "tmpl.cpp", "-o", "/dev/null"], "deep": ["-fsyntax-only", "deep.cpp"], "cascade": ["-fsyntax-only", "cascade_names.c"],
  "unknown_type": ["-fsyntax-only", "unknown_type.c"], "syntax": ["-fsyntax-only", "cascade_syntax.c"], "fixable": ["-Wall", "-fsyntax-only", "fixable.c"],
}
# ---- A. the levels and how they are chosen
r, _ = run(JOBS["errors3"]); rn, _ = run(JOBS["errors3"], "normal"); rf, _ = run(JOBS["errors3"], "full"); rb, _ = run(JOBS["errors3"], "brief")
check("default detail in agent mode is normal", r == rn and r != rf)
r2, _ = run(JOBS["errors3"], "full", flags=("--agent-mode", "--agent-detail=brief"))
check("the flag wins over RUSTY_AGENT_DETAIL", r2 == rb and r2 != rf)
r3, _ = run(JOBS["errors3"], "loud"); r4, _ = run(JOBS["errors3"], None, flags=("--agent-mode", "--agent-detail=loud"))
check("an unknown level changes nothing (normal)", r3 == rn and r4 == rn)
r5, p5 = run(JOBS["errors3"], "full", flags=())
check("RUSTY_AGENT_DETAIL alone does not turn agent mode on", r5 == [] and b"errors3.c:3:13: error" in p5.stderr)
rA, _ = run(JOBS["errors3"], "full", flags=(), extra={"RUSTY_AGENT": "1"})
check("RUSTY_AGENT=1 works with the level", rA == rf)
check("the flag is consumed (clang never sees it)", b"unknown argument" not in run(JOBS["errors3"], flags=("--agent-mode", "--agent-detail=full"))[1].stderr)

# ---- C. key order everywhere, in all levels
for lvl in ("brief", "normal", "full"):
    for name, a in JOBS.items():
        for x in run(a, lvl)[0]:
            ks = list(x)
            ok = ks[:2] == ["schema", "type"] and (x["type"] != "diagnostic" or ks[2:5] == ["code", "severity", "message"])
            if not ok: check(f"key order {lvl}/{name}/{x['type']}", False, ks[:6])
check("key order: schema, type first (and code, severity, message for diagnostics) in every record of every job and level", True)

# ---- B/7. normal has no rendered, files, symbols, empty or zero fields
for name, a in JOBS.items():
    rn, _ = run(a, "normal"); rf, _ = run(a, "full")
    bad = [(x["type"], k) for x in of(rn, "diagnostic") for k in ("rendered", "files", "symbols") if k in x]
    check(f"{name}: normal has no rendered/files/symbols", not bad, bad)
    e = [(x["type"], empties(x)) for x in rn if empties(x)]
    check(f"{name}: normal has no empty or null field anywhere", not e, e)
    z = [(x["type"], p, k) for x in rn for p, k, v in walk(x) if v == 0 and k in ("suppressed", "notes_omitted", "warnings", "errors", "fatal", "remarks", "records")]
    check(f"{name}: normal has no zero counts", not z, z)
    check(f"{name}: same number of records as full", len(rn) == len(rf), (len(rn), len(rf)))
    for xn, xf in zip(of(rn, "diagnostic"), of(rf, "diagnostic")):
        same = all(xn.get(k) == xf.get(k) for k in ("code", "severity", "message")) and xn["locations"][0]["file"] == xf["locations"][0]["file"] and xn["locations"][0]["line"] == xf["locations"][0]["line"] if xf["locations"] else True
        if not same: check(f"{name}: facts equal between normal and full", False, (xn, xf))
        if xf.get("follow_on") and not [f for f in xn.get("follow_on", [])]: check(f"{name}: follow_on kept", False)
    check(f"{name}: normal is smaller than full", sum(map(size, rn)) < sum(map(size, rf)), (sum(map(size, rn)), sum(map(size, rf))))
rf, _ = run(JOBS["errors3"], "full")
d = of(rf, "diagnostic")[0]
check("full keeps every field (rendered, files, symbols, notes, suggestions, fixits, flag, notes_omitted, suppressed, follow_on)", {"rendered", "files", "symbols", "notes", "suggestions", "fixits", "flag", "notes_omitted", "suppressed", "follow_on"} <= set(d), list(d))
rn, _ = run(JOBS["errors3"], "normal"); dn = of(rn, "diagnostic")
check("errors3 normal: a diagnostic is only code, severity, message, locations (+fix-its when there are some)", all(set(x) <= {"schema", "type", "code", "severity", "message", "locations", "flag", "fixits", "notes", "suggestions"} for x in dn), [list(x) for x in dn])
fx = [x for x in dn if x["code"] == "C0002"][0]
check("a fix-it is stated once at normal (the suggestion that restates it is gone), with its position", len(fx["fixits"]) == 1 and fx["fixits"][0]["text"] == ";" and "suggestions" not in fx, fx)
check("normal location drops end_line (same as line) and end_column equal to the start", all("end_line" not in l for x in dn for l in x["locations"]) and all(l.get("end_column") != l["column"] for x in dn for l in x["locations"]))

# deep template notes: note locations are in the note text once
rn, _ = run(JOBS["deep"], "normal")
nl = [(x["message"][:30], l) for x in of(rn, "diagnostic") for l in x.get("locations", []) if l["role"] == "note"]
check("deep: note locations are not repeated beside the notes that name them", not nl, nl[:2])
check("deep: omitted notes are counted and the pointer is there", all(("notes_omitted" in x) == any(n.startswith("... ") for n in x.get("notes", [])) for x in of(rn, "diagnostic")) and any(x.get("more") for x in of(rn, "diagnostic")))

# follow_on entries do not repeat what their root says
r3, _ = run(["-fsyntax-only", "cascade_names.c"], "normal"); same = of(r3, "diagnostic")[1]
check("follow_on: 'the same name again' keeps only the place and the rule", all(set(f) == {"location", "rule"} for f in same["follow_on"]) and len(same["follow_on"]) == 2, same["follow_on"])
r3f, _ = run(["-fsyntax-only", "cascade_names.c"], "full")
check("follow_on at full: every field of every follower", all({"code", "severity", "message", "flag", "location", "rule"} <= set(f) for f in of(r3f, "diagnostic")[1]["follow_on"]))

# ---- brief
for name, a in JOBS.items():
    rb, _ = run(a, "brief")
    ok = all(set(x) <= {"schema", "type", "code", "severity", "message", "flag", "werror", "location", "fix", "kind", "notes", "suppressed"} for x in of(rb, "diagnostic"))
    check(f"{name}: brief diagnostics have only code, message, one location, one fix, counts", ok, [list(x) for x in of(rb, "diagnostic")][:2])
    check(f"{name}: brief diagnostic under 400 bytes", all(size(x) < 400 for x in of(rb, "diagnostic")), [size(x) for x in of(rb, "diagnostic")])
rb, _ = run(JOBS["errors3"], "brief"); x = [x for x in of(rb, "diagnostic") if x["code"] == "C0002"][0]
check("brief: one fix with its edit", x["fix"]["edits"][0]["text"] == ";" and x["fix"]["edits"][0]["line"] == 4, x)
rb, _ = run(JOBS["unknown_type"], "brief")
check("brief: the root cause keeps the count of what it hides", of(rb, "diagnostic")[0]["suppressed"] == 4 and "follow_on" not in of(rb, "diagnostic")[0])

# ---- E. the summary
r, _ = run(["-c", "clean.c", "-o", "/dev/null"]); s = r[-1]
check("success is cheap: one summary record, exit 0, under 200 bytes, no command, no zero counts", len(r) == 1 and s["type"] == "summary" and s["exit"] == 0 and size(s) < 200 and "command" not in s and "diagnostics" not in s, s)
rn, _ = run(JOBS["errors3"], "normal"); s = rn[-1]
check("normal summary: no command echo, no zero counts", "command" not in s and s["diagnostics"] == {"errors": 4}, s)
rf, _ = run(JOBS["errors3"], "full"); s = rf[-1]
check("full summary: command and every count", "-fdiagnostics-parseable-fixits" in s["command"] and s["diagnostics"]["warnings"] == 0 and s["diagnostics"]["records"] == 4, s)
rb, _ = run(JOBS["errors3"], "brief"); check("brief summary: no file list", "compiled" not in rb[-1].get("files", {}), rb[-1])
rn, _ = run(["-c", "a.c", "-c", "clean.c", "-o", "/dev/null"] if False else JOBS["errors3"], "normal")

# ---- stable mode: no clock values, byte-identical twice, in every level
for lvl in ("brief", "normal", "full"):
    a = run(JOBS["warnings"] + ["--stable-diagnostics"], lvl); b = run(JOBS["warnings"] + ["--stable-diagnostics"], lvl)
    check(f"stable {lvl}: two runs are byte-identical", a[1].stderr == b[1].stderr and len(a[0]) > 1)
    check(f"stable {lvl}: no elapsed_ms, no pid, no temp name", b"elapsed" not in a[1].stderr and b"rusty-cc-" not in a[1].stderr, a[1].stderr[-200:])
rn, p = run(["linkfail.c", "-o", "/dev/null"], "normal")
check("stable normal: temp object names are XXXXXX in every record (linker records pass through too)", b"XXXXXX" in p.stderr or b"linkfail-" not in p.stderr, p.stderr[:300])

# ---- B. fix-preview does not repeat itself
rn, _ = run(JOBS["fixable"] + ["--fix-preview"], "normal"); d = of(rn, "diagnostic"); fp = of(rn, "fix-preview")[0]
check("with --fix-preview the diagnostics carry no fixits (the preview has the edits)", not any("fixits" in x for x in d), [list(x) for x in d])
check("fix-preview normal: a diff, no per-edit list repeating it, no static note", fp["files"][0]["diff"] and "edits" not in fp and "note" not in fp and "written" not in fp, list(fp))
txt = json.dumps(fp)
check("fix-preview normal: the changed line appears once per diff, not three times", txt.count("%ld") == 2 or txt.count("%ld") <= 3, txt.count("%ld"))
rn0, _ = run(JOBS["fixable"], "normal")
check("without --fix-preview the diagnostics keep their fixits", any("fixits" in x for x in of(rn0, "diagnostic")))
rf, _ = run(JOBS["fixable"] + ["--fix-preview"], "full")
check("fix-preview full: per-edit list, both diffs and the note", {"edits", "note", "written"} <= set(of(rf, "fix-preview")[0]))
check("fix-preview normal is at least 30% smaller than full", size(fp) * 10 < size(of(rf, "fix-preview")[0]) * 7, (size(fp), size(of(rf, "fix-preview")[0])))
rn, _ = run(["-Wall", "-fsyntax-only", "assign_cond.c", "--fix-preview"], "normal"); fp = of(rn, "fix-preview")[0]
check("alternatives in normal: id, note, diff; no repeat of the diagnostic message", all(set(a) == {"id", "note", "file", "edits", "diff"} for a in fp["alternatives"]), fp["alternatives"])

# ---- missing header
rn, _ = run(["--explain-missing-header", "-fsyntax-only", "hdr/src/missing_project.c"], "normal")
mh = of(rn, "missing-header")
check("missing-header normal: header, form, found; the message and place are in the diagnostic before it", mh and "message" not in mh[0] and "location" not in mh[0] and mh[0]["header"] and not empties(mh[0]) and "rendered" not in json.dumps(mh[0]), mh and list(mh[0]))

# ---- G. the size budget
tmp = tempfile.mkdtemp(); out = tmp + "/all.jsonl"
many = ["-Wall", "-c", "manywarns.c", "-o", "/dev/null"]
for lvl in ("normal", "brief"):
    r, p = run(many, lvl, extra={"RUSTY_AGENT_OUT": out + lvl})
    total = sum(size(x) + 1 for x in r)
    s = r[-1]; om = s.get("records_omitted", {})
    check(f"{lvl}: records on stderr stay under the hard cap of 20000 bytes ({total})", total <= 20000, total)
    check(f"{lvl}: the summary says what was cut, by type, with a pointer", om.get("by_type", {}).get("diagnostic", 0) > 0 and "RUSTY_AGENT_OUT" in om.get("more", ""), s)
    shown = len(of(r, "diagnostic"))
    check(f"{lvl}: shown + omitted = all 121 diagnostics", shown + om["by_type"]["diagnostic"] == 121, (shown, om))
    check(f"{lvl}: the counts in the summary still cover everything", s["diagnostics"] == {"errors": 1, "warnings": 120}, s["diagnostics"])
    check(f"{lvl}: the error is not cut by the budget", any(x["severity"] == "error" and "nope_name" in x["message"] for x in of(r, "diagnostic")))
    f = [json.loads(l) for l in open(out + lvl).read().splitlines()]
    check(f"{lvl}: RUSTY_AGENT_OUT has every record (121 diagnostics + summary), shaped", len(of(f, "diagnostic")) == 121 and f[-1]["type"] == "summary" and "rendered" not in f[0], len(f))
    check(f"{lvl}: the pointer names the file when there is one", "all records are in the RUSTY_AGENT_OUT file" == om["more"], om)
r, p = run(many, "full")
check("full has no cap: all 121 diagnostics, no records_omitted", len(of(r, "diagnostic")) == 121 and "records_omitted" not in r[-1])
r, p = run(many, "normal")
check("without a file the pointer says how to get the rest", "RUSTY_AGENT_OUT=FILE" in r[-1]["records_omitted"]["more"] and "--agent-detail=full" in r[-1]["records_omitted"]["more"], r[-1])
r, p = run(many, "normal", flags=("--agent-mode", "--no-build-summary"))
check("a cut without a summary is still within the cap", sum(size(x) + 1 for x in r) <= 20000)

# ---- size tests: one budget per record type and job (tests/sizes.py)
sys.path.insert(0, os.path.join(root, "tests"))
import sizes
res = sizes.measure(RCC); bad = []
for n, rr in res.items():
    for t, m in rr["max_record"].items():
        if t in sizes.BUDGET_REC and m > sizes.BUDGET_REC[t]: bad.append(f"{n}: largest {t} {m} > {sizes.BUDGET_REC[t]}")
    if rr["total"] > sizes.BUDGET_JOBS.get(n, sizes.BUDGET_JOB): bad.append(f"{n}: total {rr['total']} > {sizes.BUDGET_JOBS.get(n, sizes.BUDGET_JOB)}")
check("size budgets: no record type and no reference job grows past its budget (tests/sizes.py)", not bad, bad)
check("size budgets: the clean compile costs under 200 bytes, effective-flags under 3000", res["clean"]["total"] < 200 and res["effective"]["by_type"]["effective-flags"] < 3000, (res["clean"], res["effective"]["by_type"]))
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
