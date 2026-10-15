#!/usr/bin/env python3
"""Feature 1: diagnostics as shared-format JSON lines, on real compiles."""
import hashlib, json, os, subprocess, sys, tempfile
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures"); CLANG = "/usr/bin/clang"
env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}; env["RUSTY_AGENT_DETAIL"] = "full"
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:300]))
def run(exe, args, e=env):
    return subprocess.run(exe + args, cwd=FX, capture_output=True, env=e)
def recs(p):
    out = []; plain = []
    for l in p.stderr.decode().splitlines():
        if l.startswith('{"schema"'):
            d = json.loads(l)
            if d["type"] != "summary": out.append(d)  # the summary has its own test (agent_summary.py)
        else: plain.append(l)
    return out, plain
FIELDS = {"schema", "type", "code", "severity", "message", "files", "symbols", "notes", "suggestions", "locations", "rendered", "flag", "fixits"}

# 1. 3 syntax/semantic errors in C
p = run([RCC, "--agent-mode"], ["-c", "errors3.c", "-o", "/dev/null"]); r, plain = recs(p)
base = run([CLANG], ["-fno-color-diagnostics", "-c", "errors3.c", "-o", "/dev/null"])
check("errors3: exit code equals clang", p.returncode == base.returncode == 1, (p.returncode, base.returncode))
check("errors3: stdout equals clang", p.stdout == base.stdout)
check("errors3: 4 diagnostic records, no stray text", len(r) == 4 and plain == [], (len(r), plain))
check("errors3: every record has the shared fields", all(FIELDS <= set(x) and x["schema"] == 1 and x["type"] == "diagnostic" for x in r))
check("errors3: codes", [x["code"] for x in r] == ["C0001", "C0002", "C0005", "C0004"], [x["code"] for x in r])
check("errors3: severity and flag", [x["severity"] for x in r] == ["error"] * 4 and r[2]["flag"] == "-Wint-conversion" and r[0]["flag"] is None)
check("errors3: location of first error", r[0]["locations"][0]["file"] == "errors3.c" and r[0]["locations"][0]["line"] == 3 and r[0]["locations"][0]["column"] == 13 and r[0]["locations"][0]["end_column"] == 27, r[0]["locations"])
check("errors3: symbols", r[0]["symbols"] == ["undefined_name"] and r[3]["symbols"] == ["nosuch"], [x["symbols"] for x in r])
fx = r[1]["fixits"]
check("errors3: fix-it is a structured edit", len(fx) == 1 and fx[0]["file"] == "errors3.c" and fx[0]["text"] == ";" and fx[0]["start"]["line"] == 4 and fx[0]["start"]["column"] == 13 and fx[0]["start"] == fx[0]["end"], fx)
src = open(os.path.join(FX, "errors3.c"), "rb").read()
check("errors3: fix-it byte offset points after 'return x'", src[:fx[0]["start"]["offset"]].endswith(b"return x"), fx)
check("errors3: suggestion text", r[1]["suggestions"][0]["option"] is None and "insert ';'" in r[1]["suggestions"][0]["message"], r[1]["suggestions"])
# rendered text of all records, in order, equals what plain clang printed (minus the "N errors generated." line)
rendered = "".join(x["rendered"] for x in r)
check("errors3: rendered equals plain clang text", rendered == base.stderr.decode().replace("4 errors generated.\n", ""), (rendered[-200:], base.stderr.decode()[-200:]))

# 2. warnings, exit 0
p = run([RCC, "--agent-mode"], ["-Wall", "-Wextra", "-c", "warn.c", "-o", "/dev/null"]); r, plain = recs(p)
base = run([CLANG], ["-fno-color-diagnostics", "-Wall", "-Wextra", "-c", "warn.c", "-o", "/dev/null"])
check("warn: exit 0 like clang", p.returncode == 0 == base.returncode)
check("warn: 5 warnings, all severity warning", len(r) == 5 and all(x["severity"] == "warning" for x in r), len(r))
check("warn: flags", [x["flag"] for x in r] == ["-Wunused-function", "-Wunused-parameter", "-Wunused-parameter", "-Wunused-variable", "-Wformat"], [x["flag"] for x in r])
fmt = [x for x in r if x["flag"] == "-Wformat"][0]
check("warn: format fix-it replaces %d by %ld", fmt["fixits"][0]["text"] == "%ld" and fmt["fixits"][0]["end"]["column"] - fmt["fixits"][0]["start"]["column"] == 2, fmt["fixits"])
r2, _ = recs(run([RCC, "--agent-mode", "--no-stable-diagnostics"], ["-Wall", "-Wextra", "-c", "warn.c", "-o", "/dev/null"]))
check("warn: rendered equals plain clang text (clang's own order)", "".join(x["rendered"] for x in r2) == base.stderr.decode().replace("5 warnings generated.\n", ""))
check("warn: stable order has the same diagnostics, sorted by place", sorted(x["rendered"] for x in r) == sorted(x["rendered"] for x in r2))

# 3. C++ template error: notes and locations in another file (stable mode sorts by place: the Box<int> error first)
p = run([RCC, "--agent-mode"], ["-c", "tmpl.cpp", "-o", "/dev/null"]); r, plain = recs(p)
base = run([CLANG], ["-fno-color-diagnostics", "-c", "tmpl.cpp", "-o", "/dev/null"])
check("template: 2 main diagnostics", len(r) == 2 and sorted(x["code"] for x in r) == ["C0008", "C0100"], [x["code"] for x in r])
ov = [x for x in r if x["code"] == "C0008"][0]; inst = [x for x in r if x["code"] == "C0100"][0]
check("template: notes carry the candidates", len(ov["notes"]) == 2 and "candidate function not viable" in ov["notes"][0], ov["notes"])
check("template: instantiation note on the other", "in instantiation of member function" in inst["notes"][0])
check("template: header file listed", any(f.endswith("stl_vector.h") for f in ov["files"]), ov["files"])
r2, _ = recs(run([RCC, "--agent-mode", "--no-stable-diagnostics"], ["-c", "tmpl.cpp", "-o", "/dev/null"]))
check("template: rendered equals plain clang text (clang's own order)", "".join(x["rendered"] for x in r2) == base.stderr.decode().replace("2 errors generated.\n", ""))

# 4. -Werror shows up as flag + werror
p = run([RCC, "--agent-mode"], ["-Werror=unused-variable", "-Wall", "-c", "warn.c", "-o", "/dev/null"]); r, plain = recs(p)
check("werror: error with flag -Wunused-variable", any(x["severity"] == "error" and x["flag"] == "-Wunused-variable" and x.get("werror") for x in r), [(x["severity"], x["flag"]) for x in r])
check("werror: exit 1", p.returncode == 1)

# 5. link failure: linker's JSON passes through untouched, clang driver error is a record
p = run([RCC, "--agent-mode"], ["linkfail.c", "-o", "/tmp/rcc-lf.out"]); r, plain = recs(p)
check("link: linker record first, untouched (code L0001)", r and r[0]["code"] == "L0001" and r[0]["message"].startswith("undefined symbol"), r[:1])
check("link: clang's own error is C0009", any(x["code"] == "C0009" and x["files"] == [] and x["locations"] == [] for x in r), [x["code"] for x in r])
check("link: exit 1", p.returncode == 1)
# env switch is equal to the flag, and reaches the linker
p2 = run([RCC], ["linkfail.c", "-o", "/tmp/rcc-lf.out"], dict(env, RUSTY_AGENT="1")); r2, _ = recs(p2)
check("env RUSTY_AGENT=1 equals --agent-mode", [x["code"] for x in r2] == [x["code"] for x in r])
for val in ("0", "true", "yes", ""):
    p3 = run([RCC], ["-c", "errors3.c", "-o", "/dev/null"], dict(env, RUSTY_AGENT=val))
    check(f"RUSTY_AGENT={val!r} changes nothing", p3.stderr == run([CLANG], ["-c", "errors3.c", "-o", "/dev/null"]).stderr)

# 6. clean compile: no records, no output; stdout of tools untouched
p = run([RCC, "--agent-mode"], ["-c", "clean.c", "-o", "/dev/null"])
check("clean: only the summary record on stderr, exit 0", [json.loads(l)["type"] for l in p.stderr.decode().splitlines()] == ["summary"] and p.returncode == 0)
p = run([RCC, "--agent-mode"], ["-E", "clean.c"]); b = run([CLANG], ["-E", "clean.c"])
check("stdout of -E identical in agent mode", p.stdout == b.stdout and p.returncode == 0)

# 7. objects and binaries identical in agent mode, with -g -O2
d = tempfile.mkdtemp()
def sha(f): return hashlib.sha256(open(f, "rb").read()).hexdigest()
for src in ("clean.c", "warn.c"):
    run([CLANG], ["-g", "-O2", "-Wall", "-c", src, "-o", d + "/a.o"]); run([RCC, "--agent-mode"], ["-g", "-O2", "-Wall", "-c", src, "-o", d + "/b.o"])
    check(f"object bytes identical in agent mode ({src})", sha(d + "/a.o") == sha(d + "/b.o"))
run([CLANG], ["-O2", "clean.c", "-o", d + "/a.out"]); run([RCC, "--agent-mode"], ["-O2", "clean.c", "-o", d + "/b.out"])
check("binary bytes identical in agent mode", sha(d + "/a.out") == sha(d + "/b.out"))

# 8. --version
p = run([RCC, "--agent-mode"], ["--version"]); b = run([CLANG], ["--version"])
check("--version: clang text, the agent line as the next line, then the features line", p.stdout.startswith(b.stdout + b"agent mode: yes, schema 1\nagent features: ") and p.stdout.count(b"\n") == b.stdout.count(b"\n") + 2, p.stdout)
p = run([RCC], ["--version"]); check("--version without agent mode: unchanged", p.stdout == b.stdout)

# 9. our flags are stripped; --diagnostics-format=json alone works; =text wins
p = run([RCC, "--diagnostics-format=json"], ["-c", "errors3.c", "-o", "/dev/null"]); r, _ = recs(p)
check("--diagnostics-format=json alone", len(r) == 4)
p = run([RCC, "--agent-mode", "--diagnostics-format=text"], ["-c", "errors3.c", "-o", "/dev/null"]); r, _ = recs(p)
check("--diagnostics-format=text turns records off", r == [] and b"error:" in p.stderr)
# 10. no collision with clang: none of our flag names is known to clang
names = ["--agent-mode", "--no-agent-mode", "--diagnostics-format=json", "--diagnostics-format=text"]
hh = subprocess.run([CLANG, "--help-hidden"], capture_output=True, text=True).stdout
check("flag names absent from clang --help-hidden", not any(n.split("=")[0] in hh for n in names))
for n in names:
    q = subprocess.run([CLANG, n, "-fsyntax-only", "clean.c"], cwd=FX, capture_output=True, text=True)
    check("clang itself rejects " + n, q.returncode != 0 and "unknown argument" in q.stderr, q.stderr)
# 11. no-input and bad flag: driver diagnostics become records, exit code 1
p = run([RCC, "--agent-mode"], ["-fno-such-flag", "-c", "clean.c"]); r, _ = recs(p)
check("driver error: C0011 record, exit 1", len(r) >= 1 and r[0]["code"] == "C0011" and p.returncode == 1, (r[:1], p.returncode))
p = run([RCC, "--agent-mode"], ["-c", "nope.c"]); r, _ = recs(p)
check("missing input file: C0012", r and r[0]["code"] == "C0012", r[:1])
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
