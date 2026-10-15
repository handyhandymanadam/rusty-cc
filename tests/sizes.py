#!/usr/bin/env python3
"""Record sizes in agent mode for the reference jobs (checklist item 4 / rule G). Runs the REAL tool, parses the JSON lines.
usage: sizes.py [BIN]            print bytes per job and per record type
       sizes.py --check [BIN]    also fail when a record type or a job grows past its budget (BUDGET_* below)
       DETAIL=full|brief|normal  picks the detail level (default normal)"""
import json, os, subprocess, sys, tempfile
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FX = os.path.join(root, "tests/fixtures")
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc")
O = "/dev/null"
# name: (extra flags, args)
JOBS = {
  "clean":        (["--agent-mode"], ["-c", "clean.c", "-o", O]),
  "errors3":      (["--agent-mode"], ["-c", "errors3.c", "-o", O]),
  "warnings":     (["--agent-mode"], ["-Wall", "-Wextra", "-c", "warn.c", "-o", O]),
  "template":     (["--agent-mode"], ["-c", "tmpl.cpp", "-o", O]),
  "cascade_names":(["--agent-mode"], ["-c", "cascade_names.c", "-o", O]),
  "unknown_type": (["--agent-mode"], ["-c", "unknown_type.c", "-o", O]),
  "deep_templates": (["--agent-mode"], ["-fsyntax-only", "deep.cpp"]),
  "manywarns":    (["--agent-mode", "-Wall"], ["-c", "manywarns.c", "-o", O]),
  "linkfail":     (["--agent-mode"], ["linkfail.c", "-o", O]),
  "fix_preview":  (["--agent-mode", "--fix-preview"], ["-Wall", "-fsyntax-only", "fixable.c"]),
  "fix_preview_assign": (["--agent-mode", "--fix-preview"], ["-Wall", "-fsyntax-only", "assign_cond.c"]),
  "fix_preview_mixed": (["--agent-mode", "--fix-preview"], ["-Wall", "-fsyntax-only", "mixed.c"]),
  "effective":    (["--agent-mode", "--effective-flags"], ["-c", "clean.c", "-o", O]),
  "why_not":      (["--agent-mode", "--why-not-inlined=big", "--why-not-vectorized=nov"], ["-O2", "-g", "-c", "opt.c", "-o", O]),
  "compile_time": (["--agent-mode", "--compile-time-report"], ["-std=c++20", "-c", "ct.cpp", "-o", O]),
}
# budgets in bytes: per record type (largest single record) and per job (all records)
BUDGET_REC = {"diagnostic": 1500, "summary": 450, "fix-preview": 1200, "effective-flags": 1200, "why-not": 3000, "compile-time": 3000, "missing-header": 1500}
BUDGET_JOBS = {"manywarns": 20000, "deep_templates": 5000}   # 121 diagnostics: held to the hard cap by the size budget
BUDGET_JOB = 3500   # typical command (rule G says about 5 KB; the hard cap per command is 20000)
def records(text):
    out = []
    for l in text.splitlines():
        if l.startswith("{"):
            try: out.append(json.loads(l))
            except ValueError: pass
    return out
def measure(bin_, jobs=JOBS):
    env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}
    if os.environ.get("DETAIL"): env["RUSTY_AGENT_DETAIL"] = os.environ["DETAIL"]
    res = {}
    for name, (flags, args) in jobs.items():
        p = subprocess.run([bin_] + flags + args, cwd=FX, capture_output=True, env=env)
        recs = records(p.stderr.decode("utf-8", "replace"))
        per = {}; mx = {}
        for r in recs:
            n = len(json.dumps(r, separators=(",", ":")))
            per[r["type"]] = per.get(r["type"], 0) + n; mx[r["type"]] = max(mx.get(r["type"], 0), n)
        res[name] = {"total": sum(per.values()), "by_type": per, "max_record": mx, "records": len(recs), "stderr_bytes": len(p.stderr)}
    return res
if __name__ == "__main__":
    a = [x for x in sys.argv[1:] if x != "--check"]; bin_ = a[0] if a else RCC
    res = measure(bin_); bad = 0
    for n, r in res.items():
        print(f"{n:20} total {r['total']:>6} records {r['records']:>3}  " + " ".join(f"{t}={b}" for t, b in r["by_type"].items()))
        if "--check" in sys.argv:
            for t, m in r["max_record"].items():
                if t in BUDGET_REC and m > BUDGET_REC[t]: print(f"  FAIL {n}: largest {t} record {m} > {BUDGET_REC[t]}"); bad += 1
            if r["total"] > BUDGET_JOBS.get(n, BUDGET_JOB): print(f"  FAIL {n}: total {r['total']} > {BUDGET_JOBS.get(n, BUDGET_JOB)}"); bad += 1
    sys.exit(1 if bad else 0)
