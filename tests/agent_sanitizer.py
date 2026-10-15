#!/usr/bin/env python3
"""Feature 10: rusty-cc run-san on real programs built with -fsanitize=address,undefined."""
import json, os, subprocess, sys, tempfile
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures"); CLANG = "/usr/bin/clang"
env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT") and k not in ("ASAN_OPTIONS", "UBSAN_OPTIONS")}; env["RUSTY_AGENT_DETAIL"] = "full"
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:600]))
d = tempfile.mkdtemp()
def build(name, san="address,undefined"):
    out = f"{d}/{name}"
    r = subprocess.run([CLANG, "-g", "-fno-omit-frame-pointer", f"-fsanitize={san}", f"{FX}/{name}.c", "-o", out], capture_output=True)
    assert r.returncode == 0, r.stderr
    return out
def san(prog, extra_env=None):
    e = dict(env, **(extra_env or {}))
    p = subprocess.run([RCC, "run-san", "--", prog], capture_output=True, env=e)
    recs = [json.loads(l) for l in p.stderr.decode().splitlines() if l.startswith('{"schema"')]
    text = [l for l in p.stderr.decode().splitlines() if not l.startswith('{"schema"')]
    plain = subprocess.run([prog], capture_output=True, env=dict(e, ASAN_OPTIONS="color=never", UBSAN_OPTIONS="print_stacktrace=1:color=never"))
    return [r for r in recs if r["type"] == "diagnostic"], [r for r in recs if r["type"] == "summary"], text, p, plain

# heap overflow
r, s, text, p, plain = san(build("san_heap"))
check("heap: one diagnostic, code C0400, severity error, shared fields", len(r) == 1 and r[0]["code"] == "C0400" and r[0]["severity"] == "error" and {"schema", "type", "message", "files", "symbols", "notes", "suggestions", "locations", "rendered"} <= set(r[0]), r)
x = r[0]; sz = x["sanitizer"]
check("heap: kind, access, size, region", sz["name"] == "AddressSanitizer" and sz["kind"] == "heap-buffer-overflow" and sz["access"] == "WRITE" and sz["size"] == 4 and sz["region"] == {"offset": 0, "relation": "after", "size": 16}, sz)
check("heap: primary location is the faulting line (6) and the allocation (5) is named", any(l["role"] == "primary" and l["line"] == 6 and l["file"].endswith("san_heap.c") for l in x["locations"]) and any(l["role"] == "allocated" and l["line"] == 5 for l in x["locations"]), x["locations"])
check("heap: symbol main", x["symbols"] == ["main"])
check("heap: suggestion about the bound", x["suggestions"] and "past the end" in x["suggestions"][0]["message"])
check("heap: rendered is the sanitizer's own text, shadow bytes included", "ERROR: AddressSanitizer: heap-buffer-overflow" in x["rendered"] and "Shadow bytes around the buggy address" in x["rendered"] and "ABORTING" in x["rendered"])
check("heap: program's exit code kept (ASan exits 1), stdout kept", p.returncode == plain.returncode == 1 and p.stdout == plain.stdout == b"before\n", (p.returncode, p.stdout))
check("heap: no sanitizer text left outside records", not any("AddressSanitizer" in t or "Shadow" in t for t in text), text[:3])
check("heap: summary record counts 1 report and has the exit", s and s[-1]["sanitizer_reports"] == 1 and s[-1]["exit"] == 1 and s[-1]["mode"] == "run-san", s)
# frames
fr = [f for st in sz["stacks"] if st["role"] == "access" for f in st["frames"]]
check("heap: frames structured: index, function, file, line, column", fr and fr[0]["function"] == "main" and fr[0]["line"] == 6 and fr[0]["column"] > 0 and fr[0]["index"] == 0, fr[:1])

# use after free
r, s, text, p, plain = san(build("san_uaf")); x = r[0]
check("uaf: kind and READ", x["sanitizer"]["kind"] == "heap-use-after-free" and x["sanitizer"]["access"] == "READ", x["sanitizer"])
check("uaf: freed and allocated frames named with lines", any(l["role"] == "freed" and l["line"] == 2 for l in x["locations"]) and any(n.startswith("freed by: dangling") for n in x["notes"]) and any(n.startswith("allocated by: dangling") for n in x["notes"]), x["notes"])
check("uaf: suggestion mentions free", "free()" in x["suggestions"][0]["message"])
# leak
r, s, text, p, plain = san(build("san_leak")); x = r[0]
check("leak: LeakSanitizer memory-leak with the allocating function", x["sanitizer"]["name"] == "LeakSanitizer" and x["sanitizer"]["kind"] == "memory-leak" and any("Direct leak of 100 byte" in n and "leak (" in n for n in x["notes"]), x["notes"])
check("leak: exit code 1 like the plain run", p.returncode == plain.returncode == 1)
# double free
r, s, text, p, plain = san(build("san_double")); x = r[0]
check("double free: kind double-free, main named", x["sanitizer"]["kind"] == "double-free" and "main" in x["symbols"], (x["sanitizer"]["kind"], x["symbols"]))
# UBSan continues: several reports, program's own exit code and stdout
r, s, text, p, plain = san(build("san_ub", "undefined"), None)
kinds = [x["sanitizer"]["kind"] for x in r]
check("ub: two UBSan reports in order, program runs on", kinds[:2] == ["signed-integer-overflow", "index-6-out-of-bounds-for-type-'int[4]'"] or (kinds[0] == "signed-integer-overflow" and "out-of-bounds" in kinds[1]), kinds)
check("ub: exit code 3 from the program and its stdout untouched", p.returncode == 3 == plain.returncode and p.stdout == plain.stdout and p.stdout.startswith(b"sum="), (p.returncode, p.stdout))
check("ub: location is the source place on the report line", r[0]["locations"][0]["line"] == 6 and r[0]["locations"][0]["role"] == "primary", r[0]["locations"])
check("ub: overflow message has the operands", "2147483647 + 1" in r[0]["message"])
check("ub: no sanitizer text outside records", not any("runtime error" in t for t in text), text[:3])
# the user's own options are respected (user last)
r, s, text, p, plain = san(build("san_heap"), {"ASAN_OPTIONS": "detect_leaks=0"})
check("user ASAN_OPTIONS still honoured", len(r) == 1)
# a program that is clean: no records, only the summary
prog = d + "/clean"; subprocess.run([CLANG, "-fsanitize=address", f"{FX}/clean.c", "-o", prog], check=True)
r, s, text, p, plain = san(prog)
check("clean program: 0 reports, exit 0, stdout kept", r == [] and p.returncode == 0 and p.stdout == b"5\n" and s[-1]["sanitizer_reports"] == 0, (p.stdout, s))
# program's own stderr text passes through
open(d + "/e.sh", "w").write("#!/bin/sh\necho oops >&2\nexit 4\n"); os.chmod(d + "/e.sh", 0o755)
r, s, text, p, plain = san(d + "/e.sh")
check("plain program: its stderr passes through, exit 4 kept", text == ["oops"] and p.returncode == 4 and s[-1]["exit"] == 4, (text, p.returncode))
# missing program
p = subprocess.run([RCC, "run-san", "--", d + "/nope"], capture_output=True, env=env)
check("missing program: error and exit 127", p.returncode == 127 and b"cannot run" in p.stderr, p.stderr)
p = subprocess.run([RCC, "run-san"], capture_output=True, env=env)
check("no program: usage and exit 2", p.returncode == 2 and b"usage" in p.stderr)
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
