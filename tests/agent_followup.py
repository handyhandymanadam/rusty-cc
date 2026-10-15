#!/usr/bin/env python3
"""MASTER follow-up: header explainer in the umbrella (bounded time) and the RUSTY_AGENT_OUT records file."""
import json, os, subprocess, sys, tempfile, time
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures"); CLANG = "/usr/bin/clang"
env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}; env["RUSTY_AGENT_DETAIL"] = "full"
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:500]))
def run(flags, args=(), e=env, cwd=FX): return subprocess.run([RCC] + list(flags) + list(args), cwd=cwd, capture_output=True, env=e)
def recs(b): return [json.loads(l) for l in b.decode().splitlines() if l.startswith('{"schema"')]
tmp = tempfile.mkdtemp()

# 1. the explainer is in the umbrella; time is bounded even from a huge folder
p = run(["--agent-mode", "-fsyntax-only", "hdr/src/missing_pkg.c"]); r = recs(p.stderr)
check("umbrella: C0003 diagnostic followed by a missing-header record with suggestions", [x["type"] for x in r] == ["diagnostic", "missing-header", "summary"] and r[0]["code"] == "C0003" and r[1]["suggestions"], [x["type"] for x in r])

src = tmp + "/m.c"; open(src, "w").write("#include <missing_thing.h>\n")
t = time.time(); p = run(["--agent-mode", "-fsyntax-only", src], cwd="/"); wall = time.time() - t; r = recs(p.stderr)
mh = [x for x in r if x["type"] == "missing-header"]
check("searching from / still answers in under 4 s with a record", wall < 4 and len(mh) == 1 and mh[0]["suggestions"], wall)
check("record says whether the search was cut short", "truncated" in mh[0] and isinstance(mh[0]["truncated"], bool), mh[0].keys())
check("a header that is simply missing gets the dnf hint in suggestions", any("dnf provides" in s["message"] for s in mh[0]["suggestions"]) or mh[0]["truncated"], mh[0]["suggestions"])

# 2. RUSTY_AGENT_OUT
out = tmp + "/records.jsonl"; e = dict(env, RUSTY_AGENT_OUT=out)
p = run(["--agent-mode", "-c", "errors3.c", "-o", "/dev/null"], [], e)
lines = open(out).read().splitlines()
check("file has the same records as stderr, in order, one object per line", [json.loads(l) for l in lines] == recs(p.stderr) and len(lines) == 5, (len(lines), len(recs(p.stderr))))
check("stderr still has them (in addition)", len(recs(p.stderr)) == 5)
p = run(["--agent-mode", "-c", "clean.c", "-o", "/dev/null"], [], e)
check("a second run appends", len(open(out).read().splitlines()) == 6)
# linker records that pass through are copied too
open(out, "w").close()
p = run(["--agent-mode", "linkfail.c", "-o", tmp + "/lf"], [], e)
codes = [json.loads(l).get("code") for l in open(out).read().splitlines()]
check("passed-through linker record and clang's own are both in the file", "L0001" in codes and "C0009" in codes, codes)
# stable mode: the file gets the normalised text, same as stderr
check("file equals stderr records in stable mode (temp names fixed)", [json.loads(l) for l in open(out).read().splitlines()] == recs(p.stderr))
# agent off: never written, even when the variable is set
off = tmp + "/off.jsonl"
for flags in ([], ["--fix-preview"], ["--diagnostics-format=json"]):
    run(flags + ["-c", "errors3.c", "-o", "/dev/null"], [], dict(env, RUSTY_AGENT_OUT=off))
check("agent mode off: no file written (plain, --fix-preview, --diagnostics-format=json)", not os.path.exists(off))
p = run(["-c", "errors3.c", "-o", "/dev/null"], [], dict(env, RUSTY_AGENT_OUT=off))
plain = subprocess.run([CLANG, "-c", "errors3.c", "-o", "/dev/null"], cwd=FX, capture_output=True)
check("agent mode off: output bytes identical to plain clang with the variable set", p.stderr == plain.stderr and p.stdout == plain.stdout and p.returncode == plain.returncode)
# RUSTY_AGENT=1 enables it like the flag
envA = dict(env, RUSTY_AGENT="1", RUSTY_AGENT_OUT=tmp + "/envA.jsonl"); run(["-c", "errors3.c", "-o", "/dev/null"], [], envA)
check("RUSTY_AGENT=1 + RUSTY_AGENT_OUT works", len(open(tmp + "/envA.jsonl").read().splitlines()) == 5)
# bad paths: empty, missing folder; compile result is unaffected, no folder created
for val, name in (("", "empty"), (tmp + "/nodir/x.jsonl", "missing folder")):
    p = run(["--agent-mode", "-c", "errors3.c", "-o", "/dev/null"], [], dict(env, RUSTY_AGENT_OUT=val))
    rr = recs(p.stderr); w = [x for x in rr if x.get("code") == "C0500"]
    check(f"{name} path: a C0500 diagnostic RECORD (not plain text), records still on stderr, exit 1, nothing created", len(w) == 1 and "RUSTY_AGENT_OUT" in w[0]["message"] and w[0]["severity"] == "warning" and len(rr) == 6 and all(l.startswith("{") for l in p.stderr.decode().splitlines()) and p.returncode == 1 and not os.path.exists(tmp + "/nodir"), p.stderr[:300])
# run-san and --agent-help
fix = tmp + "/h"; subprocess.run([CLANG, "-g", "-fsanitize=address", FX + "/san_heap.c", "-o", fix], check=True)
out2 = tmp + "/san.jsonl"; p = run(["run-san", "--", fix], [], dict(env, RUSTY_AGENT="1", RUSTY_AGENT_OUT=out2))
check("run-san also writes the file when agent mode is on", [json.loads(l)["type"] for l in open(out2).read().splitlines()] == ["diagnostic", "summary"])
h = json.loads(run(["--agent-help"], []).stdout)
check("--agent-help documents the file and the new umbrella", "RUSTY_AGENT_OUT" in h["records_file"]["env"] and "missing-header" in h["umbrella_features"])
check("AGENT-MODE.md documents both", "RUSTY_AGENT_OUT" in open(os.path.join(root, "AGENT-MODE.md")).read() and "--no-explain-missing-header" in open(os.path.join(root, "AGENT-MODE.md")).read())
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
