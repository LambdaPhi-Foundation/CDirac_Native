//! `cdirac`: frontend (lexer/parser/AST) for the CDirac-native compiler.
//!
//! CDirac is the C-surface syntax for the Dirac language; SDirac
//! (`[...]`) is the bracket surface. Both describe the same AST plus
//! semantics — the language is the AST, surfaces are syntax. This crate
//! implements the CDirac surface fresh (span-carrying tokens and nodes,
//! migration errors, surface desugars) so the LLVM backend in this repo
//! builds on a clean foundation.
//!
//! Relation to `E:\bracket-jvm` (sibling checkout, separate workspace):
//! that repo owns the reference pipeline (checker, tree-walker, JVM
//! backend) and the normative specs (`CDirac.md`, `docs/builtins.md`,
//! `SYNTAX.md`). This crate tracks that surface definition; the shared
//! corpus under `../bracket-jvm/examples/cdc/` is the compatibility
//! gate (see `tests/corpus.rs`). If this crate disagrees with those
//! docs, the docs win.

pub mod ast;
pub mod closures;
pub mod driver;
pub mod lexer;
pub mod llvmgen;
pub mod native;
pub mod parser;
