# Actor model + Nuker GC — implementation plan

Status: **PROPOSED** (not implemented). Decision record + build plan
for per-actor isolated heaps with O(1) teardown. See §0 for why
GC-managed handles were cut instead of built.

## 0. Verdict: GC handles are cut (measured)

**Would a GC handle put overhead on? Yes — three times over — so it
is cut, not shrunk.** `docs/CPP-INTEROP.md` §6 updated accordingly.
Handles stay manual (`Class_delete`) and process-global; the Nuker
never frees them (§4).

Measured on this box (MinGW gcc 15.2 `-O2`, 2M × 32-byte boxes —
`Value`-sized — 3 stable runs, `C:\Users\ITER\AppData\Local\Temp\opencode\bench_alloc.c`):

| op | ns/op |
|---|---|
| `malloc` + `free` | ~50 |
| `malloc`, never freed (today's leak-by-design default) | ~60 |
| bump pointer over preallocated block (arena model) | ~8 |

Takeaways: today's default already pays full `malloc` per box with
zero reclamation, and bump allocation is **~7× cheaper**. A tracing
collector can only move the wrong way from there.

Structural costs of GC-managed handles (finalizer model), beyond the
numbers:

1. **Per-handle registration** — finalizer tables are hashtable
   insert + lock per handle, paid on every object birth, plus
   nondeterministic destruction (bad for scarce resources; fine for
   memory, but pointless next to a Nuker).
2. **Infection** — a finalizable box must be collector-allocated, or
   the collector can't see it. Sound mixing of leak-`malloc` boxes
   with finalizable handles is fragile; realistically *all* `Value`
   boxes move onto Boehm: slower allocation path, conservative
   retention bloat, mark-phase pauses.
3. **Unverifiable here** — Boehm is not installed on this box (no
   `gc.h`, no `libgc`), so the dependency the feature would rest on
   can't even be exercised by our gates.

The refcounting alternative is no better: atomic inc/dec per handle
op, retain/release insertion throughout codegen, and cycles leak
anyway.

Shrunk middle path (recorded, not built): deterministic per-arena
finalizer lists run at nuke time (`cdi_arena_on_nuke(fn, arg)`) —
automation with zero steady-state overhead and no nondeterminism.
Fast-follow candidate once actors land, if manual `delete` proves
painful in practice.

## 1. Model

- An **actor** = one isolated heap (arena block chain) + one mailbox
  (FIFO) + one entry (top-level 1-param proc) + one registry slot
  (index + generation).
- **Cooperative, single-threaded, deterministic FIFO scheduling.**
  An actor runs until `recv` on an empty mailbox (yields), `exit`,
  or falling off its entry (done → **nuke**). Determinism is a
  requirement, not an accident: it makes goldens meaningful.
- **Isolation** = separate arenas + copy-on-send across heaps.
  Same-heap send moves (no copy). Handles (`machine.alloc`
  registry, C++ objects) are process-global by design: copied
  shallow, owned by nobody, never nuked.

## 2. Surface (CDirac C-style, `machine` namespace)

```c
fn worker(arg|int) | int {
  var msg = machine.recv();
  machine.send(msg.from, arg + 1);   // exact sugar TBD at build time
  return 0;
}
var h = machine.spawn("worker", 41);
displayln(machine.recv());
```

Planned ops (all `machine`-only, like `alloc`/`dealloc`):

| op | shape |
|---|---|
| `machine.spawn("name", arg)` | entry name must be a string literal; resolves at compile time to a top-level 1-param proc; yields an actor handle |
| `machine.send(h, msg)` | deep-copies `msg` into the recipient's heap (move if same heap); send-to-dead aborts |
| `machine.recv()` | dequeue own mailbox, or yield; zero args |
| `machine.self()` | own handle; zero args |
| `machine.exit()` | done; zero args (falling off the entry is the same) |
| `machine.yield()` | requeue self at tail; zero args (starvation guard for long compute loops) |

Rules: entry must be a **top-level** proc of **exactly 1** parameter
(nested needs env — same rejection shape as callbacks). `main` is
actor 0 with a permanent heap and a working mailbox, so there are no
"called outside an actor" special cases; exiting in main ends the
program rc 0.

## 3. Runtime (`llvmrt/actor.{h,c}`, plus hooks)

- **Arena**: `{base, ptr, end, next}` blocks (default 64 KiB, geometric
  growth); 8-byte aligned bump; overflow chains a new block.
  `cdi_arena_realloc` = alloc + memcpy (no in-place; a tail fast-path
  is optional later).
- **Reroute**: `CDI_MALLOC`/`CDI_REALLOC` (value.h) become
  `cdi_heap_alloc`/`cdi_heap_realloc`, consulting global
  `cdi_current_arena` (`NULL` → `malloc`/`realloc` fallback, so
  standalone programs behave exactly as today). All ~25 box/payload
  sites funnel through the two macros — **no call-site changes**.
  Cost: one predictable branch (~1 ns) to buy ~50 ns per box.
- **Registry**: slots `{heap, mailbox, state, generation}`; handles
  pack `(index, generation)` into pointer bits (never dereferenced).
  Slot reuse bumps generation — no use-after-nuke, ever.
- **Mailbox**: intrusive FIFO nodes allocated *in the actor heap*
  (freed by the nuke; dequeue never frees). Unbounded (documented).
- **Scheduler**: global run queue; `cdi_actor_run()` drains it until
  no runnable actor remains; remaining blocked actors are nuked at
  program end for hygiene. Emitted into `main` only when the program
  contains a spawn (byte-identical IR otherwise — project invariant).
- **Deep copy** `cdi_copy_to(heap, v, map)`: fresh box + struct copy,
  then per-tag payload (`str`: fresh bytes; array/queue: fresh items,
  registered in the map *before* recursing so **cycles** terminate);
  `ptr` boxes copy shallow (global handles). Linear visited map —
  mailbox payloads are small; note the O(n²) worst case in code.
- **Nuke** `cdi_actor_nuke()`: free every arena block, drop mailbox,
  mark slot dead. O(blocks) — O(1) typical. Never touches handle
  targets.
- **Accounting**: `bytes_used` per heap, always on (one add per
  alloc; enables future limits/debugging).

## 4. Codegen (`llvmgen`, minimal)

- `MACHINE_ONLY` += the six ops; `rt_name` + `declare` entries;
  `closures.rs` globalish list alongside `alloc`/`dealloc`.
- `machine.spawn("name", arg)`: first arg must be a `Str` literal
  (never evaluated as a value); resolve to top-level proc at compile
  time, require exactly 1 param, emit
  `call ptr @cdi_machine_spawn(ptr @cdi_proc_X_N, ptr %arg)`.
  (Same function-address-as-`ptr` pattern as callback trampolines.)
  Unknown/nested/arity violations get tailored errors reusing the
  `has_nested_proc` helper shape from callbacks.
- The six declares + the scheduler call emit **only when an actor op
  appears in the program** (callbacks precedent) — actor-free IR stays
  byte-identical.
- **No proc-codegen changes**: 1-param procs already compile to
  `define ptr @…(ptr %p0)` — directly C-callable as
  `Value* (*)(Value*)`.

## 5. Semantics & errors

Follow the `machine.alloc` registry precedent (linear scan, abort
with `Error: …`, exit 1 — the `BracketError` shape `llvmcheck`
already byte-gates on nonzero exit):

| situation | result |
|---|---|
| spawn non-literal / unknown / nested / arity≠1 | compile error at lowering |
| send to dead actor (generation mismatch) | `machine.send: dead actor` abort |
| send non-handle | tag-check abort (`expects an actor handle` shape) |
| cyclic message | terminates via copy map (tested, not rejected) |
| all actors blocked forever | scheduler drains; program exits (documented, not an error) |
| `set`/mutation of received values | ordinary boxes; aliasing rules unchanged |

## 6. Long-lived actors (explicitly v1-out)

No tracing collection inside arenas: actors are units of work,
finish → nuke. Long-lived servers grow via chained blocks. Two
honest mitigations, both documented not built: the **restart
pattern** (spawn a replacement; Erlang-style) and, later, a
per-actor mark-sweep — which would need stack cooperation and
reintroduce exactly the overhead/dependency §0 cut, so it stays a
sketch until demand proves otherwise.

## 7. Overhead budget (measured + estimated)

| op | today | with Nuker |
|---|---|---|
| box alloc | ~55 ns malloc | ~8 ns bump + ~1 ns dispatch (measured) |
| send, same heap | n/a | pointer enqueue, ~10 ns (est.) |
| send, cross heap | n/a | deep copy O(message) |
| actor teardown | n/a (leak) | O(blocks), 1 block typical |
| finalizer path (CUT) | — | registration + lock + nondeterministic sweep + global Boehm |

## 8. Test plan

- `llvmrt/tests/test_actor.c` (+ `run_tests.py` entry): arena
  alloc/alignment/growth, `bytes_used` accounting, deep copy incl.
  **cycles** (`put` array into itself), same-heap move, shallow
  handle copy, generation mismatch unit coverage where non-aborting.
- `filetest/ok`: spawn/send/recv shape lowers (+ `is_atom`-style?
  no — keep: one shape file). `filetest/err`: spawn non-literal,
  unknown proc, nested entry, arity≠1.
- `tests/*.rs` lowering-shape test mirroring `machine_memory_ops`
  (declares + `call ptr @cdi_machine_spawn(ptr @cdi_proc_`).
- `llvmcheck` golden: `examples/actors/pingpong.cdc` +
  `.expected` via `--native-dir` — requires relaxing the
  shim-required rule (tools/llvmcheck.py:298-301) so shim-less
  goldens take the plain clang flow (missing-`.expected` FAIL stays
  the guard). Deterministic FIFO order is an acceptance criterion.
- Byte-identical IR for actor-free programs (old-vs-new differential
  like P5/P11).

## 9. Work breakdown

1. `llvmrt/actor.{h,c}`: arena, registry+generations, mailbox,
   scheduler, deep copy + cycle map, nuke; reroute macros in
   `value.h`; `llvmrt/tests/test_actor.c` + runner entry.
2. `llvmgen.rs`: six machine ops (list + `rt_name` + declares,
   conditional emission), spawn literal resolution + trampoline-free
   address passing, tailored errors; `closures.rs` literals.
3. `filetest` ok/err + `tests` shape test + `examples/actors/
   pingpong.cdc` + golden + llvmcheck shim-rule relaxation.
4. Docs: `CPP-INTEROP.md` actor section (or new `docs/ACTORS.md` if
   it outgrows a section), `ERRORS.md` rows, `BUILD.md` row, PHASES
   entry; full verify (suite + fmt + clippy + 18+1 gate).

## 10. Non-goals (v1)

Threads/TLS (design keeps the path open: `cdi_current_arena`
becomes `_Thread_local` later — near-one-line change), GC
handles/finalizers (cut, §0), intra-arena tracing GC, bounded
mailboxes, join/select, distribution, nested-proc entries,
`machine.spawn` with computed (non-literal) names.
