#!/usr/bin/env python3
"""agent-fixes 2-6: effective-flags size, unknown-type cascades, compile-time/why-not tidy-ups, bad RUSTY_AGENT_OUT, run-san paths,
locations in step with notes, quiet EPIPE. Every check runs the real tool and parses its JSON."""
import json, os, shutil, subprocess, sys, tempfile
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures")
base = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:700]))
def run(args, detail="normal", extra=None, cwd=FX, flags=("--agent-mode",)):
    e = dict(base, RUSTY_AGENT_DETAIL=detail); e.update(extra or {})
    p = subprocess.run([RCC] + list(flags) + args, cwd=cwd, capture_output=True, env=e)
    return [json.loads(l) for l in p.stderr.decode().splitlines() if l.startswith('{"schema"')], p
def of(r, t): return [x for x in r if x["type"] == t]
def size(x): return len(json.dumps(x, separators=(",", ":")))

# ---- 2. effective-flags: a count plus user macros at normal, everything at full
rn, _ = run(["--effective-flags", "-DFOO=1", "-DBAR", "-c", "clean.c", "-o", "/dev/null"]); n = of(rn, "effective-flags")[0]
rf, _ = run(["--effective-flags", "-DFOO=1", "-DBAR", "-c", "clean.c", "-o", "/dev/null"], "full"); f = of(rf, "effective-flags")[0]
check("effective normal: user defines listed, predefined only counted", n["defines"]["user"] == ["FOO=1", "BAR"] and n["defines"]["predefined_count"] > 100 and "all" not in n["defines"], n["defines"])
check("effective full: every predefined macro, count matches", len(f["defines"]["all"]) == f["defines"]["predefined_count"] == n["defines"]["predefined_count"] and "__clang__" in f["defines"]["all"])
check("effective normal is under 3 KB (was 25 KB), full is not", size(n) < 3000 < size(f), (size(n), size(f)))
rb, _ = run(["--effective-flags", "-c", "clean.c", "-o", "/dev/null"], "brief"); b = of(rb, "effective-flags")[0]
check("effective brief: triple, cpu, language, count; under 700 bytes", b["target"]["triple"] and "include_dirs" not in b and size(b) < 700, size(b))
check("normal keeps what answers 'what flags': target, include search, linker, language", n["include_dirs"]["search"] and n["linker"]["path"] and n["language"]["x"] == "c" and n["clang"]["version"])
check("normal: include dirs are tidy (no /../)", all("/../" not in d for d in n["include_dirs"]["search"] + n["include_dirs"].get("system", [])) and "/../" not in n["clang"].get("gcc_installation", ""), n["include_dirs"])
check("normal: nothing said twice (system dirs that are in search are not repeated)", not set(n["include_dirs"].get("system", [])) & set(n["include_dirs"]["search"]))

# ---- 3. root cause: declarations with an unknown type
r, _ = run(["-fsyntax-only", "unknown_type.c"])
d = of(r, "diagnostic")
roots = [x for x in d]
check("unknown_type: roots are the unknown type and the genuinely undeclared 'y'", [x["message"] for x in roots] == ["unknown type name 'foo_t'", "use of undeclared identifier 'y'"], [x["message"] for x in roots])
fo = d[0]["follow_on"]
check("x and z (declared as 'foo_t x', 'foo_t z') are followers under the root, rule declared-with-unknown-type", {(f["message"], f["rule"]) for f in fo} >= {("use of undeclared identifier 'x'", "declared-with-unknown-type"), ("use of undeclared identifier 'z'", "declared-with-unknown-type")}, fo)
check("the summary still counts all 6 errors and 4 suppressed", of(r, "summary")[0]["diagnostics"] == {"errors": 6, "suppressed": 4}, of(r, "summary")[0]["diagnostics"])
r2, _ = run(["-fsyntax-only", "unknown_type.c"], "full")
check("full: same grouping", [x["message"] for x in of(r2, "diagnostic")] == [x["message"] for x in roots])
r3, _ = run(["-fsyntax-only", "cascade_names.c"])
check("a name not declared with the unknown type is not folded (cascade_names: 'missing' stays its own root)", any("'missing'" in x["message"] for x in of(r3, "diagnostic")))

# ---- 4. compile-time and why-not
r, _ = run(["--compile-time-report", "-std=c++20", "-c", "ct.cpp", "-o", "/dev/null"]); ct = of(r, "compile-time")[0]
check("compile-time: header paths are tidy", ct["slowest"]["headers"] and all("/../" not in h["path"] and "/./" not in h["path"] for h in ct["slowest"]["headers"]), [h["path"] for h in ct["slowest"]["headers"]])
check("compile-time: tidy header paths still exist", all(os.path.exists(h["path"]) for h in ct["slowest"]["headers"]), [h["path"] for h in ct["slowest"]["headers"]])
check("compile-time: function names are demangled", ct["slowest"]["functions"] and all(not f["name"].startswith("_Z") for f in ct["slowest"]["functions"]) and any("(" in f["name"] or "::" in f["name"] for f in ct["slowest"]["functions"]), [f["name"] for f in ct["slowest"]["functions"]])
rf, _ = run(["--compile-time-report", "-std=c++20", "-c", "ct.cpp", "-o", "/dev/null"], "full"); ctf = of(rf, "compile-time")[0]
check("compile-time full: paths tidy and names demangled there too", all("/../" not in h["path"] for h in ctf["slowest"]["headers"]) and all(not f["name"].startswith("_Z") for f in ctf["slowest"]["functions"]))
O = ["-O2", "-g", "-c", "opt.c", "-o", "/dev/null"]
wn = lambda what, fn, detail="normal": of(run(["--why-not-%s=%s" % (what, fn)] + O, detail)[0], "why-not")[0]
w = wn("vectorized", "nov")
check("why-not grammar: '1 loop in 'nov' was not vectorized' (no 'loop(s)', no 'were 0')", w["summary"] == "1 loop in 'nov' was not vectorized", w["summary"])
w = wn("vectorized", "vec")
check("why-not grammar: optimized loop", "(s)" not in w["summary"] and w["outcome"] == "optimized", w["summary"])
w = wn("inlined", "never")
check("why-not grammar: no '(s)' anywhere", "(s)" not in w["summary"], w["summary"])
for detail in ("normal", "full"):
    r, p = run(["--why-not-inlined=big", "--why-not-vectorized=nov", "--compile-time-report"] + O, detail)
    txt = p.stderr.decode()
    check(f"{detail}: no private temp path (rusty-cc-PID, opt.yaml, trace dir) in any record", "rusty-cc-" not in txt and "/tmp/" not in txt and (detail == "full" or "opt.yaml" not in txt), [l for l in txt.splitlines() if "rusty-cc-" in l][:1])
r, _ = run(["--why-not-vectorized=nov"] + O, "full"); s = of(r, "summary")[0]
check("full summary command names the private temp dir, not its path", any("<private temp dir>" in a for a in s["command"]) and not any(a.startswith("/tmp") or "rusty-cc-" in a for a in s["command"]), s["command"])

# ---- 5. a bad RUSTY_AGENT_OUT is a record; locations follow notes; run-san paths
tmp = tempfile.mkdtemp()
for val in ("", tmp + "/nodir/x.jsonl"):
    r, p = run(["-c", "errors3.c", "-o", "/dev/null"], extra={"RUSTY_AGENT_OUT": val})
    lines = p.stderr.decode().splitlines()
    c0500 = [x for x in r if x.get("code") == "C0500"]
    check(f"bad RUSTY_AGENT_OUT {val!r:.12}: every stderr line is a JSON record, one C0500 warning that names the variable", all(l.startswith("{") for l in lines) and len(c0500) == 1 and "RUSTY_AGENT_OUT" in c0500[0]["message"] and c0500[0]["severity"] == "warning", p.stderr[:300])
    check("  ... and the compile result is unchanged (exit 1, 4 errors in the summary)", p.returncode == 1 and of(r, "summary")[0]["diagnostics"]["errors"] == 4)
r, p = run(["-c", "errors3.c", "-o", "/dev/null"], extra={"RUSTY_AGENT_OUT": tmp + "/ok.jsonl"})
check("a good RUSTY_AGENT_OUT: no C0500", not [x for x in r if x.get("code") == "C0500"] and len(open(tmp + "/ok.jsonl").read().splitlines()) == len(r))

rf, _ = run(["-fsyntax-only", "deep.cpp"], "full")
for x in of(rf, "diagnostic"):
    nl = [l for l in x["locations"] if l["role"] == "note"]
    shown = [n for n in x["notes"] if not n.startswith("... ")]
    with_place = [n for n in shown if n.startswith("deep.cpp:") or n.startswith("/")]
    check(f"locations in step with notes ({x['message'][:40]}): one note location per shown note with a place", len(nl) == len(with_place), (len(nl), len(with_place), x["notes"]))
    for l in nl:
        check("  note location is named by a note", any(n.startswith(f"{l['file']}:{l['line']}:{l['column']}: ") for n in shown), (l, x["notes"]))
n_cut = [x for x in of(rf, "diagnostic") if x.get("notes_omitted")]
check("deep.cpp has a diagnostic with a truncated chain (the test means something)", len(n_cut) >= 1)

# run-san: file names relative to the current directory when under it, however the program was compiled
d = tempfile.mkdtemp(); shutil.copy(FX + "/san_heap.c", d + "/san_heap.c")
for how, src in (("relative", "san_heap.c"), ("absolute", d + "/san_heap.c")):
    subprocess.run(["/usr/bin/clang", "-g", "-fsanitize=address", "-fno-omit-frame-pointer", src, "-o", d + "/prog_" + how], cwd=d, check=True)
    r, p = run(["run-san", "--", "./prog_" + how], cwd=d, flags=())
    x = of(r, "diagnostic")[0]
    files = [l["file"] for l in x["locations"]] + [n.split("(")[-1].split(":")[0] for n in x["notes"] if "(" in n]
    check(f"run-san ({how} compile): file names are relative (san_heap.c)", files and all(f == "san_heap.c" for f in files), files)
    rf, _ = run(["run-san", "--", "./prog_" + how], "full", cwd=d, flags=())
    fr = [fr["file"] for st in of(rf, "diagnostic")[0]["sanitizer"]["stacks"] for fr in st["frames"] if fr["file"]]
    check(f"run-san full ({how}): frame files relative too", fr and all(f == "san_heap.c" for f in fr), fr)
# a program outside the current directory keeps its (tidy) absolute names
r, p = run(["run-san", "--", d + "/prog_absolute"], cwd='/usr', flags=())
check("run-san from another directory: absolute names stay absolute", all(l["file"].startswith("/") for l in of(r, "diagnostic")[0]["locations"]), of(r, "diagnostic")[0]["locations"])

# ---- 6. EPIPE
for args in (["--agent-help"], ["--agent-mode", "--version"]):
    pr = subprocess.Popen([RCC] + args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=base)
    head = pr.stdout.read(10); pr.stdout.close(); err = pr.stderr.read(); rc = pr.wait()
    check(f"{' '.join(args)} | head -c 10: nothing on stderr, no panic, exit 0", err == b"" and rc == 0 and len(head) == 10, (rc, err[:200]))
p = subprocess.run(["sh", "-c", f"'{RCC}' --agent-help | head -c 7"], capture_output=True, env=base)
check("through a real shell pipe: 7 bytes, empty stderr", p.stdout == b'{"schem' and p.stderr == b"", (p.stdout, p.stderr))
full = subprocess.run([RCC, "--agent-help"], capture_output=True, env=base)
check("--agent-help to a file/pipe that stays open still prints one complete JSON object", json.loads(full.stdout)["tool"] == "rusty-cc" and full.returncode == 0)
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
