# P.O.C. — Project · Operation · Compose

[![CI](https://github.com/xiaozhao45/poc/actions/workflows/ci.yml/badge.svg)](https://github.com/xiaozhao45/poc/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**P.O.C.** is a userspace, fully local version control tool: a single binary with no network, no daemon, and no staging area. It maintains complete history for a directory just like Git does — but the history model is different: history is a **stack of freely reorderable Operations**, not a chain of commits.

## Why P.O.C.

- **Stack-based history.** An *Operation* (op) is the smallest self-contained change: a content-addressed pair of before/after snapshots, bound to no parent. A *Compose* is an ordered stack of ops — like a branch, but one you can reorder, squash, amend, fork, and merge without fear.
- **Free pool.** Ops that don't belong to any stack live in a per-project free pool, always addressable by id and ready to be pushed onto any stack.
- **Non-destructive by construction.** The `poc` binary contains no destruction code path at all. Squash, amend, merge, fork — none of them change head or lose information: every replaced op simply drops into the free pool. Combined with the append-only `poc log`, any history shape can be rebuilt by hand.
- **Conflicts never touch your working tree.** When an operation can't resolve automatically, it parks a pending step and exits; conflicts are resolved in swap files under `.poc/swap/`, then replayed atomically. Your working tree stays clean until you commit the step.
- **Danger is quarantined.** All destructive verbs live in a separate binary, `dpoc` — which you can simply not install. It needs explicit enablement, an interactive terminal, and full-id confirmation; there is no `--yes` bypass. Deletion is two-stage (attic → purge) and recoverable in between.
- **Boring, solid plumbing.** Line-level diffs, a single-file store (redb) with BLAKE3 content addressing, append-only journaling in the same transaction as every change, automatic TTY/pipeline color handling, and built-in English/Chinese messages.

## The four binaries

| Binary | Purpose |
|---|---|
| `poc` | Everyday, safe operations — all you need to work. |
| `dpoc` | Dangerous operations and their safety net (attic, restore, verify, audit). |
| `cpoc` | Link-based collaboration across projects (placeholder; not implemented yet). |
| `fpoc` | `poc` + `dpoc` combined: dispatches on the first word, convenient to alias as `poc`. |

## Install

Build from source with Rust 1.90 or newer:

```console
$ git clone https://github.com/xiaozhao45/poc.git
$ cd poc
$ cargo install --path .
```

This installs `poc`, `dpoc`, `cpoc`, and `fpoc` into `~/.cargo/bin`.

## Quick start

```console
$ poc proj -N demo            # create directory `demo` and initialize (compose: main)
$ cd demo
$ poc config user.name "Your Name"
$ poc config user.email "you@example.com"
$ echo "# demo" > README.md
$ poc opt -M "init"
  1 added
  created operation 3f9c0a1b "init"
$ poc show
compose main (current)
  base:  b18b177d
  head:  6115a54d

stack (top first, 1 total):
  3f9c0a1b  "init"
    Your Name <you@example.com> · 2026-10-02 11:04
```

The daily loop is just: edit files → `poc opt -M "message"` → keep working. There is no staging area — what gets recorded is what's on disk.

## Cheat sheet

```console
# Try a change on a disposable stack; the stack survives if you switch back later
$ poc cmp -N try --fork
$ ... edit ... && poc opt -M "try x"
$ poc cmp main

# Cherry-pick an op from another stack onto the current one
$ poc opt <op-id>

# Squash the three ops at the top of the stack into one
$ poc opt <idA> <idB> <idC> -c -M "squash"

# Undo the top of the stack (the change drops into the free pool, never lost)
$ poc opt <top-id> -f

# Preview any rewriting command without touching anything
$ poc --dry-run <command>
```

## Documentation

- [docs/USAGE.md](docs/USAGE.md) — the full manual: concepts, every command, the step protocol, and the `dpoc` safety net.
- [docs/DESIGN.md](docs/DESIGN.md) — the design document: goals, storage layout, algorithms, and decision log.
- [docs/OUTPUT-DESIGN.md](docs/OUTPUT-DESIGN.md) — output and rendering conventions.
- [docs/acceptance-demo.sh](docs/acceptance-demo.sh) — an assertion-based acceptance demo of the seven 1.0 goals (`bash docs/acceptance-demo.sh` after `cargo build --release`).

## Configuration and localization

- `poc config user.name / user.email` per project; falls back to `GIT_AUTHOR_NAME` / `GIT_AUTHOR_EMAIL`, then to your Git global config.
- Ignore rules go in `.pocignore` (glob syntax; `.gitignore` is not read).
- Interface language: `POC_LANG` (`en` by default, `zh-CN` built in); the chain is `POC_LANG → LC_ALL → LC_MESSAGES → LANG`.
- Colors are managed automatically: kept on TTYs, stripped in pipes; `NO_COLOR` is honored.

## Status

This is an early public release of a personal tool, offered as-is. `cpoc` (link-based collaboration) is designed but not implemented. There is no network protocol by design — P.O.C. is local-first, end to end.

## License

Distributed under the [MIT License](LICENSE).
