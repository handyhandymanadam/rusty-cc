# Agent mode (rusty-cc)

`rusty-cc` is a driver in front of clang (`/usr/bin/clang`, or `RUSTY_CC_CLANG`). Without agent mode it is a plain
pass-through: it replaces itself with clang (same arguments, environment, standard streams, exit code, output bytes).
Agent mode is for programs and scripts that read the compiler's output. It is off unless asked for:
`--agent-mode` (removed before clang sees the arguments) or `RUSTY_AGENT=1` (any other value changes nothing;
`--no-agent-mode` turns it off). `--version` prints clang's text unchanged and, only in agent mode, the line
`agent mode: yes, schema 1` and then `agent features: ...` listing what this build can do. `--help` in agent mode is clang's help
followed by a section on these options; without agent mode both are exactly clang's bytes. The variable is also passed on to clang's children, so the linker (rusty-ld) writes its own
JSON records; those lines pass through untouched.

Records are JSON, one per line, on standard error. Every record starts with `"schema": 1` and a `"type"`. The format is
shared with rusty-ld, rusty-grep and rusty-sh. Standard output and everything clang writes (objects, binaries) are not
changed. Agent mode only adds diagnostic-printing flags (`-fno-color-diagnostics -fdiagnostics-parseable-fixits ...`),
checked by tests to give byte-identical object files and binaries.

| Flag | What | In `--agent-mode` |
|---|---|---|
| `--diagnostics-format=json` | every diagnostic as a `diagnostic` record; `=text` turns it off again | yes |
| `--root-cause` / `--no-root-cause` | group follow-on errors under the first cause; compress long note chains | yes |
| `--fix-preview` | the machine fix-its as unified diffs in a `fix-preview` record; writes nothing | no |
| `--explain-missing-header` / `--no-explain-missing-header` | a `missing-header` record after each "file not found": where clang looked, where the header exists, the exact `-I`, pkg-config package, near-miss names. The file search stops after 1.5 s (`truncated: true`) | yes |
| `--build-summary` / `--no-build-summary` | the `summary` record last: exit code, elapsed ms, files compiled, error/warning counts (also the linker's), the clang command actually run | yes |
| `--stable-diagnostics` / `--no-stable-diagnostics` | deterministic output: clang runs with `LC_ALL=C`, records sorted by place within each translation unit, random temp-file names written as `XXXXXX` | yes |
| `--why-not-inlined=NAME`, `--why-not-vectorized=NAME`, `--why-not-unrolled=NAME` | a `why-not` record per question, from clang's optimization record (temp dir) | no |
| `--effective-flags` | an `effective-flags` record: target, sysroot, include dirs, defines, language, linker | no |
| `--agent-detail=brief\|normal\|full` | how much each record says (also `RUSTY_AGENT_DETAIL`; the flag wins; default `normal`) | normal |
| `--agent-help` | the options, record types, codes and rules of this tool as one JSON object on standard output (works without agent mode) | no |
| `run-san -- PROGRAM ARGS` | subcommand: run a program built with `-fsanitize=...` and turn its sanitizer reports into `diagnostic` records (code `C0400`) | no |
| `--compile-time-report[=N]` | a `compile-time` record per source file: slowest N (default 5) headers, templates, functions from `-ftime-trace` | no |

## Records file: `RUSTY_AGENT_OUT=<path>`
In agent mode, if this variable is set, every JSON record (including the linker's records that pass through, and `run-san`
records) is also appended to that file: one object per line, flushed per record, so a harness that merges standard output and
standard error still has a clean copy. The file is created if missing; its folder is not (a missing folder or an empty value
gives one `diagnostic` record with code `C0500` and severity `warning` on standard error, and nothing is written). The file is written
by this process only (clang's children, including the linker, do not inherit the variable, so no record is written twice). Never written when agent mode is off. Umbrella (`--agent-mode`,
`RUSTY_AGENT=1`): diagnostics, root cause, missing-header explainer, summary, stable.

## Records descriptor: `RUSTY_AGENT_FD=<n>`
Same records, same format and precedence, appended to descriptor `n`, which the caller opened and left open (for example `3>records.jsonl`
or a pipe). It is re-opened through `/proc/self/fd/n` as a new append handle, so the caller's descriptor is untouched and there is no
unsafe code. `n` must be a decimal number of 3 or more and open for writing. One write per record. If `RUSTY_AGENT_OUT` is also set, both
get every record. A bad value, a closed descriptor or a read-only one gives one `diagnostic` record with code `C0501` and severity
`warning` (on standard error and on the channel that works); the job and its exit code do not change. The compiler, linker and `run-san`
child do not inherit the variable. Ignored when agent mode is off (plain output unchanged).

## Detail levels and the size budget (`--agent-detail=`, `RUSTY_AGENT_DETAIL=`)
Agent mode is for programs and scripts that pay for every byte, so records come in three sizes (the same names and meaning as rusty-cargo).
An absent field means empty, zero or null.
- `full`: every field, as the first version wrote them (`rendered`, `files`, `symbols`, null and empty fields, the summary's `command`,
  all predefined macros). Keys start with `schema`, `type`, `code`, `severity`, `message`.
- `normal` (default): what is needed to act. No `rendered` (clang's own text, which repeats message, locations and notes; use `full` to see
  it), no `files`, `symbols`, no empty, null or zero fields, `end_line`/`end_column` when they equal the start, note locations that a
  `notes` entry already names, suggestions that only restate a fix-it, fix-its when `--fix-preview` carries the edits, the paths tidied
  (`/usr/bin/../lib/gcc/.../../include/x.h` becomes `/usr/include/x.h`). A `follow_on` entry leaves out `code`, `severity` and `message` when they are the root's own (the same name again). Lists are capped (`follow_on` 5, `notes` 6, `include_stack` 3,
  `searched` 8, sanitizer frames 8 per stack, `why-not` events 12, summary `compiled` 5) with a `<list>_omitted` count and a `more` pointer.
  The summary has no `command`, no zero counts and no `elapsed_ms` in stable mode. `effective-flags` has `defines.user` and
  `predefined_count` (all predefined macros are in `full`), no `cc1_args`, no linker library lists.
- `brief`: code, severity, message, flag, one `location`, one `fix` (the edits), counts of notes; the summary without file names.
Size budget (about 5 KB for a typical command): at `normal` and `brief`, once the records on stderr pass 12 KB only errors and requested
records go on, and past 20 KB nothing but the summary. The summary then has `records_omitted` (`by_type`, `more`). `RUSTY_AGENT_OUT` always
gets every record (shaped, without the budget); `full` has no cap. Records of other tools that pass through (rusty-ld) are not counted or
changed. `tests/sizes.py --check` fails when a record type or a job grows past its budget.

## Record `diagnostic`
`schema`, `type`, `code`, `severity` (`fatal`, `error`, `warning`), `message`, `files`, `symbols`, `notes`,
`suggestions` (`{message, option}`; from fix-its, `option` is null), `locations` (`{file, line, column, end_line,
end_column, role}`; `role` is `primary` or `note`), `flag` (the `-W` option or null), `fixits` (`{file, start{line,
column,offset}, end{...}, text}`), `rendered` (exactly what clang printed for this diagnostic and its notes),
`werror` (only when the `-W` option was made an error), `include_stack` (only when present).
Remarks (`-Rpass...`) have `type` `remark`. With `--root-cause`: `suppressed`, `follow_on` (`{code, severity, message,
flag, location, rule}`), `notes_omitted`.

Codes (set from the `-W` option, else from the start of clang's own message): `C0001` undeclared identifier,
`C0002` syntax error, `C0003` header not found, `C0004` implicit function declaration, `C0005` type or pointer
conversion, `C0006` unused name, `C0007` format string, `C0008` no matching function, `C0009` linker command failed,
`C0010` comparison problem, `C0011` bad or unused option, `C0012` input file missing, `C0013` deprecated,
`C0014` uninitialized, `C0015` unknown type or member, `C0016` return problem, `C0017` redefinition,
`C0018` static assertion, `C0400` sanitizer finding (run-san), `C0500` agent-mode setup problem (`RUSTY_AGENT_OUT`), `C0501` the same for `RUSTY_AGENT_FD`; any other: `C0100` error, `C0200` warning, `C0300` fatal.

## Root cause rules (`follow_on[].rule`)
`same-place` (identical diagnostic at the same place), `same-error-repeated` (the same undeclared name again),
`mentions-undeclared-name` (a later error names an undeclared name or unknown type), `same-line-as-unknown-type`,
`declared-with-unknown-type` (`foo_t x = 0;` with `foo_t` unknown never declares `x`: every later "undeclared identifier 'x'" is folded
under the unknown type),
`syntax-error-nearby` (another syntax error within 10 lines, same file). Warnings are never grouped except `same-place`.
Note chains: runs of more than 4 template/macro frames keep the first 2 and last 2; more than 3 overload candidates keep 3.
The full text stays in `rendered`; nothing is deleted from the record set, followers are listed in `follow_on`.

## Record `fix-preview`
`dry_run`, `written` (always false), `files` (`file`, `safe_edits`, `unsafe_edits`, `conflicts`, `diff`,
`diff_with_unsafe`), `edits` (`safety`, positions with byte offsets, `text`, `in_diff`, `in_diff_with_unsafe`).
Safe = a fix-it on an error or warning (what `clang -Xclang -fixit` applies); unsafe = a fix-it on a note (clang
suggests it, `-fixit` does not apply it). The safe diff equals the result of `clang -fixit` (tested). Apply with
`patch -p1` from the directory clang ran in.

## Record `missing-header`
`code` (`C0003`), `header`, `form` (`angle`, `quote`), `message`, `location`, `included_from`, `searched` (clang's include
search list from `-v`), `found` (`{path, kind: project|system, add_include_dir, option, rpm?, already_searched?}`),
`packages` (`{name, pc_file, cflags, option}` from pkg-config whose `-I` is that directory), `near_misses`
(`{path, distance, include_as, option?}`: case or up to 2 letters apart), `suggestions` (`{message, option}`; `option` is
the exact flag, e.g. `-Iinc`, or null). With nothing found the suggestion names `dnf provides '*/NAME'` but does not
run it. It searches the project folder (8 levels, hidden folders skipped), `/usr/include` and `/usr/local/include`.
Clang stops at the first missing header, so one is explained per run.

## Record `summary`
`exit`, `elapsed_ms` (not in stable mode), `command` (`full` only: the argument list as run, with the diagnostic flags agent mode adds;
private temp directories are written `<private temp dir>`), `records_omitted` (when the size budget cut records), `files`
(`compiled`, `linked`; from `clang -###`), `diagnostics` (`errors`, `fatal`, `warnings`, `remarks`, `records`, `suppressed`;
totals equal clang's own "N errors generated" even when `--root-cause` folds them), `linker` (`errors`, `warnings`
counted from the JSON records that passed through).

## Stable mode
`LC_ALL=C LANG=C` for clang, `LANGUAGE` unset; records sorted by (file, line, column, code, message), driver
diagnostics (no place) last, translation units keep their input order; names like `/tmp/x-1a2b3c.o` become
`/tmp/x-XXXXXX.o` in records and passed-through text. No dates, times or elapsed times in records (the summary has no `elapsed_ms`; `--no-stable-diagnostics` brings it back). The
`compile-time` record is timing by definition and is the one record whose numbers differ between runs.

## Record `why-not`
`what` (`inlined`, `vectorized`, `unrolled`), `function`, `optimization` (the `-O` option or null), `outcome`
(`not-optimized`, `optimized`, `no-data`), `summary`, `events`, `suggestions`. Inline events: `role` (`callee`: calls of
NAME; `caller`: what NAME calls), `status`, `reason` (clang's name, e.g. `TooCostly`, `NoDefinition`, `NeverInline`),
`message`, `location`, `details` (`cost`, `threshold`, `callee`, `caller`). Loop events: `role` `loop`, `status`
(`vectorized`/`not-vectorized`, `unrolled`/`not-unrolled`), `location`, `function`, `reasons`. A loop inlined into several
callers is reported once. Needs `-O1` or higher and exactly one source file; otherwise `outcome` is `no-data` and
`summary` says why ("1 loop in 'f' was not vectorized; 1 other was vectorized"). The optimization record goes to a private temp
directory and is deleted; the record names none of it.

## Record `effective-flags`
`clang` (`version`, `path`, `config_file`, `gcc_installation`, `default_target`), `target` (`triple`, `cpu`, `features`),
`sysroot`, `resource_dir`, `language` (`x`, `std`), `optimization`, `debug_info`, `position_independent`,
`include_dirs` (`search` in order, `quote_only`, `from_dash_I`, `system`), `defines` (`user`, `predefined_count`, `all`),
`linker` (`path`, `resolved` (symlinks followed), `version`, `fuse_ld`, `library_dirs`, `libraries`, `dynamic_linker`),
`cc1_args`. With no input file nothing is compiled and an empty C file is described.

## Record `compile-time`
`file`, `total_ms`, `phases` (`frontend_ms`, `backend_ms`, `header_parse_ms`, `instantiate_*_ms`, `optimize_ms`,
`codegen_ms`; they overlap), `slowest` (`headers`: `{path, self_ms, inclusive_ms, count}`; `templates`: `{name, kind,
self_ms, inclusive_ms, count}`; `functions`: `{name, optimize_ms, codegen_ms, total_ms}`), `suggestions`. Function names are demangled and header paths tidied (`/usr/include/c++/16/bits/x.h`). Self time
is the time minus nested items. The trace uses clang's 100 microsecond granularity unless you give
`-ftime-trace-granularity=`; it is written to a private temp directory and deleted.

Objects and binaries are byte-identical to plain clang in all these modes (tests compare hashes).

## `run-san`
`rusty-cc run-san -- ./program args` runs the program with `ASAN_OPTIONS`, `UBSAN_OPTIONS`, `TSAN_OPTIONS`, `MSAN_OPTIONS`
and `LSAN_OPTIONS` set to `color=never` (UBSan also `print_stacktrace=1`); your own values of these variables come after ours and
win. Standard output and stdin are the program's own. Standard error is read to the end: each sanitizer report becomes one
`diagnostic` record with code `C0400`, severity `error`, and everything else the program wrote to standard error passes through
unchanged, in order. `sanitizer` holds `name`, `kind` (`heap-buffer-overflow`, `heap-use-after-free`, `double-free`, `memory-leak`,
`signed-integer-overflow`, ...), `access`, `size`, `address`, `region` (`offset`, `relation`, `size`), `summary` and `stacks`
(`role`: `access`, `allocated`, `freed`, or the leak line; frames with `index`, `function`, `file`, `line`, `column`, `module`,
`offset`). File names are relative to the current directory when they lie under it (the same way the compile records name files).
`locations` have roles `primary`, `allocated`, `freed`; `notes` name the user frames only (the runtime's and libc's frames
stay in `rendered`, which is the sanitizer's own text including shadow bytes). A last `summary` record has `mode: "run-san"`,
`exit`, `sanitizer_reports`. The exit code is the program's. ASan stops at its first error, UBSan reports and continues. Reports
from ThreadSanitizer are read by the same rules but are not covered by tests.

## Output to a closed pipe
`rusty-cc --agent-help | head -c 10`, and the other texts printed on standard output (`--version`, `--help` in agent mode), stop quietly when
the reader goes away: no panic message, exit code 0.
