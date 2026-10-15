#!/usr/bin/env python3
"""Feature 7: --effective-flags, checked against facts clang itself reports."""
import json, os, subprocess, sys
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RCC = os.environ.get("RUSTY_CC_BIN") or os.path.join(root, "target/release/rusty-cc"); FX = os.path.join(root, "tests/fixtures"); CLANG = "/usr/bin/clang"
env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTY_AGENT")}; env["RUSTY_AGENT_DETAIL"] = "full"
fails = 0
def check(name, cond, detail=""):
    global fails
    if not cond: fails += 1
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else "  " + str(detail)[:500]))
def ef(args, flags=("--effective-flags",)):
    p = subprocess.run([RCC] + list(flags) + args, cwd=FX, capture_output=True, env=env)
    r = [json.loads(l) for l in p.stderr.decode().splitlines() if l.startswith('{"schema"')]
    r = [x for x in r if x["type"] == "effective-flags"]
    return (r[0] if r else None), p
def out(*a): return subprocess.run(list(a), cwd=FX, capture_output=True, text=True).stdout.strip()

d, p = ef(["-O2", "-DFOO=3", "-Ihdr/inc", "-std=c11", "-c", "opt.c", "-o", "/tmp/rcc-ef.o"])
check("one record, shared header fields", d and d["schema"] == 1 and d["type"] == "effective-flags")
check("target triple equals clang -dumpmachine", d["target"]["triple"] == out(CLANG, "-dumpmachine"), d["target"])
check("language and standard", d["language"] == {"x": "c", "std": "c11"} and d["optimization"] == "-O2", (d["language"], d["optimization"]))
check("user define listed, and visible in the predefined macros", d["defines"]["user"] == ["FOO=3"] and d["defines"]["all"]["FOO"] == "3", d["defines"]["user"])
major = out(CLANG, "-dumpversion").split(".")[0]
check("predefined macros come from clang: __clang_major__", d["defines"]["all"]["__clang_major__"] == major, d["defines"]["all"].get("__clang_major__"))
check("-I directory is first in the search list", d["include_dirs"]["search"][0] == "hdr/inc" and d["include_dirs"]["from_dash_I"] == ["hdr/inc"], d["include_dirs"])
check("system directories include /usr/include", "/usr/include" in d["include_dirs"]["system"])
check("search list equals what clang -v prints", d["include_dirs"]["search"] == [l.strip() for l in subprocess.run([CLANG, "-v", "-E", "-Ihdr/inc", "-std=c11", "-x", "c", "/dev/null", "-o", "/dev/null"], cwd=FX, capture_output=True, text=True).stderr.split("#include <...> search starts here:\n")[1].split("End of search list.")[0].splitlines()])
check("sysroot absent when not given", d["sysroot"] is None)
check("linker is the machine default (mold fork) and resolved through the symlink", d["linker"]["path"] == "/usr/bin/ld" and "mold" in (d["linker"]["version"] or "") and d["linker"]["resolved"].endswith("mold"), d["linker"])
check("linker library dirs and libc listed", "c" in d["linker"]["libraries"] and any(x.startswith("/usr/lib") for x in d["linker"]["library_dirs"]), d["linker"])
check("the compile still ran: object exists", os.path.exists("/tmp/rcc-ef.o") and p.returncode == 0)
check("version and config file", d["clang"]["version"].startswith("clang version " + out(CLANG, "-dumpversion")) and d["clang"]["config_file"], d["clang"])

d, p = ef(["--target=aarch64-linux-gnu", "-fsyntax-only", "clean.c"])
check("--target changes the triple", d["target"]["triple"].startswith("aarch64"), d["target"])
check("aarch64 predefines differ", "__aarch64__" in d["defines"]["all"] and "__x86_64__" not in d["defines"]["all"])
d, p = ef(["--sysroot=/tmp/sr", "-c", "clean.c", "-o", "/dev/null"])
check("sysroot reported", d["sysroot"] == "/tmp/sr" and d["linker"]["sysroot"] == "/tmp/sr", (d["sysroot"], d["linker"]["sysroot"]))
d, p = ef(["-fuse-ld=bfd", "clean.c", "-o", "/tmp/rcc-ef.out"])
check("-fuse-ld=bfd selects GNU ld", d["linker"]["fuse_ld"] == "bfd" and ("bfd" in d["linker"]["resolved"] or "GNU ld" in (d["linker"]["version"] or "")), d["linker"])
d, p = ef(["-x", "c++", "-std=c++20", "-c", "clean.c", "-o", "/dev/null"])
check("C++ language and standard", d["language"] == {"x": "c++", "std": "c++20"}, d["language"])
d, p = ef(["-g", "-fPIC", "-c", "clean.c", "-o", "/dev/null"])
check("debug info and PIC", d["debug_info"] and d["position_independent"] is True, (d["debug_info"], d["position_independent"]))
# no input: describes an empty C file, runs no compile, exit 0
d, p = ef(["-O1", "-DX=1"])
check("no input: record with no_input_given, exit 0, no clang error text", d and d["no_input_given"] is True and p.returncode == 0 and b"no input files" not in p.stderr, p.stderr[:200])
# alone: clang text unchanged before the record
txt = subprocess.run([RCC, "--effective-flags", "-c", "errors3.c", "-o", "/dev/null"], cwd=FX, capture_output=True, env=env).stderr.decode()
plain = subprocess.run([CLANG, "-fno-color-diagnostics", "-c", "errors3.c", "-o", "/dev/null"], cwd=FX, capture_output=True).stderr.decode()
check("alone: clang's text then the record", txt.startswith(plain) and '"type":"effective-flags"' in txt[len(plain):])
# not part of --agent-mode
r = subprocess.run([RCC, "--agent-mode", "-c", "clean.c", "-o", "/dev/null"], cwd=FX, capture_output=True, env=env).stderr.decode()
check("--agent-mode does not turn it on", "effective-flags" not in r)
print("FAILED %d" % fails if fails else "ALL PASS"); sys.exit(1 if fails else 0)
