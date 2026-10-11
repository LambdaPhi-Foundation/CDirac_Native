# C++-first native interop (LLVM backend)

Status: ACTIVE (v1). C++ headers are first-class Dirac: declare them,
then call them like ordinary code. Thin C headers ride the same path
with a few decls; the focus is C++.

## 0. Idea

`llvmgen` never learns C++ name mangling, overload resolution, or
templates. Instead it collects your declarations and generates a C++
**shim** (`cdirac --emit-cpp-shim`) that includes the real headers and
exposes stable `extern "C"` wrappers (`cdi_wrap_N`) over boxed
`Value*`. The LLVM IR only calls those wrappers; the host C++ compiler
(`g++`/`clang++`) resolves everything inside the shim. If the shim
doesn't compile, your declarations are wrong — the C++ compiler is the
type checker.

```
FILE.cdc --emit-llvm--> out.ll --clang++--> prog
         --emit-cpp-shim--> cdi_shim.cpp --g++ -c--> cdi_shim.o --┘
                              + llvmrt/*.c o
```

## 1. Directives (bracket-form, top-level only, emit no code)

```racket
[include-c "string.h" "stdlib.h"]        ; thin C headers
[include-cpp "<vector>" "widget.h"]     ; C++ headers (<...> or "..." as written)
[link-native "-lm" "-L/opt/x/lib"]      ; link flags ([link-c]/[link-cpp] aliases)
```

## 2. Declarations

```racket
[extern-c cstrlen "strlen" [string] int]     ; C free function
[extern-c puts [string] int]                 ; alias == symbol shorthand
[extern-cpp add2 "demo::add" [int int] int]  ; C++ free function (qualified Str)

[cpp-class Accum "demo::Accum" "accum.h"]    ; opaque class: Alias, Qualified, header
[cpp-class Plain "plain.h"]                  ; 2-arg: Str with `::` = qualified, else header
[cpp-ctor Accum [int]]                       ; constructors (overloads: one per arity)
[cpp-method Accum push [int] void]           ; instance methods
[cpp-static Accum make [int] Accum]          ; static methods
[cpp-field Accum size int]                   ; instance fields (std::string/int/double/bool/ptr/class)
```

Types in decls: `int double bool string void ptr` plus registered
class aliases. `cpp-const` types are `int double bool string` only
(consts are plain values). Mapping (v1): `int`->`int32_t`/`int`,
`double`->`double`, `bool`->`bool`, `string`->`const char*` (C) /
`std::string` (C++), `void`->`void`, `ptr`->`void*`,
`Class`->`Class*` (opaque handle, `CDI_PTR`). C++ class results must
be pointers; value-returning factories need a header-side function
returning a pointer. Enum/const snapshots cast (`(int32_t)` etc.),
so scoped enums and macros of any arithmetic type work; integer
consts must fit Dirac `int32`.

## 3. Calls (first-class Dirac, both surfaces)

```c
var a|ptr = Accum(10);   // construction (arity picks the overload)
a.push(5);               // instance method (dot sugar over the handle)
var b|ptr = Accum.make(100);  // static
displayln(a.size);       // field read
set a.size = 3;          // field write
displayln(Accum_push(a, 5));  // explicit alias (also the ambiguity escape hatch)
Accum_delete(a);         // frees (auto-registered per class)
displayln(RED);          // named constant reads as a value (RED() also works)
displayln(Widget.VERSION);  // static data member read
set Widget.VERSION = 3;  // static data member write
displayln(Widget_VERSION_sget());  // explicit static aliases
```

Constants (`cpp-enum`, `cpp-const`) are snapshots taken per call —
a `#define` that changes between calls (rare; think `errno`-style)
reads fresh each time, since the wrapper re-evaluates it. They are
read-only: `set RED = 5` fails with `cannot assign to constant`.

Rules that fit in your head:

- A Dirac `proc`/`var` literally named like an alias or class wins
  (lookup-first); only on lookup failure does native dispatch run.
- `Class(args)` with no matching ctor arity is an arity error naming
  the expected arities. Bare `Class` as a value is an error pointing
  at `Class(...)`.
- Instance sugar needs a bound handle and a declared method name.
  Same method name on two classes + sugar call = ambiguity error;
  call `Class_method(recv, ...)` explicitly.
- `Class.method(...)` on an instance-only name hints at the handle
  form. `Class.field` reads static data members declared with
  `[cpp-static-field ...]`; unknown statics name the directive, and
  an instance-only name hints at the handle form. Trailing `const`
  on field directives marks read-only members (writes fail with
  `cannot assign to constant`).
- Handles are unmanaged: the box owns nothing. `Class_delete`
  (or an explicit `free`-style extern) is the story; `machine.alloc`
  stays for raw Dirac-side allocation.
- One namespace with procs/vars/builtins: collisions are compile
  errors. C `extern-c` aliases must be unique (C has no overloads);
  C++ decls may overload by arity.

## 4. Errors

- Compile time: bad decl shapes, `unknown native type`, unknown
  `cpp-class` in member decls, duplicates, collisions, arity
  mismatches (`expects A or B (got C)`), unknown/ambiguous members,
  directives inside proc bodies (`must be top-level`).
- Runtime (`BracketError` shape: `Error: ...` on stderr, exit 1):
  conversion failures (`cannot pass ...`), null handle misuse,
  `expects a native handle`. Unchecked host (C++) faults stay loud.

## 5. Build

```sh
cdirac --emit-llvm FILE.cdc > out.ll
cdirac --emit-cpp-shim FILE.cdc > cdi_shim.cpp
g++ -std=c++17 -c cdi_shim.cpp -o cdi_shim.o -I llvmrt -I <your-include-dirs>
clang++ out.ll llvmrt/value.c llvmrt/arith.c llvmrt/strings.c \
  llvmrt/containers.c llvmrt/cinterop.c cdi_shim.o -o prog <link-native flags>
```

The `--emit-llvm` header comment prints the exact line (with your
`link-native` flags appended). Without `clang`, `g++ -fsyntax-only`
on the shim still proves declarations resolve against the headers.

Floor: `clang` ≥ 15 (opaque-pointer-only IR), C++17 for the shim,
C11 + `-fwrapv` for `llvmrt/*.c`. Verified end-to-end on clang
23.1.2: `accum` prints `22 2 100 42 HELLO`, `cbasic` prints `5 42`.

## 6. Non-goals (v1, updated)

~~C varargs (`printf(...)`)~~ DONE (§8): trailing `varargs` on extern
decls, fixed + up to 4 extras with tag dispatch. ~~Struct-by-value
params~~ DONE (§9): `[cpp-struct ...]` bridges aggregates by value.
~~Function pointers~~ DONE (§10): `[callback ...]` exposes Dirac
procs as C function pointers via LLVM trampolines.
Still out: union/bitfield params, member signatures under
`@typecheck`. STL containers are
bridged explicitly (see §7) rather than implicitly. Each remaining
item needs a 2-line header-side wrapper today.

> GC-managed handles are CUT, not deferred (see `docs/ACTOR-GC.md`
> §0): finalizers cost per-handle registration plus nondeterministic
> destruction and would force Boehm onto every box; handles stay
> manual (`Class_delete`) and process-global, and the Nuker never
> frees them.

## 7. STL bridging (toward retiring built-in containers)

`[cpp-vector Alias elem]` bridges one `std::vector<E>` as an opaque
Dirac handle with the Dirac-array-shaped API. Status: ACTIVE (v1);
`for x in v` does NOT apply (it lowers to the array builtins) — use
index loops over `v.len()`.

```racket
[include-cpp "<vector>"]
[cpp-vector IntVec int]       ; elem: int | double | string | ptr | Class
[cpp-vector StrVec string]
```

Desugar (at `collect_native`): a class entry qualified
`std::vector<E>` + `0`-arg ctor + n-ary `VecBuild` + methods
`len/get/put/push/pop/str` + `Alias_delete`:

| Dirac | C++ |
|---|---|
| `IntVec()` / `IntVec(1, 2)` | fresh `new std::vector<E>` (note: `IntVec(5)` is `[5]`, never a size) |
| `v.len()` / `IntVec_len(v)` | `self->size()` → Dirac `int` |
| `v.get(i)` / `IntVec_get(v, i)` | bounds-checked `(*self)[i]`; failure aborts `Alias_get: index %d out of range for length %d` |
| `v.put(i, x)` | bounds-checked store |
| `v.push(x)` / `v.pop()` | `push_back` / back+`pop_back` (`empty vector` aborts) |
| `v.str()` | Dirac write spelling via `cdi_write_cstr` (e.g. `[9, 2, 3]`) |
| `IntVec_delete(v)` | `delete self` (handles are unmanaged; the box owns nothing) |

Element map: `int`→`int32_t`, `double`→`double`,
`string`→`std::string`, `ptr`→`void*`, `Class`→`Class*`.
Bans (compile errors): `bool` (`std::vector<bool>` is bit-packed —
use `int` 0/1), `void`, nested vectors, unknown classes.
Calls dispatch like other natives: dot sugar when unambiguous
(single class registering the method), else explicit `Alias_op`
aliases. See `examples/cpp/vecs.cdc` (expect
`3, 1, 4, 9, 4, [9, 2, 3], ["a", "b", "c"], 0, 3, [7]`).

## 8. C varargs + struct pointers (v2a)

```racket
[include-c "stdio.h"]
[extern-c cprintf "printf" [string] int varargs]   ; or extern-cpp
```

A trailing `varargs` covers Dirac arities `fixed..=fixed+4` over the
n-ary `(Value**, i32)` shape (one wrapper, `declare ptr @cdi_wrap_N(ptr,
i32)`). The shim unboxes fixed params normally, then forwards extras
by runtime tag: `int|bool`→`int32_t` (C promotions), `double`,
`string`→`char*` (C) / `std::string` (C++), `ptr`→`void*`. Anything
else aborts `cannot pass vararg`; more than 4 extras aborts `at most 4
varargs`; fewer than fixed fails statically (`expects K or more`).
One varargs decl per alias (it covers the whole range). See
`examples/cpp/printf.cdc`.

Structs ride the class bridge by pointer (no by-value ABI):

```racket
[cpp-class Point "demo::Point" "point.h"]  ; struct == class w/ public fields
[cpp-ctor Point [int int]]                 ; needs a real C++ ctor
[cpp-field Point x int]                    ; field read/write
[extern-cpp describe "demo::describe" [Point string] string]
```

Rules: value-returning factories need a header-side function returning
a pointer (`static Point* make(...)`); `ptr` params stay `void*`
(use the class type for typed pointers). See
`examples/cpp/point.h` + `point.cdc` (expect `25, 0, 7, P(3,4)`).

## 9. Structs by value (v2b)

```racket
[cpp-struct Point "demo::Point" "point.h" [x int] [y int]]
[cpp-struct Labeled "demo::Labeled" "point.h" [at Point] [tag int]]
[extern-cpp move_by "demo::move_by" [Point int int] Point]
```

A struct value **is** its field array on the Dirac side
(`array(3, 4)` for a two-int struct — no constructor sugar). Params
unbox field by field; returns build the array the same way:

```c
var p = move_by(array(3, 4), 10, -2);  // p == [13, 2]
displayln(manhattan(p));               // 15
```

Rules that fit in your head:

- Shape is fixed: `Alias Qualified Header [field type] ...` with at
  least one `[name type]` pair. Field types: scalars, `ptr`, class
  aliases (as handles), and earlier-registered structs (nesting must
  be acyclic — use-before-declare reads as `unknown native type`,
  like classes). `void` is not a field type.
- The C++ type must be an **aggregate** matching the declared field
  order: the shim builds `Qualified{f0, f1, ...}` (brace init —
  paren-init of aggregates is C++20, the floor is C++17) and reads
  `.field`. Anything else fails in the shim with the host
  compiler's error, as usual.
- Padding never matters (conversion is field-by-field, never memcpy).
  `string` fields map to `std::string`/`char*` like params;
  `bool` is a plain field (the `vector<bool>` ban does not apply).
- Mismatched shapes abort at runtime in conversion-failure shape:
  non-arrays (`cannot pass value as S (expects array of K fields)`)
  and wrong lengths (`cannot pass array as S (expects K fields)`).
- One namespace, as ever: struct aliases collide with classes,
  aliases, vectors, builtins, procs, and vars. Bare `S` as a value
  is an error (`is a cpp-struct, not a value`). Structs are not
  vector elements and not `cpp-const` types.
- The §8 pointer pattern still stands for non-aggregates (private
  fields, user ctors): bridge by pointer instead.

See `examples/cpp/struct.h` + `struct.cdc` (expect `15`,
`[[13 2] 7]`).

## 10. Function pointers (v2c)

```racket
[callback Inc [int] int]
[extern-c apply_twice "apply_twice" [Inc int] int]

fn inc(x|int) | int { return x + 1; }
displayln(apply_twice(inc, 41));   ; 43
```

A `[callback Alias [params] ret]` decl exposes a top-level Dirac
proc under a C signature; the alias doubles as a function-pointer
*type* in extern decls. Bare `inc` above reads as the trampoline
address (boxed handle) — the one way a proc name is a value.

How it works: `llvmgen` emits one LLVM trampoline per decl
(`define i32 @cdi_cb_inc_N(i32 %a0)` — C ABI in, boxed proc call,
unboxed result) and the shim `reinterpret_cast`s the handle back to
the declared signature. No `libffi`, no name mangling: the trampoline
lives in the same LLVM module as the proc, so no cross-TU naming.

Rules that fit in your head:

- The target must be a **top-level** proc with matching arity (types
  are unchecked, as ever — only arity). Missing target, nested
  target, and mismatches are compile errors naming the alias.
- Signatures stay scalar: `int double bool string ptr` params,
  plus `void` returns. (`bool` is one C byte; `string` is `char*`
  in C decls, `std::string` in C++ — matching the trampoline's
  box/unbox pair. Struct/class/func sig types are a follow-up.)
- Callbacks compose with everything taking args: `extern-c`,
  `extern-cpp`, methods, field setters — anywhere a param type can
  appear. Raw `ptr` values still pass through where a `Func` is
  expected (unchecked escape hatch).
- One namespace, as ever: callback aliases collide with classes,
  structs, aliases, vectors, builtins, and vars — but a same-named
  top-level `proc` is *required*, not a collision.
- Don't pass callbacks as varargs extras: the tag switch has no
  function case, so they forward as raw addresses.
- Nested procs can't be targets (a trampoline has no env to pass);
  same-named nested procs shadow calls but not the trampoline value.

See `examples/cpp/cb.h` + `callback.cdc` (expect `43`, `5`, `4`,
`7`, `7`).
