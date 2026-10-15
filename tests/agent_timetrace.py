#!/usr/bin/env python3
"""Feature 8: --compile-time-report from -ftime-trace on a real, header-heavy C++ compile."""
import glob, hashlib, json, os, subprocess, sys, tempfile, time
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures"); CLANG = "/usr/bin/clang"
PRIV = tempfile.mkdtemp()
env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}; env["RUSTY_AGENT_DETAIL"] = "full"; env["TMPDIR"] = PRIV
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:500]))
def run(flags, args):
    t = time.time(); p = subprocess.run([RCC] + flags + args, cwd=FX, capture_output=True, env=env); wall = (time.time() - t) * 1000
    return [d for d in (json.loads(l) for l in p.stderr.decode().splitlines() if l.startswith('{"schema"')) if d["type"] == "compile-time"], p, wall
out = tempfile.mkdtemp()
r, p, wall = run(["--compile-time-report"], ["-std=c++20", "-c", "ct.cpp", "-o", out + "/ct.o"])
check("one compile-time record, exit 0", len(r) == 1 and p.returncode == 0 and r[0]["schema"] == 1 and r[0]["file"] == "ct.cpp", r[:1])
d = r[0]
check("total_ms is real: positive and not more than the wall time of the whole run", 0 < d["total_ms"] <= wall, (d["total_ms"], wall))
check("total_ms is most of the wall time", d["total_ms"] > wall * 0.5, (d["total_ms"], wall))
check("frontend + backend are within total", d["phases"]["frontend_ms"] + d["phases"]["backend_ms"] <= d["total_ms"] * 1.02, d["phases"])
for k in ("headers", "templates", "functions"): check(f"top {k}: 5 entries", len(d["slowest"][k]) == 5, len(d["slowest"][k]))
h = d["slowest"]["headers"]
check("headers sorted by self time, inclusive >= self", [x["self_ms"] for x in h] == sorted((x["self_ms"] for x in h), reverse=True) and all(x["inclusive_ms"] >= x["self_ms"] for x in h), h)
check("header paths are real files", all(os.path.exists(x["path"]) for x in h), [x["path"] for x in h])
check("sum of header self times is below the header parse phase", sum(x["self_ms"] for x in h) <= d["phases"]["header_parse_ms"] * 1.02, (sum(x["self_ms"] for x in h), d["phases"]["header_parse_ms"]))
f = d["slowest"]["functions"]
check("functions: sorted by total, total = optimize + codegen", [x["total_ms"] for x in f] == sorted((x["total_ms"] for x in f), reverse=True) and all(abs(x["total_ms"] - x["optimize_ms"] - x["codegen_ms"]) < 0.02 for x in f), f)
r2, p2, _ = run(["--compile-time-report=100000"], ["-std=c++20", "-ftime-trace-granularity=0", "-c", "ct.cpp", "-o", out + "/ct2.o"])
names = [t["name"] for t in r2[0]["slowest"]["templates"]]
check("our own template Fib<20> is among the instantiations", any(n == "Fib<20>" for n in names), len(names))
check("--compile-time-report=N gives more than 5 per list", len(names) > 5)
fn = [x["name"] for x in r2[0]["slowest"]["functions"]]
check("our own function slow(int) is among the compiled functions", any(n.startswith("slow(") or n == "slow" for n in fn), fn[:10])
t = r2[0]["slowest"]["templates"]
check("templates sorted by self time", [x["self_ms"] for x in t] == sorted((x["self_ms"] for x in t), reverse=True))
# nothing is left behind and the object is the same as plain clang's
check("no trace file next to the object or in the source folder; temp folder removed", glob.glob(out + "/*.json") == [] and glob.glob(FX + "/*.json") == [] and os.listdir(PRIV) == [])
subprocess.run([CLANG, "-std=c++20", "-c", "ct.cpp", "-o", out + "/plain.o"], cwd=FX)
sha = lambda f: hashlib.sha256(open(f, "rb").read()).hexdigest()
check("object bytes identical to plain clang", sha(out + "/plain.o") == sha(out + "/ct.o"))
# a C file, a failing compile, a missing file
r3, p3, _ = run(["--compile-time-report"], ["-c", "clean.c", "-o", out + "/c.o"])
check("C file works too", len(r3) == 1 and r3[0]["file"] == "clean.c" and r3[0]["total_ms"] > 0, r3[:1])
r4, p4, _ = run(["--compile-time-report"], ["-c", "errors3.c", "-o", out + "/e.o"])
check("failed compile: exit code kept, record still tells what happened", p4.returncode == 1 and len(r4) == 1, (p4.returncode, r4[:1]))
# alone: clang's text first; stable under agent mode but not on by default
txt = subprocess.run([RCC, "--compile-time-report", "-c", "errors3.c", "-o", out + "/e.o"], cwd=FX, capture_output=True, env=env).stderr.decode()
plain = subprocess.run([CLANG, "-fno-color-diagnostics", "-c", "errors3.c", "-o", out + "/e.o"], cwd=FX, capture_output=True).stderr.decode()
check("alone: clang's text kept, then the record", txt.startswith(plain))
r5, _, _ = run(["--agent-mode"], ["-c", "clean.c", "-o", out + "/c.o"]); check("--agent-mode does not turn it on", r5 == [])
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
