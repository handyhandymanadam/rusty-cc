#!/usr/bin/env python3
"""Run fixed compile jobs through a compiler and record exit code, stdout/stderr sha256, timing median.
usage: baseline.py COMPILER OUT.json [runs]"""
import hashlib, json, re, os, statistics, subprocess, sys, time, tempfile
here = os.path.dirname(os.path.abspath(__file__))
fx = os.path.join(here, "fixtures")
JOBS = {
  "clean_c":   ["-c", "clean.c", "-o", "OUT/clean.o"],
  "errors3_c": ["-c", "errors3.c", "-o", "OUT/e.o"],
  "template_cpp": ["-c", "tmpl.cpp", "-o", "OUT/t.o"],
  "warnings_c": ["-Wall", "-Wextra", "-c", "warn.c", "-o", "OUT/w.o"],
  "linkfail_c": ["linkfail.c", "-o", "OUT/lf"],
}
def norm(b):
    # clang names its temp object randomly (/tmp/linkfail-ab12cd.o); mask it before hashing
    return re.sub(rb"/[^ \n:]*/(\w+)-[0-9a-f]{6}\.o", rb"/TMP/\1-XXXXXX.o", b)
def run(comp, name, runs, out):
    args = [a.replace("OUT", out) for a in JOBS[name]]
    exe = comp
    times = []; res = None
    for _ in range(runs):
        t = time.perf_counter()
        p = subprocess.run(exe + args, cwd=fx, capture_output=True)
        times.append((time.perf_counter() - t) * 1000)
        r = (p.returncode, hashlib.sha256(norm(p.stdout)).hexdigest(), hashlib.sha256(norm(p.stderr)).hexdigest(), len(p.stderr))
        if res and res != r: r = ("UNSTABLE",) + r
        res = r
    return {"exit": res[0], "stdout_sha256": res[1], "stderr_sha256": res[2], "stderr_bytes": res[3] if len(res) > 3 else None, "median_ms": round(statistics.median(times), 2)}
if __name__ == "__main__":
    comp = sys.argv[1].split(); outf = sys.argv[2]; runs = int(sys.argv[3]) if len(sys.argv) > 3 else 12
    out = tempfile.mkdtemp()
    data = {}
    for n in JOBS: data[n] = run(comp, n, runs, out)
    json.dump({"compiler": comp, "runs": runs, "jobs": data}, open(outf, "w"), indent=1)
    for n, d in data.items(): print(n, d["exit"], d["stderr_sha256"][:12], d["median_ms"], "ms")
