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
    /// The record's contributed fields in declaration order, with their
    /// skeleton init — the skeleton's raw record literal.
    pub fields: Vec<TxField>,
}

pub(crate) struct TxCtorArg {
    pub arg: Ident2,
    /// The arg's target entity — whose arena, compaction map, and pending
    /// table the ref resolves through at flush.
    pub target_entity: Ident2,
    pub target_ref_alias: Ident2,
    /// The target entity's conversion trait and method (`IntoNodeArg` /
    /// `into_node_ref`).
    pub target_into_trait: Ident2,
    pub target_into_fn: Ident2,
}

/// How one record/store field initializes in the flush skeleton (step 4a).
/// The uniform init rule the constructors use, minus the one class the
/// skeleton refines: ctor-arg id fields copy the OLD record's field
/// remapped through the target's compaction map.
pub(crate) enum TxFieldInit {
    CtorArg { target: Ident2 },
    /// `Option` link/head: born `None`; the intrusive carry re-threads.
    None_,
    /// `Vec`/`HashMap` container: born empty (its carry class arrives with
    /// the keyed/phased steps).
    New,
}

pub(crate) struct TxField {
    pub emitted: Ident2,
    pub init: TxFieldInit,
}

/// One intrusive-list binding the flush carries by rebuild-by-walking the
/// old chains (kept hosts only, removed members skipped, ids remapped) —
/// never per-field remap, which truncates at a removed mid-chain member.
pub(crate) struct TxIntrusive {
    /// The head's owner: `Some(entity)` whose record holds it, or `None`
    /// when the self is the unfused root (its `on A` contribution folds to
    /// a store field — one chain for the whole store).
    pub host: Option<Ident2>,
    pub head_field: Ident2,
    /// The member entity (B) and the link field on its record.
    pub member: Ident2,
    pub member_link: Ident2,
}

/// The whole store as the transaction sees it.
pub(crate) struct TxModel {
    pub store: Ident2,
    /// Runtime-crate absolute path (`::metagraph_exper`, rename-aware).
    pub rt: TokenStream2,
    /// Record-owning entities in schema order (root entity excluded — it
    /// owns no record, id, or removal op).
    pub entities: Vec<TxEntity>,
    /// Store-level contributed fields (a root-sourced arrow's head, keyed
    /// maps) with their skeleton init, for the new store's literal.
    pub store_fields: Vec<TxField>,
    /// Intrusive-carry bindings in schema arrow order.
    pub intrusive: Vec<TxIntrusive>,
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

    // -- flush: per-entity derived names -----------------------------------
    struct Ent {
        name: Ident2,
        snake: Ident2,
        arena: Ident2,
        data: Ident2,
        id_ty: Ident2,
        arena_str: String,
        count: Ident2,   // old_<snake>_count
        map: Ident2,     // <snake>_map
        removed: Ident2, // removed_<snake>
        removed_by: Ident2,
        pending: Ident2, // pending_<snake>
    }
    let ents: Vec<Ent> = m
        .entities
        .iter()
        .map(|e| {
            let snake_name = format_ident!("{}", snake(&e.name.to_string()));
            Ent {
                name: e.name.clone(),
                arena: e.arena.clone(),
                data: format_ident!("{}Data", e.name),
                id_ty: format_ident!("{}Id", e.name),
                arena_str: snake(&e.name.to_string()),
                count: format_ident!("old_{}_count", snake(&e.name.to_string())),
                map: format_ident!("{}_map", snake(&e.name.to_string())),
                removed: format_ident!("removed_{}", snake(&e.name.to_string())),
                removed_by: format_ident!("removed_by_{}", snake(&e.name.to_string())),
                pending: format_ident!("pending_{}", snake(&e.name.to_string())),
                snake: snake_name,
            }
        })
        .collect();
    let ent_of = |name: &Ident2| ents.iter().find(|x| &x.name == name).expect("entity");

    // -- flush: the error/report surface ------------------------------------
    let report_fields = ents.iter().map(|x| {
        let f = &x.map;
        quote!(#[doc = concat!("Old->new id map of `", stringify!(#f), "'s arena: old-id-indexed, `None` where removed.")] pub #f: ::std::vec::Vec<::core::option::Option<u32>>,)
    });
    let report_init = ents.iter().map(|x| {
        let f = &x.map;
        quote!(#f: #f,)
    });
    let flush_types = quote! {
        #[doc = "How a record was named in a [`Violation::DanglingEndpoint`]: an old-store record kept from the previous store (its arena name and old id), or a record staged in this change set (its `add` op's index)."]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum RecordDesc {
            Kept { arena: &'static str, id: u32 },
            Staged(u32),
        }

        #[doc = "One flush violation, reported in OLD ids by change-set op index. The listed order is discovery order: step 1 (removals, range validation) then step 2 (dangler detection)."]
        #[derive(Clone, PartialEq, Eq, Debug)]
        pub enum Violation {
            #[doc = "An `Existing` ref named an id outside the old arena."]
            UnknownOldId { op: usize, arena: &'static str, id: u32 },
            #[doc = "The same old id was removed twice."]
            DoubleRemoval { op: usize, arena: &'static str, id: u32 },
            #[doc = "A record (kept or staged) whose field target was removed: a kept record's violation points at the removal op that caused it, a staged add's at the op that named the removed endpoint."]
            DanglingEndpoint { op: usize, record: RecordDesc, endpoint_arena: &'static str, endpoint: u32 },
            #[doc = "Final `validate()` failed — defensive arm; the pipeline should never produce damage."]
            ValidateFailed { detail: ::std::string::String },
        }

        #[doc = "The flush's rejection: every violation found, in discovery order."]
        #[derive(Clone, PartialEq, Eq, Debug)]
        pub struct FlushError {
            pub violations: ::std::vec::Vec<Violation>,
        }

        impl ::core::fmt::Display for FlushError {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                for (i, v) in self.violations.iter().enumerate() {
                    if i > 0 {
                        ::core::writeln!(f)?;
                    }
                    ::core::write!(f, "{v:?}")?;
                }
                ::core::result::Result::Ok(())
            }
        }
        impl ::std::error::Error for FlushError {}

        #[doc = "The flush report: per-arena old->new id maps. Kept records compact first (old arena order), staged adds append in replay order. For raw-id holders — handles never cross stores."]
        #[derive(Clone, Debug)]
        pub struct FlushReport {
            #(#report_fields)*
        }

        #[doc = "A successful flush: the new store and its report."]
        #[derive(Debug)]
        pub struct FlushOutcome {
            pub store: #store,
            pub report: FlushReport,
        }
    };

    // -- flush: the pipeline ------------------------------------------------
    // Step 1 — counts, kept sets, and the op scan (removals resolve;
    // every `Existing` ref range-validates; anything read by a later step
    // is range-guarded — a flagged id must never be indexed).
    let s1_counts = ents.iter().map(|x| {
        let c = &x.count;
        let a = &x.arena;
        quote!(let #c = old.#a.len();)
    });
    let s1_state = ents.iter().map(|x| {
        let (removed, removed_by, c) = (&x.removed, &x.removed_by, &x.count);
        quote!(
            let mut #removed = ::std::vec![false; #c];
            let mut #removed_by: ::std::vec::Vec<::core::option::Option<usize>> =
                ::std::vec![::core::option::Option::None; #c];
        )
    });
    let s1_add_arms = m.entities.iter().map(|e| {
        let variant = format_ident!("Add{}", e.name);
        if e.ctor_args.is_empty() {
            return quote!(Op::#variant {} => {});
        }
        let checks = e.ctor_args.iter().map(|c| {
            let t = ent_of(&c.target_entity);
            let (t_count, t_str) = (&t.count, &t.arena_str);
            let a = &c.arg;
            quote! {
                if let #rt::tx::RefKind::Existing(id) = #a.kind {
                    if (id).0 as usize >= #t_count {
                        violations.push(Violation::UnknownOldId {
                            op: op_index,
                            arena: #t_str,
                            id: (id).0,
                        });
                    }
                }
            }
        });
        let arg_names = e.ctor_args.iter().map(|c| &c.arg);
        quote! {
            Op::#variant { #(#arg_names),* } => {
                #(#checks)*
            }
        }
    });
    let s1_remove_arms = m.entities.iter().map(|e| {
        let variant = format_ident!("Remove{}", e.name);
        let x = ent_of(&e.name);
        let payload = &x.snake;
        let (removed, removed_by, arena_str) = (&x.removed, &x.removed_by, &x.arena_str);
        quote! {
            Op::#variant { #payload } => {
                if let #rt::tx::RefKind::Existing(id) = #payload.kind {
                    match #removed_by.get((id).0 as usize) {
                        ::core::option::Option::None => {
                            violations.push(Violation::UnknownOldId {
                                op: op_index,
                                arena: #arena_str,
                                id: (id).0,
                            });
                        }
                        ::core::option::Option::Some(::core::option::Option::Some(_)) => {
                            violations.push(Violation::DoubleRemoval {
                                op: op_index,
                                arena: #arena_str,
                                id: (id).0,
                            });
                        }
                        ::core::option::Option::Some(::core::option::Option::None) => {
                            #removed[(id).0 as usize] = true;
                            #removed_by[(id).0 as usize] = ::core::option::Option::Some(op_index);
                        }
                    }
                }
            }
        }
    });

    // Step 2, pass A — kept-record danglers over every ctor-arg field (a
    // field-backed `one` arrow): a kept record whose field target was
    // removed dangles, attributed to the removal op.
    let s2_kept = m.entities.iter().filter(|e| !e.ctor_args.is_empty()).map(|e| {
        let x = ent_of(&e.name);
        let (c, removed, arena, arena_str) = (&x.count, &x.removed, &x.arena, &x.arena_str);
        let checks = e.ctor_args.iter().map(|ca| {
            let t = ent_of(&ca.target_entity);
            let (t_removed, t_removed_by, t_str) = (&t.removed, &t.removed_by, &t.arena_str);
            let f = &ca.arg; // the arg name is the arrow name is the field name
            quote! {
                {
                    let v = rec.#f;
                    if #t_removed[(v).0 as usize] {
                        violations.push(Violation::DanglingEndpoint {
                            op: #t_removed_by[(v).0 as usize].unwrap(),
                            record: RecordDesc::Kept { arena: #arena_str, id: old_id as u32 },
                            endpoint_arena: #t_str,
                            endpoint: (v).0,
                        });
                    }
                }
            }
        });
        quote! {
            for old_id in 0..#c {
                if #removed[old_id] {
                    continue;
                }
                let rec = &old.#arena[old_id];
                #(#checks)*
            }
        }
    });

    // Step 2, pass B — staged adds naming removed endpoints, attributed to
    // the naming op. Range-guarded: a flagged id is never indexed.
    let s2_staged_arms = m.entities.iter().filter(|e| !e.ctor_args.is_empty()).map(|e| {
        let variant = format_ident!("Add{}", e.name);
        let arg_names = e.ctor_args.iter().map(|c| &c.arg);
        let checks = e.ctor_args.iter().map(|c| {
            let t = ent_of(&c.target_entity);
            let (t_count, t_removed, t_str) = (&t.count, &t.removed, &t.arena_str);
            let a = &c.arg;
            quote! {
                if let #rt::tx::RefKind::Existing(id) = #a.kind {
                    if ((id).0 as usize) < #t_count && #t_removed[(id).0 as usize] {
                        violations.push(Violation::DanglingEndpoint {
                            op: op_index,
                            record: RecordDesc::Staged(op_index as u32),
                            endpoint_arena: #t_str,
                            endpoint: (id).0,
                        });
                    }
                }
            }
        });
        quote! {
            if let Op::#variant { #(#arg_names),* } = op {
                #(#checks)*
            }
        }
    });

    // Step 3 — compaction maps.
    let s3_maps = ents.iter().map(|x| {
        let (map, removed, c) = (&x.map, &x.removed, &x.count);
        quote! {
            let mut #map: ::std::vec::Vec<::core::option::Option<u32>> =
                ::std::vec![::core::option::Option::None; #c];
            {
                let mut next = 0u32;
                for (old_id, slot) in #map.iter_mut().enumerate() {
                    if !#removed[old_id] {
                        *slot = ::core::option::Option::Some(next);
                        next += 1;
                    }
                }
            }
        }
    });

    // Step 4a — the skeleton: store literal (capacities borrowed from the
    // old sizes; store-level links born empty), then kept records in old
    // arena order as raw literals — ctor-arg id fields remapped through
    // the target's map, everything else per the uniform init rule — with
    // on_add hooks fired manually per kept record.
    let s4_arena_inits = m.entities.iter().map(|e| {
        let x = ent_of(&e.name);
        let (arena, c) = (&x.arena, &x.count);
        quote!(#arena: ::std::vec::Vec::with_capacity(#c),)
    });
    let s4_store_inits = m.store_fields.iter().map(|f| {
        let n = &f.emitted;
        match f.init {
            TxFieldInit::None_ => quote!(#n: ::core::option::Option::None,),
            TxFieldInit::New => quote!(#n: ::std::default::Default::default(),),
            TxFieldInit::CtorArg { .. } => quote!(#n: ::std::default::Default::default(),),
        }
    });
    let s4_record_loops = m.entities.iter().map(|e| {
        let x = ent_of(&e.name);
        let (c, map, arena, data, id_ty) =
            (&x.count, &x.map, &x.arena, &x.data, &x.id_ty);
        let field_inits = e.fields.iter().map(|f| {
            let n = &f.emitted;
            match &f.init {
                TxFieldInit::CtorArg { target } => {
                    let t = ent_of(&target);
                    let (t_map, t_id) = (&t.map, &t.id_ty);
                    quote!(#n: #t_id(#t_map[(rec.#n).0 as usize].expect("field target kept: step 2 proved it")),)
                }
                TxFieldInit::None_ => quote!(#n: ::core::option::Option::None,),
                TxFieldInit::New => quote!(#n: ::std::default::Default::default(),),
            }
        });
        let onadd_fires = e.onadd_hooks.iter().map(|h| {
            quote!(new_store.#h(#id_ty(new_id));)
        });
        quote! {
            for old_id in 0..#c {
                if let ::core::option::Option::Some(new_id) = #map[old_id] {
                    let rec = &old.#arena[old_id];
                    new_store.#arena.push(#data { #(#field_inits)* });
                    #(#onadd_fires)*
                    ::core::debug_assert_eq!(
                        new_id as usize,
                        new_store.#arena.len() - 1,
                        "kept records compact in old arena order"
                    );
                }
            }
        }
    });

    // Step 4b — the intrusive carries: rebuild by walking the OLD chains
    // (kept hosts only, removed members skipped, ids remapped), threading
    // the new chains. Head at a record (per-host loop) or at the store
    // (one chain).
    let s4_walks = m.intrusive.iter().map(|i| {
        let member = ent_of(&i.member);
        let (m_arena, m_map, m_id, m_link) =
            (&member.arena, &member.map, &member.id_ty, &i.member_link);
        let walk_body = |head_write: TokenStream2, first: TokenStream2| {
            quote! {
                let mut prev: ::core::option::Option<u32> = ::core::option::Option::None;
                let mut cur = #first;
                while let ::core::option::Option::Some(old_member) = cur {
                    cur = old.#m_arena[old_member.0 as usize].#m_link;
                    if let ::core::option::Option::Some(new_member) = #m_map[old_member.0 as usize] {
                        match prev {
                            ::core::option::Option::None => #head_write,
                            ::core::option::Option::Some(p) =>
                                new_store.#m_arena[p as usize].#m_link =
                                    ::core::option::Option::Some(#m_id(new_member)),
                        }
                        prev = ::core::option::Option::Some(new_member);
                    }
                }
            }
        };
        match &i.host {
            Some(host) => {
                let h = ent_of(host);
                let (h_count, h_map, h_arena) = (&h.count, &h.map, &h.arena);
                let head = &i.head_field;
                let body = walk_body(
                    quote!(new_store.#h_arena[new_host as usize].#head =
                        ::core::option::Option::Some(#m_id(new_member))),
                    quote!(old.#h_arena[old_host].#head),
                );
                quote! {
                    for old_host in 0..#h_count {
                        let ::core::option::Option::Some(new_host) = #h_map[old_host] else {
                            continue;
                        };
                        #body
                    }
                }
            }
            None => {
                let head = &i.head_field;
                let body = walk_body(
                    quote!(new_store.#head = ::core::option::Option::Some(#m_id(new_member))),
                    quote!(old.#head),
                );
                quote! {
                    #body
                }
            }
        }
    });

    // Step 4c — replay constructive ops in submission order through the
    // REAL constructors (on_add growth + insert hooks fire as construction
    // always fires them). Removals never replay. A `Pending` ref resolves
    // through its entity's replay table; a miss is an internal bug.
    let s4c_pending = ents.iter().map(|x| {
        let (p, id_ty) = (&x.pending, &x.id_ty);
        quote!(let mut #p: ::std::vec::Vec<::core::option::Option<#id_ty>> =
            ::std::vec![::core::option::Option::None; ops.len()];)
    });
    let s4c_add_arms = m.entities.iter().map(|e| {
        let x = ent_of(&e.name);
        let variant = format_ident!("Add{}", e.name);
        let (add_fn, p) = (format_ident!("add_{}", &x.snake), &x.pending);
        if e.ctor_args.is_empty() {
            return quote! {
                Op::#variant {} => {
                    #p[op_index] = ::core::option::Option::Some(new_store.#add_fn());
                }
            };
        }
        let resolves = e.ctor_args.iter().map(|c| {
            let t = ent_of(&c.target_entity);
            let (t_map, t_p, t_id) = (&t.map, &t.pending, &t.id_ty);
            let a = &c.arg;
            quote! {
                let #a = match #a.kind {
                    #rt::tx::RefKind::Existing(id) =>
                        #t_id(#t_map[(id).0 as usize].expect("Existing id validated in steps 1-2")),
                    #rt::tx::RefKind::Pending(i) => #t_p[i as usize]
                        .expect("an add precedes its use: submission order"),
                };
            }
        });
        let arg_use: Vec<&Ident2> = e.ctor_args.iter().map(|c| &c.arg).collect();
        quote! {
            Op::#variant { #(#arg_use),* } => {
                #(#resolves)*
                #p[op_index] = ::core::option::Option::Some(new_store.#add_fn(#(#arg_use),*));
            }
        }
    });
    let remove_patterns = m.entities.iter().map(|e| {
        let variant = format_ident!("Remove{}", e.name);
        quote!(Op::#variant { .. })
    });

    let flush_method = quote! {
        #[doc = "Run the flush pipeline and consume the transaction: (1) resolve removals and range-validate every `Existing` ref, (2) dangler detection over kept records and staged adds in old-id space, (3) compaction with per-arena old->new id maps, (4) the kept skeleton (records with remapped id fields, intrusive lists rebuilt by walking the old chains) then submission-order replay through the same constructors construction uses, (5) induction (constructor logic — none staged by the built-ins), (6) backing finalization (a phased store's freeze — flat stores have none), (7) `validate()`. The old store is untouched; a rejection leaves nothing half-applied."]
        pub fn flush(self) -> ::core::result::Result<FlushOutcome, FlushError> {
            let Tx { old, token: _, ops } = self;

            // -- step 1: resolve removals; validate every Existing ref -----
            #(#s1_counts)*
            #(#s1_state)*
            let mut violations: ::std::vec::Vec<Violation> = ::std::vec::Vec::new();
            for (op_index, op) in ops.iter().enumerate() {
                match op {
                    #(#s1_add_arms)*
                    #(#s1_remove_arms)*
                }
            }

            // -- step 2: dangler detection, in old-id space ----------------
            // Kept records first (attributed to the removal op), then
            // staged adds naming removed endpoints (attributed to the
            // naming op). Runs before compaction so the report names the
            // ids the user actually staged with.
            #(#s2_kept)*
            for (op_index, op) in ops.iter().enumerate() {
                #(#s2_staged_arms)*
            }
            if !violations.is_empty() {
                return ::core::result::Result::Err(FlushError { violations });
            }

            // -- step 3: compaction; per-arena old->new id maps ------------
            #(#s3_maps)*

            // -- step 4a: the kept skeleton --------------------------------
            let mut new_store = #store {
                #(#s4_arena_inits)*
                #(#s4_store_inits)*
            };
            #(#s4_record_loops)*

            // -- step 4b: intrusive carries, rebuilt by walking ------------
            #(#s4_walks)*

            // -- step 4c: replay constructive ops in submission order ------
            #(#s4c_pending)*
            for (op_index, op) in ops.iter().enumerate() {
                match op {
                    #(#s4c_add_arms)*
                    #(#remove_patterns)|* => {}
                }
            }

            // -- steps 5-6: induction and backing finalization -------------
            // Induction is constructor logic and replays through the same
            // constructors above; the flat non-phased store has no
            // finalization step.

            // -- step 7: validate ------------------------------------------
            new_store.validate().map_err(|detail| FlushError {
                violations: ::std::vec![Violation::ValidateFailed { detail }],
            })?;

            ::core::result::Result::Ok(FlushOutcome {
                store: new_store,
                report: FlushReport { #(#report_init)* },
            })
        }
    };

    // -- the transaction ---------------------------------------------------
    quote! {
        #aliases
        #ops_enum
        #into_traits
        #flush_types

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

            #flush_method
        }
    }
}
