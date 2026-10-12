# HANDOFF — M4 implementation (write-side builders)

Branch `worktree-m4-implement`, worktree `.claude/worktrees/m4-implement`,
cut from main 4904680. Plan: plans/milestone-4-builders.md (the contract).
This file: running notes, decisions, step log. Retires to .handoffs/ with
the worktree.

## Session log

- 2026-10-12: survey complete (repo codegen, all five probes + spike 7/8).
  Baseline suite green. Starting step 1.
- Step 1 DONE (commit follows): runtime `src/tx.rs` (TxToken/Ref/
  check_stamp/HandleToRef), `macros/src/tx.rs` emitter (TxModel →
  Tx/Op/IntoArg/staging/edit), flat codegen wired with the stage-1
  shape gate (flat ∧ non-phased ∧ no keyed → Tx; else no Tx yet).
  tests/tx_stage1.rs (3) + compilefail/read_on_ref.rs. Suites green.

## Architecture decisions (settled up front, revisit only if forced)

1. **Generic tx pieces live in the runtime crate** (`src/tx.rs`): `TxToken`
   (atomic mint), `RefKind<I>` / `Ref<I>` (stamp + kind, both fields
   readable — generated staging code lives in user modules), a
   `check_stamp` helper, and the marker-generic `HandleToRef<M>` trait
   (step 6's D7 pin). Everything pattern-specific — `Tx<'a>`, `Op`,
   staging methods, `flush`, error/report types, write-trait impls — is
   emitted per `implement!` by a new `macros/src/tx.rs` emitter driven by
   a normalized **TxModel** built by both flat and composed codegen. This
   keeps the flush emission in exactly one place; the model grows per
   step (set ops → step 7, paths → step 9, keyed carry → step 11, phased
   → step 12).
2. **Shape gating during stages 1–3**: `tx::emit` currently supports
   flat, non-phased stores with no keyed and no csr bindings; other
   shapes emit no `Tx` (no `edit()`) until their step lands. Main's tests
   never call `edit()` on those shapes mid-milestone, so the suite stays
   green. Final state: all shapes.
3. **Flush skeleton copy bypasses constructors** (probes' shape): kept
   records are pushed as raw record literals — id fields remapped,
   `Option` links `None`, containers `Default` — then `on_add` hooks fire
   manually, then per-backing carry (walk-rebuild for intrusive lists,
   map carry for keyed, bucket-feed for csr), then staged ops replay
   *through the real constructors / grown hooks* in submission order.
4. **Induction (step 10) via a generated escape hatch**: undeclared
   constructor logic runs at replay through a doc-hidden `Op::Custom(Box<
   dyn FnOnce(&mut Store, &Resolve)>)` variant plus a doc-hidden
   `__stage_custom` staging method on `Tx` (user code writes `impl Tx` in
   the same crate — the store type is generated into the user's module,
   so inherent-impl extension works, exactly like today's hand-written
   `impl EdgeInducedStore { add_subedge }`). The live-key staging check
   gains a generated `ReservedKey` enum (one variant per keyed arrow,
   carrying the (host-,)key refs) fed by a `__reserve_keys` helper, so a
   standalone keyed add after a staged inducing op still panics at
   staging (the induct-flush probe's reading). Cost: Custom ops are not
   inspectable; noted as the price of undeclared semantics.
5. **Replay-miss coverage (step 9 decision obligation)**: decided —
   same-crate unit test (runtime crate `src/stdlib.rs` tests or
   `src/tx.rs` tests driving an in-crate `implement!`), because the
   forged-stamped-ref trick needs module-private access the generated
   code does not grant downstream. Recorded here per the plan.
6. **csr × set_ interaction** (by-derived csr membership vs a `set_` on
   the determinant): plan says "a phased backing needs nothing". Emission
   follows that (no unlink, no relink for phased derivatives); the
   residual stale-bucket question is noted in step-7 notes, and if a test
   binds that combination we revisit.

## Step notes

### Step 1 — change-set data model — DONE

Acceptance: refs expose no read methods (compile-fail fixture
`read_on_ref.rs`, E0599 pinned); the change set is a plain struct (Tx
holds `Vec<Op>`; op indices count every op — asserted in
`pending_refs_count_every_queued_op`). Notes: refs' `token`/`kind`
fields are public (generated staging code in user modules must read
them; forging cross-tx Pending is the stamp-panic we want).
`Tx::old()` exposed for pass-discipline reads.
