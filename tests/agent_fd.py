#!/usr/bin/env python3
"""Item A: RUSTY_AGENT_FD=N appends the same JSON-lines records to an already-open descriptor (like RUSTY_AGENT_OUT).
Runs the real rusty-cc and the real clang; parses every record as JSON."""
import json, os, stat, subprocess, sys, tempfile
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures"); CLANG = "/usr/bin/clang"
env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}; env["RUSTY_AGENT_DETAIL"] = "full"
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:600]))
def run(flags, e=env, pass_fds=(), cmd=None):
    return subprocess.run([cmd or RCC] + list(flags), cwd=FX, capture_output=True, env=e, pass_fds=pass_fds)
def recs(b): return [json.loads(l) for l in b.decode().splitlines() if l.startswith('{"schema"')]
tmp = tempfile.mkdtemp()
JOB = ["--agent-mode", "-c", "errors3.c", "-o", "/dev/null"]
def fd_of(path, flags=os.O_WRONLY | os.O_CREAT | os.O_APPEND):
    return os.open(path, flags, 0o644)

# 1. valid fd: file opened by the caller (the shell's 3>file), records identical to stderr, stderr unchanged
f1 = tmp + "/fd.jsonl"; fd = fd_of(f1)
p = run(JOB, dict(env, RUSTY_AGENT_FD=str(fd)), pass_fds=(fd,)); os.close(fd)
lines = open(f1).read().splitlines()
check("valid fd: file has the same records as stderr, in order, every line a JSON object", [json.loads(l) for l in lines] == recs(p.stderr) and len(lines) == 5, (len(lines), len(recs(p.stderr))))
check("valid fd: exit code is clang's (1) and no warning record", p.returncode == 1 and not [r for r in recs(p.stderr) if r.get("code") in ("C0500", "C0501")])
base = run(JOB)
check("valid fd: stderr equals the run without the variable", p.stderr == base.stderr, (len(p.stderr), len(base.stderr)))
# a pipe too
r, w = os.pipe(); p = run(JOB, dict(env, RUSTY_AGENT_FD=str(w)), pass_fds=(w,)); os.close(w)
got = os.read(r, 1 << 20).decode().splitlines(); os.close(r)
check("valid fd: a pipe gets the records too", [json.loads(l) for l in got] == recs(p.stderr) and len(got) == 5, len(got))
# appends to an open file after earlier content
fd = fd_of(f1); p = run(["--agent-mode", "-c", "clean.c", "-o", "/dev/null"], dict(env, RUSTY_AGENT_FD=str(fd)), pass_fds=(fd,)); os.close(fd)
check("valid fd: a second run appends", len(open(f1).read().splitlines()) == 5 + len(recs(p.stderr)))
# linker records that pass through are copied once
f2 = tmp + "/lf.jsonl"; fd = fd_of(f2); p = run(["--agent-mode", "linkfail.c", "-o", tmp + "/lf"], dict(env, RUSTY_AGENT_FD=str(fd)), pass_fds=(fd,)); os.close(fd)
codes = [json.loads(l).get("code") for l in open(f2).read().splitlines()]
check("valid fd: passed-through linker record and clang's own are in the file, each once", codes.count("L0001") == 1 and "C0009" in codes, codes)
check("valid fd: file equals stderr for the linker job", [json.loads(l) for l in open(f2)] == recs(p.stderr))

# 2. bad values: a C0501 warning RECORD, records still on stderr, the job's exit code unchanged, nothing else fails
def bad(name, value, fd_extra=(), why=""):
    e = dict(env, RUSTY_AGENT_FD=value)
    p = run(JOB, e, pass_fds=fd_extra); rr = recs(p.stderr); w = [x for x in rr if x.get("code") == "C0501"]
    check(f"bad fd ({name}): one C0501 warning record naming RUSTY_AGENT_FD, records still on stderr, all lines JSON, exit 1", len(w) == 1 and w[0]["severity"] == "warning" and "RUSTY_AGENT_FD" in w[0]["message"] and why in w[0]["message"] and len(rr) == 6 and all(l.startswith("{") for l in p.stderr.decode().splitlines()) and p.returncode == 1, (p.stderr[:400], p.returncode))
bad("not a number", "abc", why="not a decimal number")
bad("empty", "", why="not a decimal number")
bad("negative", "-1", why="not a decimal number")
bad("too large", "99999999999999999999999", why="too large")
bad("stderr", "2", why="below 3")
bad("closed", "57", why="not open")
rf = os.open(FX + "/errors3.c", os.O_RDONLY); bad("read-only", str(rf), (rf,), why="reading only"); os.close(rf)
# clean job with a bad fd still exits 0
p = run(["--agent-mode", "-c", "clean.c", "-o", "/dev/null"], dict(env, RUSTY_AGENT_FD="57"))
check("closed fd: a job that succeeds still exits 0", p.returncode == 0 and len([x for x in recs(p.stderr) if x.get("code") == "C0501"]) == 1, p.stderr[:300])

# 3. both OUT and FD: both files get every record; one bad, the other still works and gets the warning
fo = tmp + "/o.jsonl"; ff = tmp + "/f.jsonl"; fd = fd_of(ff)
p = run(JOB, dict(env, RUSTY_AGENT_OUT=fo, RUSTY_AGENT_FD=str(fd)), pass_fds=(fd,)); os.close(fd)
a = [json.loads(l) for l in open(fo)]; b = [json.loads(l) for l in open(ff)]
check("OUT and FD both set: both get every record, equal to stderr", a == b == recs(p.stderr) and len(a) == 5, (len(a), len(b)))
fo2 = tmp + "/o2.jsonl"; p = run(JOB, dict(env, RUSTY_AGENT_OUT=fo2, RUSTY_AGENT_FD="57"))
a = [json.loads(l) for l in open(fo2)]
check("OUT good, FD closed: the file gets the C0501 warning and all records", [x.get("code") for x in a].count("C0501") == 1 and a == recs(p.stderr) and p.returncode == 1, [x.get("code") for x in a])
fd = fd_of(tmp + "/g.jsonl"); p = run(JOB, dict(env, RUSTY_AGENT_OUT=tmp + "/nodir/x", RUSTY_AGENT_FD=str(fd)), pass_fds=(fd,)); os.close(fd)
g = [json.loads(l) for l in open(tmp + "/g.jsonl")]
check("OUT bad, FD good: C0500 in the descriptor's copy, all records there", [x.get("code") for x in g].count("C0500") == 1 and g == recs(p.stderr) and p.returncode == 1)
p = run(JOB, dict(env, RUSTY_AGENT_OUT=tmp + "/nodir/x", RUSTY_AGENT_FD="57"))
check("both bad: a C0500 and a C0501 record, exit 1", sorted(x.get("code") for x in recs(p.stderr) if x.get("code") in ("C0500", "C0501")) == ["C0500", "C0501"] and p.returncode == 1)

# 4. agent mode off: the variable is ignored, nothing written, output equals plain clang
f3 = tmp + "/off.jsonl"; fd = fd_of(f3)
off = dict(env, RUSTY_AGENT_FD=str(fd)); off.pop("RUSTY_AGENT_DETAIL")
p = run(["-c", "errors3.c", "-o", "/dev/null"], off, pass_fds=(fd,)); os.close(fd)
c = subprocess.run([CLANG, "-c", "errors3.c", "-o", "/dev/null"], cwd=FX, capture_output=True, env=env)
check("agent off: plain output equals clang, nothing written to the descriptor", p.stderr == c.stderr and p.stdout == c.stdout and p.returncode == c.returncode and os.path.getsize(f3) == 0)
p = run(["-c", "errors3.c", "-o", "/dev/null"], dict(env, RUSTY_AGENT_FD="57"))
check("agent off: a closed fd gives no warning, output equals clang", p.stderr == c.stderr and b"C0501" not in p.stderr)
fd = fd_of(f3); p = run(["--no-agent-mode", "-c", "errors3.c", "-o", "/dev/null"], dict(env, RUSTY_AGENT="1", RUSTY_AGENT_FD=str(fd)), pass_fds=(fd,)); os.close(fd)
check("--no-agent-mode after RUSTY_AGENT=1: descriptor untouched", os.path.getsize(f3) == 0 and p.stderr == c.stderr)

# 5. children do not inherit RUSTY_AGENT_FD (a fake clang records its environment)
fake = tmp + "/fakeclang"; envdump = tmp + "/childenv"
open(fake, "w").write(f"#!/bin/sh\nenv | grep '^RUSTY_AGENT' >> {envdump}\nexit 0\n"); os.chmod(fake, 0o755)
fd = fd_of(tmp + "/c.jsonl")
p = run(JOB, dict(env, RUSTY_AGENT_FD=str(fd), RUSTY_AGENT_OUT=tmp + "/c2.jsonl", RUSTY_CC_CLANG=fake), pass_fds=(fd,)); os.close(fd)
ev = open(envdump).read()
check("the compiler child gets RUSTY_AGENT=1 but neither RUSTY_AGENT_FD nor RUSTY_AGENT_OUT", "RUSTY_AGENT=1" in ev and "RUSTY_AGENT_FD" not in ev and "RUSTY_AGENT_OUT" not in ev, ev)
os.remove(envdump)
prog = tmp + "/prog"; open(prog, "w").write(f"#!/bin/sh\nenv | grep '^RUSTY_AGENT' > {envdump}\nexit 0\n"); os.chmod(prog, 0o755)
fd = fd_of(tmp + "/s.jsonl"); p = run(["run-san", "--", prog], dict(env, RUSTY_AGENT="1", RUSTY_AGENT_FD=str(fd)), pass_fds=(fd,)); os.close(fd)
ev = open(envdump).read()
check("run-san's test child does not inherit RUSTY_AGENT_FD", "RUSTY_AGENT=1" in ev and "RUSTY_AGENT_FD" not in ev, ev)
# run-san writes the descriptor too
san = tmp + "/h"; subprocess.run([CLANG, "-g", "-fsanitize=address", FX + "/san_heap.c", "-o", san], check=True)
fd = fd_of(tmp + "/s2.jsonl"); p = run(["run-san", "--", san], dict(env, RUSTY_AGENT="1", RUSTY_AGENT_FD=str(fd)), pass_fds=(fd,)); os.close(fd)
check("run-san also writes the descriptor", [json.loads(l)["type"] for l in open(tmp + "/s2.jsonl")] == ["diagnostic", "summary"])

# 6. docs
h = json.loads(run(["--agent-help"], env).stdout)
check("--agent-help documents RUSTY_AGENT_FD and C0501", "RUSTY_AGENT_FD" in h["records_fd"]["env"] and any(c["code"] == "C0501" for c in h["codes"]))
doc = open(os.path.join(root, "AGENT-MODE.md")).read()
check("AGENT-MODE.md documents RUSTY_AGENT_FD and C0501", "RUSTY_AGENT_FD" in doc and "C0501" in doc)
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
