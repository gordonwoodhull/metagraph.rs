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

use crate::grammar::{camel, snake};

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
    /// Per-entity marker path (absolute) — the HandleToRef/Add/Remove
    /// impls ride under it.
    pub marker_path: TokenStream2,
    /// `Add<E>` trait path (absolute), `None` when the declaring block
    /// skipped the trait (an opaque ctor-arg arrow vetoes it — the verb
    /// stays Tx-inherent).
    pub add_trait: Option<TokenStream2>,
    /// `Remove<E>` trait path (absolute) — the declaring block always
    /// emits it for a closed non-root entity.
    pub remove_trait: TokenStream2,
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
    /// A ctor-arg id field: copies the OLD record's field remapped through
    /// the target's compaction map — or, when a staged `set_` re-pointed it
    /// and the old endpoint was removed (the honor case), the EFFECTIVE
    /// endpoint (predicted when Pending, asserted at replay).
    CtorArg { target: Ident2, arrow: Ident2 },
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
    /// The binding's arrow name — the derived walk arrays are named after
    /// it (`__<arrow>_heads` / `__<arrow>_links`).
    pub arrow: Ident2,
    /// The head's owner: `Some(entity)` whose record holds it, or `None`
    /// when the self is the unfused root (its `on A` contribution folds to
    /// a store field — one chain for the whole store).
    pub host: Option<Ident2>,
    pub head_field: Ident2,
    /// The member entity (B) and the link field on its record.
    pub member: Ident2,
    pub member_link: Ident2,
}

/// A marker-generic write-trait impl on `Tx` (the arrow-named `Insert`/
/// `Set` verbs — the names are the root's fixed contract, published via
/// the courier's write slot). `assocs` are the trait's assoc bindings in
/// declaration order, values pre-resolved to this store's ref aliases.
pub(crate) struct TxVerb {
    pub trait_path: TokenStream2,
    pub marker_path: TokenStream2,
    pub method: Ident2,
    pub assocs: Vec<(Ident2, TokenStream2)>,
}

/// The `insert_<arrow>` staging verb of one primitive (non-`by`) bound
/// `many` arrow — membership asserted directly, replayed through the
/// backing's `__insert_<arrow>` hook at flush.
pub(crate) struct TxInsert {
    /// The local arrow name: the op variant, the inherent staging verb,
    /// and the store hook are all named after it.
    pub arrow: Ident2,
    /// The self (host) entity; `None` at an unfused-root self — the verb
    /// is inherent-only in its self-less form (canonical arity takes the
    /// self ref, which does not exist there).
    pub host: Option<Ident2>,
    /// The member (target) entity.
    pub member: Ident2,
    /// The `Insert<Arrow>` trait impl, `None` on the self-less form.
    pub verb: Option<TxVerb>,
}

/// The `set_<arrow>` staging verb of one field-backed `one` arrow (a
/// `ctor_arg` backing — the builder's `__set_<arrow>` hook is the re-point
/// orchestration: unlink the by-derived derivatives, write, re-link). The
/// flush honors staged corrections in dangler detection (the probe's honor
/// reading) and replays through the same hook against the rebuilt skeleton.
pub(crate) struct TxSet {
    /// The local arrow name: the op variant `Set<camel>`, the inherent
    /// staging verb, and the store's `__set_<arrow>` hook are all named
    /// after it. Also the record field's name (the ctor-arg field is
    /// arrow-named).
    pub arrow: Ident2,
    /// The self entity (whose record's field is re-pointed).
    pub self_entity: Ident2,
    /// The field's target entity (the new endpoint's resolution arena).
    pub target_entity: Ident2,
    /// The `Set<Arrow>` trait impl (the courier's write slot).
    pub verb: TxVerb,
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
    /// Primitive-insert verbs in schema arrow order (non-`by` bound many
    /// arrows with an insert hook — by-derived and existence arrows stage
    /// no insert).
    pub inserts: Vec<TxInsert>,
    /// `set_<arrow>` verbs in schema arrow order (field-backed `one`
    /// arrows — every ctor-arg arrow grows the re-point hook).
    pub sets: Vec<TxSet>,
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

pub(crate) fn emit(m: &TxModel) -> TokenStream2 {
    let store = &m.store;
    let rt = &m.rt;

    // -- per-entity ref aliases + IntoArg traits + HandleToRef impls ------
    // Refs are aliases of the runtime `Ref<I>`; the conversion trait
    // accepts a ref (stamp-checked), a raw id (`Existing`), or an
    // old-store handle (store-identity asserted — a hard assert, not
    // debug-only: a foreign handle's id would silently resolve against
    // the wrong arena). The `HandleToRef` impl is the mapping trait the
    // generic transforms bound on: one impl per entity, marked by the
    // per-entity marker.
    let mut aliases = TokenStream2::new();
    let mut into_traits = TokenStream2::new();
    let mut handle_impls = TokenStream2::new();
    for e in &m.entities {
        let ref_alias = format_ident!("{}Ref", e.name);
        let into_trait = format_ident!("Into{}Arg", e.name);
        let into_fn = format_ident!("into_{}_ref", snake(&e.name.to_string()));
        let id_ty = format_ident!("{}Id", e.name);
        let handle_ty = &e.name;
        let marker_path = &e.marker_path;
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
                        ::core::ptr::eq(self.store, tx.old.__store()),
                        "handle from a foreign store: a ref minted from it would silently resolve against the wrong arena"
                    );
                    #ref_alias::existing(self.id())
                }
            }
        });
        handle_impls.extend(quote! {
            impl<'g> #rt::tx::HandleToRef<#marker_path> for Tx<'g> {
                type Handle = #handle_ty<'g>;
                type Ref = #ref_alias;
                fn to_ref(&self, handle: #handle_ty<'g>) -> #ref_alias {
                    handle.#into_fn(self)
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
    for i in &m.inserts {
        let variant = format_ident!("Insert{}", camel(&i.arrow.to_string()));
        let member_ref = format_ident!("{}Ref", i.member);
        match &i.host {
            Some(host) => {
                let host_ref = format_ident!("{}Ref", host);
                op_variants.extend(quote! {
                    #variant { a: #host_ref, b: #member_ref },
                });
            }
            None => {
                op_variants.extend(quote! {
                    #variant { b: #member_ref },
                });
            }
        }
    }
    for s in &m.sets {
        let variant = format_ident!("Set{}", camel(&s.arrow.to_string()));
        let a_ref = format_ident!("{}Ref", s.self_entity);
        let b_ref = format_ident!("{}Ref", s.target_entity);
        op_variants.extend(quote! {
            #variant { a: #a_ref, b: #b_ref },
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

    // -- write-trait impls on Tx --------------------------------------------
    // The declaring-block-owned existence traits: `Add<E>` (only where the
    // declaring block proved the ctor-arg arity — skipped = opaque) and
    // `Remove<E>`, both under the per-entity marker, delegating to the
    // inherent staging verbs. Then the arrow-named `Insert<Arrow>` impls
    // (root-fixed names from the courier's write slot) for the primitive
    // many arrows with a self; the self-less forms are inherent-only.
    let mut write_impls = TokenStream2::new();
    for e in &m.entities {
        let snake_name = snake(&e.name.to_string());
        let ref_alias = format_ident!("{}Ref", e.name);
        let ret_assoc = format_ident!("{}Ref", e.name);
        let marker_path = &e.marker_path;
        let add_method = format_ident!("add_{}", snake_name);
        let remove_method = format_ident!("remove_{}", snake_name);
        if let Some(trait_path) = &e.add_trait {
            let arg_assocs = e
                .ctor_args
                .iter()
                .map(|c| format_ident!("{}", camel(&c.arg.to_string())))
                .collect::<Vec<_>>();
            let arg_names = e.ctor_args.iter().map(|c| &c.arg).collect::<Vec<_>>();
            let arg_values = e.ctor_args.iter().map(|c| &c.target_ref_alias);
            let assoc_decls = arg_assocs.iter().zip(arg_values).map(|(an, av)| quote!(type #an = #av;));
            write_impls.extend(quote! {
                impl<'g> #trait_path<#marker_path> for Tx<'g> {
                    type #ret_assoc = #ref_alias;
                    #(#assoc_decls)*
                    fn #add_method(&mut self, #(#arg_names: Self::#arg_assocs),*) -> Self::#ret_assoc {
                        Tx::#add_method(self, #(#arg_names),*)
                    }
                }
            });
        }
        let remove_trait = &e.remove_trait;
        write_impls.extend(quote! {
            impl<'g> #remove_trait<#marker_path> for Tx<'g> {
                type #ret_assoc = #ref_alias;
                fn #remove_method(&mut self, rec: Self::#ret_assoc) {
                    Tx::#remove_method(self, rec)
                }
            }
        });
    }
    for i in &m.inserts {
        if let Some(verb) = &i.verb {
            let trait_path = &verb.trait_path;
            let marker_path = &verb.marker_path;
            let method = &verb.method;
            let inherent = format_ident!("insert_{}", i.arrow);
            let assoc_decls = verb.assocs.iter().map(|(an, av)| quote!(type #an = #av;));
            let (params, args): (Vec<Ident2>, Vec<Ident2>) = match &i.host {
                Some(_) => (vec![Ident2::new("a", proc_macro2::Span::call_site()), Ident2::new("b", proc_macro2::Span::call_site())], vec![Ident2::new("a", proc_macro2::Span::call_site()), Ident2::new("b", proc_macro2::Span::call_site())]),
                None => {
                    let b = Ident2::new("b", proc_macro2::Span::call_site());
                    (vec![b.clone()], vec![b])
                }
            };
            let param_tys = verb.assocs.iter().map(|(an, _)| an);
            write_impls.extend(quote! {
                impl<'g> #trait_path<#marker_path> for Tx<'g> {
                    #(#assoc_decls)*
                    fn #method(&mut self, #(#params: Self::#param_tys),*) {
                        Tx::#inherent(self, #(#args),*)
                    }
                }
            });
        }
    }
    for s in &m.sets {
        let verb = &s.verb;
        let trait_path = &verb.trait_path;
        let marker_path = &verb.marker_path;
        let method = &verb.method;
        let inherent = format_ident!("set_{}", s.arrow);
        let assoc_decls = verb.assocs.iter().map(|(an, av)| quote!(type #an = #av;));
        let a = Ident2::new("a", proc_macro2::Span::call_site());
        let b = Ident2::new("b", proc_macro2::Span::call_site());
        let a_ty = &verb.assocs[0].0;
        let b_ty = &verb.assocs[1].0;
        write_impls.extend(quote! {
            impl<'g> #trait_path<#marker_path> for Tx<'g> {
                #(#assoc_decls)*
                fn #method(&mut self, #a: Self::#a_ty, #b: Self::#b_ty) {
                    Tx::#inherent(self, #a, #b)
                }
            }
        });
    }

    // -- insert_<arrow> staging (primitive many arrows) ---------------------
    // Membership asserted directly on an existing record; the flush
    // replays the op through the backing's `__insert_<arrow>` hook after
    // the carried skeleton (the insert lands on top of carried
    // membership, never rebuilt). At an unfused-root self the verb is
    // self-less — no trait impl, per the canonical-arity rule.
    let mut insert_staging = TokenStream2::new();
    for i in &m.inserts {
        let method = format_ident!("insert_{}", i.arrow);
        let variant = format_ident!("Insert{}", camel(&i.arrow.to_string()));
        let member_into_trait = format_ident!("Into{}Arg", i.member);
        let member_into_fn = format_ident!("into_{}_ref", snake(&i.member.to_string()));
        let doc = format!(
            "Stage a membership insert through arrow `{}` (primitive list): the member record already exists (kept or staged); the flush replays the op through the backing's insert hook on top of the carried membership.",
            i.arrow
        );
        match &i.host {
            Some(host) => {
                let host_into_trait = format_ident!("Into{}Arg", host);
                let host_into_fn = format_ident!("into_{}_ref", snake(&host.to_string()));
                insert_staging.extend(quote! {
                    #[doc = #doc]
                    pub fn #method(&mut self, a: impl #host_into_trait, b: impl #member_into_trait) {
                        let a = a.#host_into_fn(self);
                        let b = b.#member_into_fn(self);
                        self.ops.push(Op::#variant { a, b });
                    }
                });
            }
            None => {
                insert_staging.extend(quote! {
                    #[doc = #doc]
                    pub fn #method(&mut self, b: impl #member_into_trait) {
                        let b = b.#member_into_fn(self);
                        self.ops.push(Op::#variant { b });
                    }
                });
            }
        }
    }

    // -- set_<arrow> staging (field-backed `one` arrows) --------------------
    // A correction op: re-points a KEPT record's field (and the by-derived
    // membership the field determines — the flush replays through the
    // grown `__set_<arrow>` hook). The self ref must name an old record: a
    // staged record's determining fields are fixed by its own add op, so
    // `set_` of a Pending ref is a staging bug and panics here (the plan's
    // dividing line: deterministically detectable at the staging call,
    // silently wrong if missed).
    let mut set_staging = TokenStream2::new();
    for s in &m.sets {
        let method = format_ident!("set_{}", s.arrow);
        let variant = format_ident!("Set{}", camel(&s.arrow.to_string()));
        let arrow_str = s.arrow.to_string();
        let self_into_trait = format_ident!("Into{}Arg", s.self_entity);
        let self_into_fn = format_ident!("into_{}_ref", snake(&s.self_entity.to_string()));
        let target_into_trait = format_ident!("Into{}Arg", s.target_entity);
        let target_into_fn = format_ident!("into_{}_ref", snake(&s.target_entity.to_string()));
        let doc = format!(
            "Stage a re-point of arrow `{}` on a kept record: the flush honors the correction in dangler detection (a removed old endpoint is cured on this side only) and replays the op through the store's re-point hook against the rebuilt skeleton, in submission order — a later `set_` of the same record wins.",
            s.arrow
        );
        set_staging.extend(quote! {
            #[doc = #doc]
            pub fn #method(&mut self, a: impl #self_into_trait, b: impl #target_into_trait) {
                let a = a.#self_into_fn(self);
                let b = b.#target_into_fn(self);
                match a.kind {
                    #rt::tx::RefKind::Existing(_) => {
                        self.ops.push(Op::#variant { a, b });
                    }
                    #rt::tx::RefKind::Pending(_) => panic!(
                        "set_{} of a Pending ref: a staged record's determining \
                         fields are fixed by its own add op; there is no old \
                         record to re-point",
                        #arrow_str
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

    // Per-set derived names: the last-wins effective-value table (step 1,
    // indexed by the self entity's old id) and the skeleton's pending-id
    // prediction table (indexed by set-op index, asserted at replay).
    struct SetP<'m> {
        m: &'m TxSet,
        variant: Ident2,
        eff: Ident2,
        pred: Ident2,
        hook: Ident2,
    }
    let sets: Vec<SetP<'_>> = m
        .sets
        .iter()
        .map(|s| SetP {
            m: s,
            variant: format_ident!("Set{}", camel(&s.arrow.to_string())),
            eff: format_ident!("eff_{}", s.arrow),
            pred: format_ident!("honor_pred_{}", s.arrow),
            hook: format_ident!("__set_{}", s.arrow),
        })
        .collect();

    // -- flush: the error/report surface ------------------------------------
    let report_fields = ents.iter().map(|x| {
        let f = &x.map;
        quote!(#[doc = concat!("Old->new id map of `", stringify!(#f), "'s arena: old-id-indexed, `None` where removed.")] pub #f: ::std::vec::Vec<::core::option::Option<u32>>,)
    });
    let report_init = ents.iter().map(|x| {
        let f = &x.map;
        quote!(#f: #f,)
    });
    // The analysis struct's per-entity members, in the pipeline's local
    // names, so the destructure in `flush` hands steps 3-7 the same
    // bindings the inlined pipeline used to declare.
    let analysis_struct_fields = ents.iter().flat_map(|x| {
        let (c, removed, removed_by) = (&x.count, &x.removed, &x.removed_by);
        [
            quote!(#c: usize,),
            quote!(#removed: ::std::vec::Vec<bool>,),
            quote!(#removed_by: ::std::vec::Vec<::core::option::Option<usize>>,),
        ]
    });
    // The set verbs' last-wins tables ride the analysis too: step 4's
    // skeleton init (the honor case) and step 4c's replay consume them.
    let analysis_set_fields = sets.iter().map(|s| {
        let eff = &s.eff;
        let b_ref = format_ident!("{}Ref", s.m.target_entity);
        quote!(#eff: ::std::vec::Vec<::core::option::Option<(usize, #b_ref)>>,)
    });
    let analysis_members: Vec<Ident2> = ents
        .iter()
        .flat_map(|x| [x.count.clone(), x.removed.clone(), x.removed_by.clone()])
        .chain(sets.iter().map(|s| s.eff.clone()))
        .collect();
    let analysis_fields = analysis_members.iter().map(|m| quote!(#m,));
    let analysis_names = analysis_members.iter().map(|m| quote!(#m,));
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
            #[doc = "A staged `set_` named a record the same change set removes: re-pointing needs an old record to re-point. Names both ops — the `set_` op and the removal op that got there first."]
            SetOfRemoved { op: usize, arena: &'static str, id: u32, removed_by: usize },
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

        #[doc(hidden)]
        #[doc = "The shared steps 1-2 analysis (per-arena counts, resolved removal sets with attribution, and the set verbs' last-wins effective-value tables) that `check` predicates on and `flush` consumes."]
        struct __TxAnalysis {
            #(#analysis_struct_fields)*
            #(#analysis_set_fields)*
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
    // The set verbs' effective-value tables: one slot per old record of the
    // self entity, filled last-wins (a later set_ overwrites an earlier one
    // on the same record — submission order).
    let s1_set_state = sets.iter().map(|s| {
        let eff = &s.eff;
        let x = ent_of(&s.m.self_entity);
        let c = &x.count;
        let b_ref = format_ident!("{}Ref", s.m.target_entity);
        quote!(
            let mut #eff: ::std::vec::Vec<::core::option::Option<(usize, #b_ref)>> =
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
    // Step 1, insert ops — every Existing ref range-validates exactly like
    // an add op's ctor args (the handoff deferred step-1/2 checks for
    // inserts to the keyed steps; the range check stays because the
    // alternative is an unvalidated index at replay — a panic on user
    // error, which the pipeline's contract forbids).
    let s1_insert_arms = m.inserts.iter().map(|i| {
        let variant = format_ident!("Insert{}", camel(&i.arrow.to_string()));
        let fields: Vec<(Ident2, Ident2)> = match &i.host {
            Some(host) => vec![(Ident2::new("a", proc_macro2::Span::call_site()), host.clone())],
            None => vec![],
        }
        .into_iter()
        .chain(std::iter::once((
            Ident2::new("b", proc_macro2::Span::call_site()),
            i.member.clone(),
        )))
        .collect();
        let field_names: Vec<&Ident2> = fields.iter().map(|(f, _)| f).collect();
        let checks = fields.iter().map(|(f, t_name)| {
            let t = ent_of(t_name);
            let (t_count, t_str) = (&t.count, &t.arena_str);
            quote! {
                if let #rt::tx::RefKind::Existing(id) = #f.kind {
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
        quote! {
            Op::#variant { #(#field_names),* } => {
                #(#checks)*
            }
        }
    });
    // Step 1, set ops — both refs range-validate like any other; the op
    // also records the record's effective value (last-wins) for step 2's
    // honor reading and step 4's skeleton.
    let s1_set_arms = sets.iter().map(|s| {
        let variant = &s.variant;
        let eff = &s.eff;
        let x = ent_of(&s.m.self_entity);
        let t = ent_of(&s.m.target_entity);
        let (x_count, x_str) = (&x.count, &x.arena_str);
        let (t_count, t_str) = (&t.count, &t.arena_str);
        quote! {
            Op::#variant { a, b } => {
                if let #rt::tx::RefKind::Existing(id) = a.kind {
                    if (id).0 as usize >= #x_count {
                        violations.push(Violation::UnknownOldId {
                            op: op_index,
                            arena: #x_str,
                            id: (id).0,
                        });
                    }
                }
                if let #rt::tx::RefKind::Existing(id) = b.kind {
                    if (id).0 as usize >= #t_count {
                        violations.push(Violation::UnknownOldId {
                            op: op_index,
                            arena: #t_str,
                            id: (id).0,
                        });
                    }
                }
                if let #rt::tx::RefKind::Existing(id) = a.kind {
                    if ((id).0 as usize) < #x_count {
                        #eff[(id).0 as usize] = ::core::option::Option::Some((op_index, *b));
                    }
                }
            }
        }
    });

    // Step 2, pass A — kept-record danglers over every ctor-arg field (a
    // field-backed `one` arrow): a kept record whose field target was
    // removed dangles, attributed to the removal op — unless a staged
    // `set_` re-pointed the field: the HONOR reading judges the record at
    // its EFFECTIVE endpoint (the set op's endpoint was checked at the op,
    // attributed to the naming op), so a re-pointed field skips the
    // old-target check entirely.
    let s2_kept = m.entities.iter().filter(|e| !e.ctor_args.is_empty()).map(|e| {
        let x = ent_of(&e.name);
        let (c, removed, arena, arena_str) = (&x.count, &x.removed, &x.arena, &x.arena_str);
        let checks = e.ctor_args.iter().map(|ca| {
            let t = ent_of(&ca.target_entity);
            let (t_removed, t_removed_by, t_str) = (&t.removed, &t.removed_by, &t.arena_str);
            let f = &ca.arg; // the arg name is the arrow name is the field name
            let eff = format_ident!("eff_{}", ca.arg);
            quote! {
                {
                    if #eff[old_id].is_none() {
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
    // Step 2, pass B — staged inserts naming removed endpoints (a removed
    // host or member record cannot accept membership): attributed to the
    // insert op, exactly like a staged add naming a removed endpoint.
    let s2_insert_arms = m.inserts.iter().map(|i| {
        let variant = format_ident!("Insert{}", camel(&i.arrow.to_string()));
        let fields: Vec<(Ident2, Ident2)> = match &i.host {
            Some(host) => vec![(Ident2::new("a", proc_macro2::Span::call_site()), host.clone())],
            None => vec![],
        }
        .into_iter()
        .chain(std::iter::once((
            Ident2::new("b", proc_macro2::Span::call_site()),
            i.member.clone(),
        )))
        .collect();
        let field_names: Vec<&Ident2> = fields.iter().map(|(f, _)| f).collect();
        let checks = fields.iter().map(|(f, t_name)| {
            let t = ent_of(t_name);
            let (t_count, t_removed, t_str) = (&t.count, &t.removed, &t.arena_str);
            quote! {
                if let #rt::tx::RefKind::Existing(id) = #f.kind {
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
            if let Op::#variant { #(#field_names),* } = op {
                #(#checks)*
            }
        }
    });
    // Step 2, set ops — the correction-op checks, attributed to the set op
    // (the op that NAMED the endpoint, the staged-op precedent): a `set_`
    // of a removed record is SetOfRemoved, and a `set_` naming a removed
    // endpoint dangles at the set op. Range-guarded: a flagged id is never
    // indexed. Runs before the kept-record pass: the honor reading reports
    // the set op's own violations first, and the kept pass then skips every
    // field a set re-pointed.
    let s2_setop_arms = sets.iter().map(|s| {
        let variant = &s.variant;
        let x = ent_of(&s.m.self_entity);
        let t = ent_of(&s.m.target_entity);
        let (x_count, x_removed, x_removed_by, x_str) =
            (&x.count, &x.removed, &x.removed_by, &x.arena_str);
        let (t_count, t_removed, t_str) = (&t.count, &t.removed, &t.arena_str);
        quote! {
            if let Op::#variant { a, b } = op {
                if let #rt::tx::RefKind::Existing(id) = a.kind {
                    if ((id).0 as usize) < #x_count {
                        if #x_removed[(id).0 as usize] {
                            violations.push(Violation::SetOfRemoved {
                                op: op_index,
                                arena: #x_str,
                                id: (id).0,
                                removed_by: #x_removed_by[(id).0 as usize].unwrap(),
                            });
                        }
                        if let #rt::tx::RefKind::Existing(nid) = b.kind {
                            if ((nid).0 as usize) < #t_count && #t_removed[(nid).0 as usize] {
                                violations.push(Violation::DanglingEndpoint {
                                    op: op_index,
                                    record: RecordDesc::Kept { arena: #x_str, id: (id).0 },
                                    endpoint_arena: #t_str,
                                    endpoint: (nid).0,
                                });
                            }
                        }
                    }
                }
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
    // Step 4b-pre — the intrusive carries, DERIVED before any write: per
    // binding, new heads (indexed by the host's NEW id; one value for a
    // store-level head) and new links (indexed by the member's new id),
    // from walking the OLD chains (kept hosts only, removed members
    // skipped, ids remapped). In-place compaction (the owned path)
    // destroys old chains as it writes, so every derived array exists
    // before the first store write (spike 7's ordering constraint).
    let s4_arrays = m.intrusive.iter().map(|i| {
        let member = ent_of(&i.member);
        let arrow = &i.arrow;
        let (heads, head_val, links) = (
            format_ident!("__{}_heads", arrow),
            format_ident!("__{}_head", arrow),
            format_ident!("__{}_links", arrow),
        );
        let (m_arena, m_map, m_link) =
            (&member.arena, &member.map, &i.member_link);
        let links_decl = quote! {
            let mut #links: ::std::vec::Vec<::core::option::Option<u32>> =
                ::std::vec![::core::option::Option::None; #m_map.iter().flatten().count()];
        };
        let walk_body = |head_set: TokenStream2, first: TokenStream2| {
            quote! {
                let mut prev: ::core::option::Option<u32> = ::core::option::Option::None;
                let mut cur = #first;
                while let ::core::option::Option::Some(old_member) = cur {
                    cur = __old.#m_arena[old_member.0 as usize].#m_link;
                    if let ::core::option::Option::Some(new_member) = #m_map[old_member.0 as usize] {
                        match prev {
                            ::core::option::Option::None => { #head_set }
                            ::core::option::Option::Some(p) => {
                                #links[p as usize] = ::core::option::Option::Some(new_member);
                            }
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
                    quote! { #heads[new_host as usize] = ::core::option::Option::Some(new_member); },
                    quote!(__old.#h_arena[old_host].#head),
                );
                quote! {
                    #links_decl
                    let mut #heads: ::std::vec::Vec<::core::option::Option<u32>> =
                        ::std::vec![::core::option::Option::None; #h_map.iter().flatten().count()];
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
                    quote! { #head_val = ::core::option::Option::Some(new_member); },
                    quote!(__old.#head),
                );
                quote! {
                    #links_decl
                    let mut #head_val: ::core::option::Option<u32> = ::core::option::Option::None;
                    #body
                }
            }
        }
    });
    // A field of a kept record's skeleton literal: ctor-arg ids remapped
    // through the target's map; a derived head/link from its array
    // (indexed by THIS record's new id); containers Default.
    let record_field_init = |entity: &Ident2, field: &TxField| -> TokenStream2 {
        let n = &field.emitted;
        match &field.init {
            TxFieldInit::CtorArg { target, arrow } => {
                let t = ent_of(target);
                let (t_map, t_id, t_removed) = (&t.map, &t.id_ty, &t.removed);
                match m.sets.iter().find(|s| s.self_entity == *entity && s.arrow == *arrow) {
                    Some(set) => {
                        let eff = format_ident!("eff_{}", set.arrow);
                        let pred = format_ident!("honor_pred_{}", set.arrow);
                        let add_variant = format_ident!("Add{}", set.target_entity);
                        quote! {
                            #n: #t_id(
                                if #t_removed[(rec.#n).0 as usize] {
                                    // HONOR case: the old endpoint was
                                    // removed and a staged `set_` re-points
                                    // this field (step 2 proved one exists —
                                    // otherwise the record dangled). The
                                    // skeleton record names its EFFECTIVE
                                    // endpoint and is born UNLINKED: the
                                    // walk-rebuild visits kept hosts' old
                                    // chains only, and the removed old
                                    // host's chain — the only list that
                                    // held this record — is skipped. A
                                    // Pending endpoint is predicted (kept
                                    // count + adds before its op) and
                                    // asserted at replay, never trusted.
                                    let (set_op, b) = #eff[old_id].expect(
                                        "a staged set_ re-points this field: step 2 honor check",
                                    );
                                    match b.kind {
                                        #rt::tx::RefKind::Existing(id) => #t_map[(id).0 as usize]
                                            .expect("effective endpoint kept: step 2 honor check"),
                                        #rt::tx::RefKind::Pending(i) => {
                                            let adds = ops[..(i) as usize]
                                                .iter()
                                                .filter(|o| matches!(o, Op::#add_variant { .. }))
                                                .count() as u32;
                                            let p = #t_map.iter().flatten().count() as u32 + adds;
                                            #pred[set_op] = ::core::option::Option::Some(p);
                                            p
                                        }
                                    }
                                } else {
                                    #t_map[(rec.#n).0 as usize]
                                        .expect("field target kept: step 2 proved it")
                                }
                            ),
                        }
                    }
                    None => quote! {
                        #n: #t_id(#t_map[(rec.#n).0 as usize]
                            .expect("field target kept: step 2 proved it")),
                    },
                }
            }
            TxFieldInit::None_ => {
                if let Some(i) = m.intrusive.iter().find(|i| {
                    i.host.as_ref() == Some(entity) && i.head_field == field.emitted
                }) {
                    let member = ent_of(&i.member);
                    let heads = format_ident!("__{}_heads", i.arrow);
                    let m_id = &member.id_ty;
                    return quote!(#n: #heads[new_id as usize].map(#m_id),);
                }
                if let Some(i) = m
                    .intrusive
                    .iter()
                    .find(|i| i.member == *entity && i.member_link == field.emitted)
                {
                    let member = ent_of(&i.member);
                    let links = format_ident!("__{}_links", i.arrow);
                    let m_id = &member.id_ty;
                    return quote!(#n: #links[new_id as usize].map(#m_id),);
                }
                quote!(#n: ::core::option::Option::None,)
            }
            TxFieldInit::New => quote!(#n: ::std::default::Default::default(),),
        }
    };
    // A store-level field: a root-hosted arrow's single head value, else
    // the uniform init.
    let store_field_expr = |field: &TxField| -> TokenStream2 {
        if let Some(i) = m
            .intrusive
            .iter()
            .find(|i| i.host.is_none() && i.head_field == field.emitted)
        {
            let member = ent_of(&i.member);
            let head_val = format_ident!("__{}_head", i.arrow);
            let m_id = &member.id_ty;
            return quote!(#head_val.map(#m_id));
        }
        match field.init {
            TxFieldInit::None_ => quote!(::core::option::Option::None),
            _ => quote!(::std::default::Default::default()),
        }
    };

    // Step 4a — the kept skeleton, two write paths over the SAME derived
    // data. Records push/compact in old arena order with on_add hooks
    // fired per kept record.
    let s4_borrowed_arena_inits = m.entities.iter().map(|e| {
        let x = ent_of(&e.name);
        let (arena, c) = (&x.arena, &x.count);
        quote!(#arena: ::std::vec::Vec::with_capacity(#c),)
    });
    let s4_borrowed_store_inits = m.store_fields.iter().map(|f| {
        let n = &f.emitted;
        let v = store_field_expr(f);
        quote!(#n: #v,)
    });
    let s4_borrowed_record_loops = m.entities.iter().map(|e| {
        let x = ent_of(&e.name);
        let (c, map, arena, data, id_ty) =
            (&x.count, &x.map, &x.arena, &x.data, &x.id_ty);
        let field_inits = e.fields.iter().map(|f| record_field_init(&e.name, f));
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
    // The owned path: forward in-place compaction — every write slot j <=
    // its read slot old_id, and all derived values (maps, arrays) were
    // computed before the first write, so overwriting a compacted slot
    // can never corrupt a later read.
    let s4_owned_compactions = m.entities.iter().map(|e| {
        let x = ent_of(&e.name);
        let (c, map, arena, data, id_ty) =
            (&x.count, &x.map, &x.arena, &x.data, &x.id_ty);
        let field_inits = e.fields.iter().map(|f| record_field_init(&e.name, f));
        let onadd_fires = e.onadd_hooks.iter().map(|h| {
            quote!(new_store.#h(#id_ty(new_id));)
        });
        quote! {
            {
                let mut j = 0usize;
                for old_id in 0..#c {
                    if let ::core::option::Option::Some(new_id) = #map[old_id] {
                        let rec = &new_store.#arena[old_id];
                        let remapped = #data { #(#field_inits)* };
                        new_store.#arena[j] = remapped;
                        j += 1;
                        #(#onadd_fires)*
                        ::core::debug_assert_eq!(new_id as usize, j - 1);
                    }
                }
                new_store.#arena.truncate(j);
            }
        }
    });
    let s4_owned_store_assigns = m.store_fields.iter().map(|f| {
        let n = &f.emitted;
        let v = store_field_expr(f);
        quote!(new_store.#n = #v;)
    });

    // Step 4c — replay constructive ops in submission order through the
    // REAL constructors (on_add growth + insert hooks fire as construction
    // always fires them). Removals never replay. A `Pending` ref resolves
    // through its entity's replay table; a miss is an internal bug.
    let s4_honor_preds = sets.iter().map(|s| {
        let pred = &s.pred;
        quote!(
            let mut #pred: ::std::vec::Vec<::core::option::Option<u32>> =
                ::std::vec![::core::option::Option::None; ops.len()];
        )
    });
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
    // Insert replays resolve both refs (Existing through the compaction
    // map, Pending through the replay table) and call the backing's insert
    // hook on the new store — the hook threads the member into the host's
    // list on top of the carried skeleton.
    let s4c_insert_arms = m.inserts.iter().map(|i| {
        let variant = format_ident!("Insert{}", camel(&i.arrow.to_string()));
        let hook = format_ident!("__insert_{}", i.arrow);
        let fields: Vec<(Ident2, Ident2)> = match &i.host {
            Some(host) => vec![(Ident2::new("a", proc_macro2::Span::call_site()), host.clone())],
            None => vec![],
        }
        .into_iter()
        .chain(std::iter::once((
            Ident2::new("b", proc_macro2::Span::call_site()),
            i.member.clone(),
        )))
        .collect();
        let field_names: Vec<&Ident2> = fields.iter().map(|(f, _)| f).collect();
        let resolves = fields.iter().map(|(f, t_name)| {
            let t = ent_of(t_name);
            let (t_map, t_p, t_id) = (&t.map, &t.pending, &t.id_ty);
            quote! {
                let #f = match #f.kind {
                    #rt::tx::RefKind::Existing(id) =>
                        #t_id(#t_map[(id).0 as usize].expect("Existing id validated in steps 1-2")),
                    #rt::tx::RefKind::Pending(i) => #t_p[i as usize]
                        .expect("an add precedes its use: submission order"),
                };
            }
        });
        quote! {
            Op::#variant { #(#field_names),* } => {
                #(#resolves)*
                new_store.#hook(#(#field_names),*);
            }
        }
    });
    // Set replays lower through the grown re-point hook AGAINST THE
    // REBUILT STATE: the skeleton left a re-pointed record under its old
    // host's carried list (or born unlinked in the honor case — the total
    // unlink legitimately finds nothing), and the hook unlinks, writes,
    // and re-links in one call. A Pending effective endpoint asserts the
    // skeleton's prediction (the honor case): the id is deterministic, and
    // a mismatch is an internal bug, never trusted.
    let s4c_set_arms = sets.iter().map(|s| {
        let variant = &s.variant;
        let hook = &s.hook;
        let pred = &s.pred;
        let x = ent_of(&s.m.self_entity);
        let t = ent_of(&s.m.target_entity);
        let (x_map, x_id) = (&x.map, &x.id_ty);
        let (t_map, t_pending, t_id) = (&t.map, &t.pending, &t.id_ty);
        quote! {
            Op::#variant { a, b } => {
                let a = match a.kind {
                    #rt::tx::RefKind::Existing(id) =>
                        #x_id(#x_map[(id).0 as usize].expect("Existing id validated in steps 1-2")),
                    #rt::tx::RefKind::Pending(_) =>
                        ::core::unreachable!("staging rejects Pending self refs on set_"),
                };
                let b = match b.kind {
                    #rt::tx::RefKind::Existing(id) =>
                        #t_id(#t_map[(id).0 as usize].expect("Existing id validated in steps 1-2")),
                    #rt::tx::RefKind::Pending(i) => #t_pending[i as usize]
                        .expect("an add precedes its use: submission order"),
                };
                if let ::core::option::Option::Some(p) = #pred[op_index] {
                    ::core::assert_eq!(b.0, p, "skeleton pending-id prediction (honor case)");
                }
                new_store.#hook(a, b);
            }
        }
    });

    let flush_method = quote! {
        #[doc(hidden)]
        #[doc = "Steps 1-2 as one pure analysis over (old store, ops): the resolved removal sets (with attributing op indices), per-arena counts, and every violation found in old-id space. `check` and `flush` consume the same function, so a green check and a committing flush can never disagree."]
        fn __analyze(
            old: &#store,
            ops: &[Op],
        ) -> ::core::result::Result<__TxAnalysis, FlushError> {
            // -- step 1: resolve removals; validate every Existing ref -----
            #(#s1_counts)*
            #(#s1_state)*
            #(#s1_set_state)*
            let mut violations: ::std::vec::Vec<Violation> = ::std::vec::Vec::new();
            for (op_index, op) in ops.iter().enumerate() {
                match op {
                    #(#s1_add_arms)*
                    #(#s1_remove_arms)*
                    #(#s1_insert_arms)*
                    #(#s1_set_arms)*
                }
            }

            // -- step 2: dangler detection, in old-id space ----------------
            // Set ops first (the correction-op checks, attributed to the
            // set op), then kept records (skipping fields a set re-pointed
            // — the honor reading), then staged adds and inserts naming
            // removed endpoints. Runs before compaction so the report
            // names the ids the user actually staged with.
            for (op_index, op) in ops.iter().enumerate() {
                #(#s2_setop_arms)*
            }
            #(#s2_kept)*
            for (op_index, op) in ops.iter().enumerate() {
                #(#s2_staged_arms)*
                #(#s2_insert_arms)*
            }
            if !violations.is_empty() {
                return ::core::result::Result::Err(FlushError { violations });
            }
            ::core::result::Result::Ok(__TxAnalysis {
                #(#analysis_fields)*
            })
        }

        #[doc = "The flush's first two steps as a pure predicate: resolves removals and collects every violation (unknown ids, double removals, danglers) the flush would reject with — without building anything. A green check means a subsequent `flush` of the SAME change set passes steps 1-2 (the committed-mode contract's read half; `into_edit` is the other)."]
        pub fn check(&self) -> ::core::result::Result<(), FlushError> {
            Self::__analyze(self.old.__store(), &self.ops).map(|_| ())
        }

        #[doc = "Run the flush pipeline and consume the transaction: (1) resolve removals and range-validate every `Existing` ref, (2) dangler detection over kept records and staged adds in old-id space — steps 1-2 are the same `check` analysis, (3) compaction with per-arena old->new id maps, (4) the kept skeleton (records with remapped id fields, intrusive lists carried by arrays derived from the old chains BEFORE any write) then submission-order replay through the same constructors construction uses, (5) induction (constructor logic — none staged by the built-ins), (6) backing finalization (a phased store's freeze — flat stores have none), (7) `validate()`. A borrowed transaction builds fresh arenas; an owned one (`into_edit`) forward-compacts in place — every write slot at or before its read slot — reusing the old store's allocations. The old store (when borrowed) is untouched; a rejection leaves nothing half-applied."]
        pub fn flush(self) -> ::core::result::Result<FlushOutcome, FlushError> {
            let Tx { old, token: _, ops } = self;
            let __TxAnalysis {
                #(#analysis_names)*
            } = Self::__analyze(old.__store(), &ops)?;

            // The skeleton's pending-id prediction tables (the honor case):
            // filled while a re-pointed field's Pending endpoint is
            // predicted, asserted at replay.
            #(#s4_honor_preds)*

            // -- step 3: compaction; per-arena old->new id maps ------------
            #(#s3_maps)*

            // -- step 4b-pre: derive the carries from the OLD store --------
            // All derived data (maps above, walk arrays here) exists before
            // the first store write: the owned path's in-place compaction
            // overwrites old chains as it goes.
            let __old = old.__store();
            #(#s4_arrays)*

            // -- step 4a: the kept skeleton, then replay --------------------
            let mut new_store = match old {
                OldStore::Borrowed(old) => {
                    let mut new_store = #store {
                        #(#s4_borrowed_arena_inits)*
                        #(#s4_borrowed_store_inits)*
                    };
                    #(#s4_borrowed_record_loops)*
                    new_store
                }
                OldStore::Owned(mut boxed) => {
                    let mut new_store = *boxed;
                    #(#s4_owned_compactions)*
                    #(#s4_owned_store_assigns)*
                    new_store
                }
            };

            // -- step 4c: replay constructive ops in submission order ------
            #(#s4c_pending)*
            for (op_index, op) in ops.iter().enumerate() {
                match op {
                    #(#s4c_add_arms)*
                    #(#s4c_insert_arms)*
                    #(#s4c_set_arms)*
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
        #handle_impls
        #flush_types

        #[doc(hidden)]
        #[doc = concat!("What a `Tx` stages against: the shared borrow (`edit`) or the consumed store (`into_edit` — flush then reuses its arena allocations).")]
        enum OldStore<'a> {
            Borrowed(&'a #store),
            Owned(::std::boxed::Box<#store>),
        }

        impl OldStore<'_> {
            fn __store(&self) -> &#store {
                match self {
                    OldStore::Borrowed(s) => s,
                    OldStore::Owned(b) => b,
                }
            }
        }

        #[doc = concat!("Transaction over `", stringify!(#store), "`: stages a change set against the old store and flushes it as a full rebuild. Staging never touches the store — the change set is a plain queue, a first-class value.")]
        pub struct Tx<'a> {
            pub(crate) old: OldStore<'a>,
            pub(crate) token: #rt::tx::TxToken,
            pub(crate) ops: ::std::vec::Vec<Op>,
        }

        impl #store {
            #[doc = "Borrow the store sharedly and start staging. Reads and old handles keep working while staging and after the flush; the old store is never mutated (the escape hatch — `into_edit` enforces the pass discipline at compile time)."]
            pub fn edit(&self) -> Tx<'_> {
                Tx {
                    old: OldStore::Borrowed(self),
                    token: #rt::tx::mint_token(),
                    ops: ::std::vec::Vec::new(),
                }
            }

            #[doc = "Consume the store and start staging. The borrow checker then requires every old handle to be dead before the pass can end, and `flush` reuses the store's arena allocations (in-place compaction) instead of cloning. Old-store reads during staging are by raw id — handles into the moved store cannot exist."]
            pub fn into_edit(self) -> Tx<'static> {
                Tx {
                    old: OldStore::Owned(::std::boxed::Box::new(self)),
                    token: #rt::tx::mint_token(),
                    ops: ::std::vec::Vec::new(),
                }
            }
        }

        impl Tx<'_> {
            /// The old store this transaction stages against — the read
            /// side of the pass discipline. On a borrowed transaction
            /// (`edit`) this is the original store and old handles still
            /// work; on an owned one (`into_edit`) it borrows the store
            /// the transaction now owns (reads by raw id; handles into it
            /// cannot exist by construction).
            pub fn old(&self) -> &#store {
                self.old.__store()
            }

            #staging

            #insert_staging

            #set_staging

            #flush_method
        }

        #write_impls
    }
}
