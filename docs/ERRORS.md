# Error catalog — `cdirac` static errors (all `line N:`)

Every entry below fails `compile_llvm` (and therefore `build`/`llvmcheck`)
before any code runs. Runtime failures keep the `BracketError` shape
(`Error: …` on stderr, exit 1) and are byte-gated by `llvmcheck`.

## 1. Lexer (migration hints)

| Message | Trigger | Fix |
|---|---|---|
| `unterminated string` | `"` without close | close the string |
| `unknown #-literal` | `#t` etc. | use `true`/`false` |
| `unexpected '&' (use &&)` | single `&` | `&&` |
| `found '--' (use //)` | `-- comment` | `// comment` |
| ``found '`' (no quote)`` | `` `x `` | `"x"` |
| `found ''' (use "...")` | `'s'` | `"s"` |
| `unexpected character` | e.g. `\`, `@` outside attrs | check surface (`CDirac.md`) |

## 2. Parser (surface shape)

`expected ';' / '{' / ']' / type`, `keyword as name`, `__for_ reserved`,
`bare =`, `: vs .`, `then/end/do/done`, `and/or/not (use &&/||/!)`,
`new (drop it)`, `elif (use else if)`, `only one dot`,
`never is not valid annotation`, `unknown type`, `only [array/queue T]`,
`for-in: of/parens rejected`. All point at the leading token.

## 3. Core static checks (`llvmgen`, no checker crate)

Types (`x|int`) are parsed but **unenforced** (all values are boxed
`ptr` at runtime). These are enforced:

| Message | Meaning |
|---|---|
| `unbound identifier: X` | no `var`/`const`/`proc`/builtin/native in scope (bare `alloc` → use `machine.alloc`) |
| `<op>: expects K argument(s), got N` / `expects A or B (got C)` | exact arity (n-ary: `+ - * / = < > <= >= array max min append queue error`) |
| `set-before-def` / `cannot assign to constant` | `set` needs a prior `var`; `const`/named natives read-only |
| `cannot redefine … as a procedure/variable` | one namespace for procs/vars/natives/builtins |
| `return/break/continue` placement | outside `fn` / loop |
| `procedures must be top-level` | `Proc` node in *value* position (statement-position nested defs lift — see `docs/CLOSURES.md`) |
| `duplicate procedure 'X'` (nested too) | same-frame redefinition |
| `cannot redefine 'X' as a procedure` | nested `fn` colliding with a live local |
| `cannot capture 'X' here (out of scope)` | capture slot exited (block over) at the call |
| nested call-before-def / mutual recursion | `unbound identifier` (incremental rule, like top level) |
| `value-if with statement prefix` | `[if … [set …] …]` in value position |
| `unknown machine operation 'X'` | closed set: all builtins except `and`/`or` + `alloc`/`dealloc` |
| `machine.and/or` rejected | write `&&` / `\|\|` |

## 4. Intentional rejections (parity with the reference pipeline)

| Message | Status |
|---|---|
| `{new\|import-java\|requires-java} is JVM-only` / `new is JVM-only (write Class(...) without new)` | Java interop stays JVM-only; `interop.cdc` must fail `llvmcheck` |
| `directives inside proc … must be top-level` | native directives are file-level |
| `[@…]` has no arm (falls to generic call/unbound) | attributes `@timeout/@retry/@tailcallopt/@typecheck` rejected by design |

## 5. Native interop (`collect_native` + lowering)

| Message | Fix |
|---|---|
| `unknown native type: T` | `int double bool string void ptr`, a registered class, or a registered (earlier-declared) struct |
| `unknown cpp-class 'C'` | declare `[cpp-class C …]` first |
| `duplicate native declaration 'A'` / `(C has no overloads)` | C aliases unique; C++ overloads differ by arity |
| `collides with builtin / cpp-class / native alias` | rename (one namespace) |
| `arity mismatch in call to F (expects A or B (got C))` | match a declared overload |
| `Class(args)` with no ctor arity | add `[cpp-ctor Class […]]` |
| `not a value … construct Class(...)` | bare `Class` is not a value |
| `unknown/ambiguous cpp member/method/static` | declare it; ambiguity → explicit `Class_method(recv, …)` |
| `cannot assign to constant 'X'` | `const` fields/statics/named consts read-only |
| `cpp-vector: bool … bit-packed; use int 0/1` / `void …` / `nested vectors` | change element type |
| `C++ class results must be pointers` | header-side factory returning a pointer |
| `expects K or more (got N)` | varargs call below fixed floor / above fixed+4 |
| `duplicate … (varargs covers all arities)` | one varargs decl per alias |
| `cpp-struct: expects Alias …` / `expects [field type] pairs` | fixed shape: alias, qualified, header, ≥1 `[name type]` pair |
| `duplicate field 'x'` / `void is not a field type` | unique non-void field types |
| `collides with cpp-struct` | rename (one namespace covers structs too) |
| `is a cpp-struct, not a value` | pass an array of its fields, not the bare alias |
| `cannot pass value/array as S (expects …)` | runtime: pass an array with exactly the field count |
| `cpp-vector: struct elements are not supported` | structs compose by nesting, not by vectorizing |
| `callback: expects Alias` / `void is not a parameter type` | fixed shape; signatures are scalar-only |
| `callback 'F': signatures support …` | no class/struct/func in callback sigs (follow-up) |
| `has no matching top-level procedure 'F'` / `must name a top-level procedure` | define a matching top-level `fn` (arity included) |
| `callback 'F' arity mismatch` | proc params must equal signature params |
| `collides with callback` | rename (one namespace covers callbacks too) |
