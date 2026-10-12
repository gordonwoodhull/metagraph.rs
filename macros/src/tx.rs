//! Transaction (`Tx`) codegen — M4's write side. One `Tx` per
//! `implement!` call: the transaction names the store type, entity ids,
//! and hooks, so only `implement!` codegen can emit it. This module owns
//! the *whole* emission — the change-set struct, the op queue, staging
//! methods, `flush`, the error/report surface, and the write-trait impls
//! — driven by a normalized [`TxModel`] that the flat and composed
//! codegens both build. Pattern-independent pieces (`Ref`, `TxToken`,
//! `HandleToRef`) live in the runtime crate's `tx` module; everything
//! here is per-store.
//!
//! The flush pipeline is the plan's fixed seven steps, emitted as one
//! generated method: (1) resolve removals and range-validate every
//! `Existing` ref, (2) dangler detection over kept records and staged
//! constructive ops in old-id space, honoring staged `set_` corrections,
//! (3) compaction with per-arena old→new id maps, (4) the kept skeleton
//! (records first, then each backing's kept-membership carry, intrusive
//! lists rebuilt by walking the old chains) followed by
//! submission-order replay through the same constructors and hooks
//! construction uses, (5) induction during replay (constructor logic),
//! (6) backing finalization (`freeze` on a phased store), (7)
//! `validate()`.

use proc_macro2::{Ident as Ident2, TokenStream as TokenStream2};
use quote::{format_ident, quote};

use crate::grammar::snake;

// ---------------------------------------------------------------------------
// The normalized model
// ---------------------------------------------------------------------------

/// One record-owning entity of the store, as the transaction sees it.
pub(crate) struct TxEntity {
    /// Entity name (`Node`); the flat handle type shares it.
    pub name: Ident2,
    /// Arena field on the store (`node_arena`).
    pub arena: Ident2,
    /// Constructor args in declaration order: (arg name, record field,
    /// target id type, ref alias of the target entity).
    pub ctor_args: Vec<TxCtorArg>,
    /// `on_add` hooks the constructor fires (store-level per-self growth,
    /// e.g. csr's bucket push) — the skeleton replays them manually.
    pub onadd_hooks: Vec<Ident2>,
}

pub(crate) struct TxCtorArg {
    pub arg: Ident2,
    pub target_ref_alias: Ident2,
    /// The target entity's conversion trait and method (`IntoNodeArg` /
    /// `into_node_ref`).
    pub target_into_trait: Ident2,
    pub target_into_fn: Ident2,
}

/// The whole store as the transaction sees it.
pub(crate) struct TxModel {
    pub store: Ident2,
    /// Runtime-crate absolute path (`::metagraph_exper`, rename-aware).
    pub rt: TokenStream2,
    /// Record-owning entities in schema order (root entity excluded — it
    /// owns no record, id, or removal op).
    pub entities: Vec<TxEntity>,
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

pub(crate) fn emit(m: &TxModel) -> TokenStream2 {
    let store = &m.store;
    let rt = &m.rt;

    // -- per-entity ref aliases + IntoArg traits -------------------------
    // Refs are aliases of the runtime `Ref<I>`; the conversion trait
    // accepts a ref (stamp-checked), a raw id (`Existing`), or an
    // old-store handle (store-identity asserted — a hard assert, not
    // debug-only: a foreign handle's id would silently resolve against
    // the wrong arena).
    let mut aliases = TokenStream2::new();
    let mut into_traits = TokenStream2::new();
    for e in &m.entities {
        let ref_alias = format_ident!("{}Ref", e.name);
        let into_trait = format_ident!("Into{}Arg", e.name);
        let into_fn = format_ident!("into_{}_ref", snake(&e.name.to_string()));
        let id_ty = format_ident!("{}Id", e.name);
        let handle_ty = &e.name;
        into_traits.extend(quote! {
            #[doc = concat!("Conversion into a `", stringify!(#ref_alias), " at a staging call: a ref (stamp-checked), a raw id (accepted as `Existing`), or an old-store handle (store identity asserted).")]
            pub trait #into_trait {
                fn #into_fn(self, tx: &Tx<'_>) -> #ref_alias;
            }
            impl #into_trait for #ref_alias {
                fn #into_fn(self, tx: &Tx<'_>) -> #ref_alias {
                    self.check_stamp(tx.token);
                    self
                }
            }
            impl #into_trait for #id_ty {
                fn #into_fn(self, _tx: &Tx<'_>) -> #ref_alias {
                    #ref_alias::existing(self)
                }
            }
            impl<'g> #into_trait for #handle_ty<'g> {
                fn #into_fn(self, tx: &Tx<'_>) -> #ref_alias {
                    assert!(
                        ::core::ptr::eq(self.store, tx.old),
                        "handle from a foreign store: a ref minted from it would silently resolve against the wrong arena"
                    );
                    #ref_alias::existing(self.id())
                }
            }
        });
        let doc = format!(
            "Provisional ref for `{}` records: `Existing` (old-store id) or `Pending` (change-set op index). Opaque — no read traits.",
            e.name
        );
        aliases.extend(quote! {
            #[doc = #doc]
            pub type #ref_alias = #rt::tx::Ref<#id_ty>;
        });
    }

    // -- the op queue ------------------------------------------------------
    // One variant per staging verb; op indices count every queued op,
    // root and nested alike, in submission order — the index is both the
    // `Pending` ref payload and the flush error attribution handle.
    let mut op_variants = TokenStream2::new();
    for e in &m.entities {
        let add = format_ident!("Add{}", e.name);
        let remove = format_ident!("Remove{}", e.name);
        let arg_fields = e.ctor_args.iter().map(|c| {
            let a = &c.arg;
            let r = &c.target_ref_alias;
            quote!(#a: #r,)
        });
        let payload = format_ident!("{}", snake(&e.name.to_string()));
        let ref_alias = format_ident!("{}Ref", e.name);
        op_variants.extend(quote! {
            #add { #(#arg_fields)* },
            #remove { #payload: #ref_alias },
        });
    }
    let ops_enum = quote! {
        #[doc(hidden)]
        #[derive(Debug)]
        pub enum Op {
            #op_variants
        }
    };

    // -- staging methods ----------------------------------------------------
    let mut staging = TokenStream2::new();
    for e in &m.entities {
        let snake_name = snake(&e.name.to_string());
        let add_op = format_ident!("add_{}", snake_name);
        let remove_op = format_ident!("remove_{}", snake_name);
        let ref_alias = format_ident!("{}Ref", e.name);
        let into_trait = format_ident!("Into{}Arg", e.name);
        let into_fn = format_ident!("into_{}_ref", snake_name);
        let add_variant = format_ident!("Add{}", e.name);
        let remove_variant = format_ident!("Remove{}", e.name);
        let payload = format_ident!("{}", snake_name);
        let arg_defs = e.ctor_args.iter().map(|c| {
            let a = &c.arg;
            let t = &c.target_into_trait;
            quote!(#a: impl #t,)
        });
        let arg_conv = e.ctor_args.iter().map(|c| {
            let a = &c.arg;
            let f = &c.target_into_fn;
            quote!(let #a = #a.#f(self);)
        });
        let arg_use = e.ctor_args.iter().map(|c| &c.arg);
        let doc_add = format!(
            "Stage a new `{}` record; returns its provisional ref — `Pending`, \
             naming this op's index. Staging never touches the store.",
            e.name
        );
        let doc_remove = format!(
            "Stage the removal of an old `{}` record. Removal is set-theoretic \
             (rebuild-without) and never replays; removal of a `Pending` ref is \
             a staging bug and panics here — staged entities have no old record \
             to remove.",
            e.name
        );
        staging.extend(quote! {
            #[doc = #doc_add]
            pub fn #add_op(&mut self, #(#arg_defs)*) -> #ref_alias {
                #(#arg_conv)*
                let op_index = self.ops.len() as u32;
                self.ops.push(Op::#add_variant { #(#arg_use),* });
                #rt::tx::Ref {
                    token: self.token,
                    kind: #rt::tx::RefKind::Pending(op_index),
                }
            }

            #[doc = #doc_remove]
            pub fn #remove_op(&mut self, #payload: impl #into_trait) {
                let #payload = #payload.#into_fn(self);
                match #payload.kind {
                    #rt::tx::RefKind::Existing(_) => {
                        self.ops.push(Op::#remove_variant { #payload });
                    }
                    #rt::tx::RefKind::Pending(_) => panic!(
                        "remove_{} of a Pending ref: staged entities have no old \
                         record to remove; removal is set-theoretic and never \
                         replays",
                        #snake_name
                    ),
                }
            }
        });
    }

    // -- the transaction ---------------------------------------------------
    quote! {
        #aliases
        #ops_enum
        #into_traits

        #[doc = concat!("Transaction over `", stringify!(#store), "`: stages a change set against the borrowed old store and flushes it as a full rebuild. Staging never touches the store — the change set is a plain queue, a first-class value.")]
        pub struct Tx<'a> {
            pub(crate) old: &'a #store,
            pub(crate) token: #rt::tx::TxToken,
            pub(crate) ops: ::std::vec::Vec<Op>,
        }

        impl #store {
            #[doc = "Borrow the store sharedly and start staging. Reads and old handles keep working while staging and after the flush; the old store is never mutated (the escape hatch — `into_edit` enforces the pass discipline at compile time)."]
            pub fn edit(&self) -> Tx<'_> {
                Tx {
                    old: self,
                    token: #rt::tx::mint_token(),
                    ops: ::std::vec::Vec::new(),
                }
            }
        }

        impl<'a> Tx<'a> {
            /// The old store this transaction borrows — the read side of
            /// the pass discipline (traverse with real handles while
            /// staging the replacement).
            pub fn old(&self) -> &'a #store {
                self.old
            }

            #staging
        }
    }
}
