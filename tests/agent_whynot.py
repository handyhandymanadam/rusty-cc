#!/usr/bin/env python3
"""Feature 6: --why-not-inlined / -vectorized / -unrolled from the optimization record, on real -O2 compiles."""
import hashlib, json, os, subprocess, sys, tempfile, glob
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures"); CLANG = "/usr/bin/clang"
PRIV = tempfile.mkdtemp()  # private TMPDIR: parallel tests cannot disturb the leftovers check
env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}; env["RUSTY_AGENT_DETAIL"] = "full"; env["TMPDIR"] = PRIV
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:500]))
def run(flags, args, cwd=FX):
    p = subprocess.run([RCC] + flags + args, cwd=cwd, capture_output=True, env=env)
    return [d for d in (json.loads(l) for l in p.stderr.decode().splitlines() if l.startswith('{"schema"')) if d["type"] == "why-not"], p
O = ["-O2", "-c", "opt.c", "-o", "/tmp/rcc-why.o"]
def one(what, fn, args=O):
    r, p = run(["--why-not-%s=%s" % (what, fn)], args); assert len(r) == 1, (what, fn, p.stderr[:300]); return r[0]

d = one("inlined", "never")
check("never: not-optimized, reason NeverInline", d["outcome"] == "not-optimized" and d["events"][0]["reason"] == "NeverInline" and d["events"][0]["role"] == "callee" and "noinline" in d["events"][0]["message"], d)
check("never: location of the call (line 9, in main)", d["events"][0]["location"]["file"] == "opt.c" and d["events"][0]["location"]["line"] == 9, d["events"][0]["location"])
check("never: shared fields schema/type/what/function", d["schema"] == 1 and d["type"] == "why-not" and d["what"] == "inlined" and d["function"] == "never" and d["optimization"] == "-O2")
d = one("inlined", "rand")
check("rand: NoDefinition with an -flto suggestion", d["events"][0]["reason"] == "NoDefinition" and any(s["option"] == "-flto" for s in d["suggestions"]), d["suggestions"])
d = one("inlined", "rec")
check("rec: recursion refused (NeverInline), one call inlined", d["outcome"] == "not-optimized" and any("recursive" in e["message"] for e in d["events"]), d["summary"])
d = one("inlined", "small")
check("small: inlined everywhere, nothing to explain", d["outcome"] == "optimized" and "nothing to explain" in d["summary"], d["summary"])
check("small: cost and threshold in details", d["events"][0]["details"]["cost"] and d["events"][0]["details"]["threshold"], d["events"][0])
d = one("inlined", "no_such_function")
check("unknown name: no-data and says why", d["outcome"] == "no-data" and "never appeared" in d["summary"], d["summary"])

d = one("vectorized", "vec")
check("vec: vectorized, loop at line 7", d["outcome"] == "optimized" and d["events"][0]["status"] == "vectorized" and d["events"][0]["location"]["line"] == 7, d)
d = one("vectorized", "nov")
check("nov: not vectorized, reason names the call", d["outcome"] == "not-optimized" and any(r["reason"] == "CantVectorizeLibcall" for e in d["events"] for r in e["reasons"]), d["events"])
check("nov: one loop reported once even though it was inlined into main", len(d["events"]) == 1, len(d["events"]))
check("nov: suggestion for the library call", any("library call" in s["message"] for s in d["suggestions"]))
d = one("vectorized", "big")
check("big: its one loop (line 5), head and call remarks grouped", len(d["events"]) == 1 and d["events"][0]["location"]["line"] == 5, d["events"])
d = one("unrolled", "vec")
check("vec unrolled: partial unroll recorded", d["outcome"] == "optimized" and d["events"][0]["status"] == "unrolled", d)
d = one("unrolled", "nov")
check("nov unroll: DontUnroll advice (call in loop)", d["outcome"] == "not-optimized" and any(r["reason"] == "DontUnroll" for e in d["events"] for r in e["reasons"]), d["events"])

# no optimization level: says so
d = one("inlined", "small", ["-c", "opt.c", "-o", "/tmp/rcc-why.o"])
check("no -O: explains that the optimizer did not run", d["outcome"] == "no-data" and d["optimization"] is None and "-O0" in d["summary"], d["summary"])
# syntax-only / multiple files: no data with reason
d = one("inlined", "small", ["-O2", "-fsyntax-only", "opt.c"])
check("-fsyntax-only: explains no code generation", d["outcome"] == "no-data" and "does not generate code" in d["summary"], d["summary"])
d = one("inlined", "small", ["-O2", "-c", "opt.c", "clean.c"])
check("two sources: explains the one-file limit", d["outcome"] == "no-data" and "one source file" in d["summary"], d["summary"])
# several questions at once
r, p = run(["--why-not-inlined=never", "--why-not-vectorized=nov"], O)
check("two questions give two records", [x["what"] for x in r] == ["inlined", "vectorized"])
# agent-mode alone has no why-not; the umbrella still gives a summary after
r, p = run(["--agent-mode"], O); check("--agent-mode alone: no why-not record", r == [])
# the object file is byte-identical to plain clang, and no .yaml is left behind
d1 = tempfile.mkdtemp(); d2 = tempfile.mkdtemp()
subprocess.run([CLANG, "-O2", "-g", "-c", "opt.c", "-o", d1 + "/a.o"], cwd=FX); subprocess.run([RCC, "--why-not-inlined=big", "-O2", "-g", "-c", "opt.c", "-o", d2 + "/a.o"], cwd=FX, env=env, capture_output=True)
sha = lambda f: hashlib.sha256(open(f, "rb").read()).hexdigest()
check("object bytes identical with the question asked", sha(d1 + "/a.o") == sha(d2 + "/a.o"))
check("no optimization record left in the output folder or the source folder", glob.glob(d2 + "/*.yaml") == [] and glob.glob(FX + "/*.yaml") == [] and os.listdir(PRIV) == [])
# clang's own diagnostics still show (compile with a warning)
p = subprocess.run([RCC, "--why-not-inlined=big", "-O2", "-Wall", "-c", "warn.c", "-o", "/dev/null"], cwd=FX, capture_output=True, env=env)
check("normal diagnostics stay (text mode)", b"unused variable" in p.stderr)
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
