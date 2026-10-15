# rusty-cc

A thin driver that sits in front of clang. Used normally it changes nothing: it hands your
command to clang unchanged, so the output, exit status and the files it builds are exactly clang's.
Turn on its agent mode and it reshapes what the compiler says into something a program can read.

## Why it exists

Compiler messages are written for a person at a terminal: coloured text, long note chains, and a
cascade of follow-on errors after one real mistake (one unknown type can produce a dozen
"undeclared identifier" lines). A script or tool that reads that text has to guess where one
diagnostic ends, which line is the real cause, and what to try next. rusty-cc does that reading once,
in one place, and prints the result as JSON, one record per line, in a format shared with the
sibling tools (rusty-ld, rusty-cargo, rusty-grep, rusty-sh).

## What it does

- **Default (agent mode off):** a transparent pass-through to clang. Nothing is added or removed.
  Tests compare output bytes and built files against plain clang.
- **Agent mode** (`--agent-mode` or `RUSTY_AGENT=1`): every diagnostic becomes a JSON record, follow-on
  errors are grouped under their first cause, a missing header gets a record saying where clang
  looked and where the file exists, and a closing summary gives the exit code, time, file count and
  error and warning counts. Object files and binaries stay byte-identical.
- **On request:** machine fix-its shown as a diff without writing anything (`--fix-preview`); why a
  function was not inlined, vectorized or unrolled; the effective flags clang would use; a compile-time
  report of the slowest headers and templates; and `run-san`, which turns sanitizer reports into records.
- **Size control:** `--agent-detail=brief|normal|full` (or `RUSTY_AGENT_DETAIL`) sets how much each
  record says, because every byte a tool reads has a cost. Records go to standard error; a copy can go
  to a file (`RUSTY_AGENT_OUT`) or an open descriptor (`RUSTY_AGENT_FD`).

The full list of options, record types and codes is in [AGENT-MODE.md](AGENT-MODE.md), and
`rusty-cc --agent-help` prints it as JSON.

## Build and test

Needs a recent Rust (built and tested with 1.98) and clang.

    cargo build --release        # binary: target/release/rusty-cc
    cargo test                   # integration tests; tests/*.py run the real binary on real input

Use it as a drop-in by calling `rusty-cc` where you would call `clang`. It finds clang on the path, or
from `RUSTY_CC_CLANG`.

## Status and limits

Early (0.1). Two tests fail on some machines: `header_explainer` needs a Python development package, and
`passthrough` needs `cc`/`clang` symlinks under `target/links`. Both fail the same way without our changes.
The record format is a draft (schema 1) and may still change.

## Licence

No licence is granted. The source is published so it can be read; ask before reusing it. The Rust crates
it depends on keep their own licences (MIT, Apache-2.0 and similar).
