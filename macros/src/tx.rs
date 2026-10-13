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
    /// Arena field on the store (`node_arena`) — or, when embedded, on the
    /// host record.
    pub arena: Ident2,
    /// Embedding host's entity name when this entity's arena is a `Vec`
    /// field of the host entity's record (a nested store — the composed
    /// placement, depth 1). Embedded entities stage adds through `SubTx`
    /// scopes and removals by host ref + old index; nothing stages a ref
    /// TO an embedded record this step (their adds return no `Pending`).
    pub host: Option<Ident2>,
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
    /// emits it for a closed non-root entity. `None` on the composed Tx
    /// (per-instance entity traits are not yet minted for compositions).
    pub remove_trait: Option<TokenStream2>,
}

pub(crate) struct TxCtorArg {
    pub arg: Ident2,
    /// The contributing arrow's unique local basis — the `set_` verb's
    /// identity, so the honor lookup (`eff_<basis>`) and the field init
    /// agree even where a composed store prefixes the arrow (the arg
    /// names the field; the basis names the verb). Flat: the arrow name.
    pub basis: Ident2,
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
    /// A host record's embedded arena — `Vec<<Sub>Data>` of the named
    /// embedded entity. The skeleton rebuilds it filtered to the sub
    /// records kept under this host, each sub record's own fields remapped
    /// (the per-host compaction map indexed by THIS host's old id).
    NestedArena(Ident2),
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
/// backing's insert hook at flush.
pub(crate) struct TxInsert {
    /// The arrow's unique local basis: the op variant and the inherent
    /// staging verb are named after it (the composed codegen qualifies it
    /// with the instance on cross-instance arrow-name collision).
    pub arrow: Ident2,
    /// The self (host) entity; `None` at an unfused-root self — the verb
    /// is inherent-only in its self-less form (canonical arity takes the
    /// self ref, which does not exist there).
    pub host: Option<Ident2>,
    /// The member (target) entity.
    pub member: Ident2,
    /// The backing's insert hook (the replay target — instance-qualified
    /// in a composed store).
    pub hook: Ident2,
    /// The `Insert<Arrow>` trait impl, `None` on the self-less form.
    pub verb: Option<TxVerb>,
}

/// The csr carry of one bound phased (`csr_adjacency`) `many` arrow (step
/// 12): kept membership is carried by WALKING the old store's frozen
/// ranges — per kept host record in old arena order, its
/// `offsets[old]..offsets[old+1]` slice of `adj` in frozen forward order,
/// removed members skipped, kept ids remapped — and feeding each kept
/// member to the backing's `on_insert` hook (a pure bucket append on the
/// new Build-phase store). Never per-field remapped (a removed mid-range
/// member would truncate), never rebuilt from arena order (backing-agnostic
/// carry requires the old iteration order). The hosts' buckets are opened
/// by the skeleton's per-kept-record `on_add` fires, so the walk runs
/// after the skeleton and before replay (staged inserts append to the
/// carried tails).
pub(crate) struct TxCsr {
    /// The self (host) entity — always a record entity (a phased backing
    /// at the unfused root was rejected at applicability).
    pub host: Ident2,
    /// The member (target) entity.
    pub member: Ident2,
    /// The backing's `on_insert` hook (the carry's write target).
    pub insert_hook: Ident2,
    /// The frozen store field holding the per-host range starts
    /// (`offsets: Vec<u32>`, arena length + 1).
    pub offsets_field: Ident2,
    /// The frozen store field holding the members (`adj: Vec<BId>`).
    pub adj_field: Ident2,
}

/// The `insert_<arrow>` staging verb of one bound keyed arrow. A keyed
/// arrow is a function: the staging call rejects a live key — checked
/// against the borrowed old store and the already-staged inserts (a
/// staged removal frees no key; re-pointing one is two flushes). The
/// flush carries the map as state (filter to entries whose key and
/// target both survive, remap through the compaction maps) and replays
/// staged inserts through the backing's check-then-write hook on top of
/// the carry. Two placements: a store-level map (the self is the
/// unfused root; the verb is `insert_<arrow>(key, target)`) and a
/// per-record map riding the host entity's records (the verb is
/// `insert_<arrow>(host, key, target)`). A per-record map whose target
/// is embedded rides the target as a RAW PATH ID (`member_host` names
/// the target's host entity) — no ref can name an embedded record, so
/// the probe's `Existing`-only restriction pins the staging shape.
pub(crate) struct TxKeyed {
    /// The arrow's unique local basis: the op variant, the inherent
    /// staging verb, and the derived carry map are named after it (the
    /// composed codegen qualifies it with the instance on cross-instance
    /// arrow-name collision).
    pub arrow: Ident2,
    /// The map-owning host entity for a per-record map (`None` = a
    /// store-level map at the unfused root).
    pub host: Option<Ident2>,
    /// The key entity — whose arena, compaction map, and pending table
    /// the key ref resolves through at flush. Always root-level.
    pub key_entity: Ident2,
    /// The target (value) entity.
    pub member: Ident2,
    /// The target's host entity when the target is embedded (`Some`) —
    /// the op then rides the target as a raw path id, and the flush
    /// remaps it through the composed maps (host map ∘ per-host sub map)
    /// at carry AND staged-insert replay.
    pub member_host: Option<Ident2>,
    /// The map field (on the store at a store-level placement; on the
    /// host record at a per-record placement).
    pub map_field: Ident2,
    /// The backing's insert hook (check-then-write — the replay target).
    pub hook: Ident2,
    /// The old-store live-key predicate (the pure form of the check the
    /// staging call makes).
    pub key_live: Ident2,
    /// The `Insert<Arrow>` trait impl (the courier's write slot) — a
    /// per-record map's self ref makes the canonical arity available;
    /// `None` on the store-level (self-less) form and wherever the
    /// declaring-block surface is not wired.
    pub verb: Option<TxVerb>,
}

/// The `set_<arrow>` staging verb of one field-backed `one` arrow (a
/// `ctor_arg` backing — the builder's re-point hook is the orchestration:
/// unlink the by-derived derivatives, write, re-link). The flush honors
/// staged corrections in dangler detection (the probe's honor reading)
/// and replays through the same hook against the rebuilt skeleton.
pub(crate) struct TxSet {
    /// The arrow's unique local basis: the op variant `Set<camel>`, the
    /// inherent staging verb, and the effective-value table are named
    /// after it (also the ctor-arg field's name-mate for the honor
    /// lookup). The composed codegen qualifies it with the instance on
    /// cross-instance arrow-name collision.
    pub arrow: Ident2,
    /// The self entity (whose record's field is re-pointed).
    pub self_entity: Ident2,
    /// The field's target entity (the new endpoint's resolution arena).
    pub target_entity: Ident2,
    /// The store's re-point hook (the replay target — instance-qualified
    /// in a composed store).
    pub hook: Ident2,
    /// The `Set<Arrow>` trait impl (the courier's write slot); `None`
    /// where the declaring-block surface is not wired (the composed Tx
    /// is inherent-surface until the composed write-trait surface lands).
    pub verb: Option<TxVerb>,
}

/// The whole store as the transaction sees it.
pub(crate) struct TxModel {
    pub store: Ident2,
    /// Runtime-crate absolute path (`::metagraph`, rename-aware).
    pub rt: TokenStream2,
    /// The store binds a phased backing (`csr_adjacency`): the Tx borrows
    /// `Store<Frozen>`, the flush builds a `Store<Build>` (skeleton + walk
    /// feeds + replay through the real ctors/hooks) and ends in
    /// `finish()` (freeze, then validate). The owned path (`into_edit`)
    /// is not emitted for a phased store — in-place forward compaction
    /// against live build-phase accumulators is not pinned by any probe.
    pub phased: bool,
    /// Record-owning entities in schema order (root entity excluded — it
    /// owns no record, id, or removal op).
    pub entities: Vec<TxEntity>,
    /// Store-level contributed fields (a root-sourced arrow's head, keyed
    /// maps) with their skeleton init, for the new store's literal.
    pub store_fields: Vec<TxField>,
    /// Intrusive-carry bindings in schema arrow order.
    pub intrusive: Vec<TxIntrusive>,
    /// Csr (phased) carry bindings in schema arrow order.
    pub csr: Vec<TxCsr>,
    /// Primitive-insert verbs in schema arrow order (non-`by` bound many
    /// arrows with an insert hook — by-derived and existence arrows stage
    /// no insert).
    pub inserts: Vec<TxInsert>,
    /// `set_<arrow>` verbs in schema arrow order (field-backed `one`
    /// arrows — every ctor-arg arrow grows the re-point hook).
    pub sets: Vec<TxSet>,
    /// Store-level keyed arrows in schema arrow order (the self is the
    /// unfused root; per-record keyed maps gate the Tx off until the
    /// keyed-carry step).
    pub keyed: Vec<TxKeyed>,
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

pub(crate) fn emit(m: &TxModel) -> TokenStream2 {
    let store = &m.store;
    let rt = &m.rt;
    // The store type at the Tx's three phase positions: the old store a
    // Tx stages against and the FlushOutcome's payload are FROZEN (the Tx
    // presupposes a frozen old store — the csr carry reads its frozen
    // ranges; reads exist only on Frozen); the rebuilt skeleton mid-flush
    // is BUILD (the phased accumulators are live until `finish()`). A
    // non-phased store is bare at every position (M1 shape).
    let store_frozen: TokenStream2 = if m.phased {
        quote!(#store<Frozen>)
    } else {
        quote!(#store)
    };
    let store_build: TokenStream2 = if m.phased {
        quote!(#store<Build>)
    } else {
        quote!(#store)
    };

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
    // `Pending` ref payload and the flush error attribution handle. A
    // nested op carries its path as the HOST REF (never a resolved id —
    // the host may be `Pending`; embedding depth 1, so the path is the
    // single segment), and a nested removal names its target by host ref +
    // old embedded-arena index (a path id is a resolved id — exactly what
    // staging must not require).
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
                host: None, // linked below, once every name is known
                count: format_ident!("old_{}_count", snake(&e.name.to_string())),
                map: format_ident!("{}_map", snake(&e.name.to_string())),
                removed: format_ident!("removed_{}", snake(&e.name.to_string())),
                removed_by: format_ident!("removed_by_{}", snake(&e.name.to_string())),
                pending: format_ident!("pending_{}", snake(&e.name.to_string())),
                snake: snake_name,
            }
        })
        .collect();
    // Link each embedded entity to its host's index (a host is itself a
    // root-level entity — embedding depth 1).
    let mut ents = ents;
    for (i, e) in m.entities.iter().enumerate() {
        if let Some(h) = &e.host {
            let hi = ents.iter().position(|x| &x.name == h).expect("host entity");
            ents[i].host = Some(hi);
        }
    }
    let ents = ents;
    let ent_of = |name: &Ident2| ents.iter().find(|x| &x.name == name).expect("entity");
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
        match &e.host {
            None => op_variants.extend(quote! {
                #add { #(#arg_fields)* },
                #remove { #payload: #ref_alias },
            }),
            Some(host) => {
                let host_ref = format_ident!("{}Ref", host);
                op_variants.extend(quote! {
                    #add { host: #host_ref, #(#arg_fields)* },
                    #remove { host: #host_ref, idx: u32 },
                });
            }
        }
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
    for kk in &m.keyed {
        let variant = format_ident!("Insert{}", camel(&kk.arrow.to_string()));
        let k_ref = format_ident!("{}Ref", kk.key_entity);
        let b_ref = format_ident!("{}Ref", kk.member);
        match (&kk.host, &kk.member_host) {
            (None, None) => op_variants.extend(quote! {
                #variant { k: #k_ref, b: #b_ref },
            }),
            (Some(host), None) => {
                let a_ref = format_ident!("{}Ref", host);
                op_variants.extend(quote! {
                    #variant { a: #a_ref, k: #k_ref, b: #b_ref },
                });
            }
            (Some(host), Some(_)) => {
                // An embedded target rides as a raw path id: no ref can
                // name an embedded record (the probe's Existing-only pin).
                let a_ref = format_ident!("{}Ref", host);
                let b_id = &ent_of(&kk.member).id_ty;
                op_variants.extend(quote! {
                    #variant { a: #a_ref, k: #k_ref, b: #b_id },
                });
            }
            (None, Some(_)) => unreachable!(
                "a store-level keyed map naming an embedded target has no \
                 host to agree with — gated off at classification"
            ),
        }
    }
    // The induction escape hatch (step 10): a user-staged opaque op. The
    // `run` closure replays against the rebuilt store during step 4c (the
    // inducing-constructor case); `named` lists the old-store ids the op
    // names so steps 1-2 can range-check them and reject removed ones at
    // the naming op; `reserve` lists the keyed slots an inducing op will
    // insert, so a later keyed staging call sees them as live (the
    // induct-flush probe's induced-key staging check). Reservation rides
    // INSIDE the op queue — a keyed insert staged BEFORE the custom op
    // must not see its reserved keys, and the probe pins both orderings.
    // `run` is an `Option` because the replay loop borrows the queue
    // mutably and takes it; a custom op replays exactly once.
    let root_entities: Vec<&TxEntity> = m.entities.iter().filter(|e| e.host.is_none()).collect();
    let custom_variant = quote! {
        #[doc = "A user-staged custom op (the induction escape hatch, staged via `__stage_custom`): an opaque `FnOnce` the flush replays against the rebuilt store during replay. `named` lists the old-store ids the op names — step 1 range-checks them, step 2 rejects a removed one at this op. `reserve` lists the keyed slots an already-staged inducing op will insert — a later keyed staging call sees them as live."]
        Custom {
            run: ::core::option::Option<
                ::std::boxed::Box<dyn ::core::ops::FnOnce(&mut #store_build, &Resolve<'_>)>,
            >,
            named: ::std::vec::Vec<__NamedRef>,
            reserve: ::std::vec::Vec<ReservedKey>,
        },
    };
    // The op enum's `Debug`: the `Custom` variant holds a closure, so the
    // derive is out — this manual impl prints every variant's payload and
    // the custom op's inspectable fields (`named`, `reserve`) with the
    // closure elided. Custom ops are user code; what they will do is not
    // derivable (noted in the step-10 notes).
    let mut op_debug_arms = TokenStream2::new();
    for e in &m.entities {
        let add = format_ident!("Add{}", e.name);
        let remove = format_ident!("Remove{}", e.name);
        let payload = format_ident!("{}", snake(&e.name.to_string()));
        match &e.host {
            None => {
                let arg_names: Vec<&Ident2> = e.ctor_args.iter().map(|c| &c.arg).collect();
                op_debug_arms.extend(quote! {
                    Op::#add { #(#arg_names),* } => {
                        let mut d = f.debug_struct(stringify!(#add));
                        #(d.field(stringify!(#arg_names), #arg_names);)*
                        d.finish()
                    }
                    Op::#remove { #payload } => {
                        f.debug_struct(stringify!(#remove)).field(stringify!(#payload), #payload).finish()
                    }
                });
            }
            Some(_) => {
                let arg_names: Vec<&Ident2> = e.ctor_args.iter().map(|c| &c.arg).collect();
                op_debug_arms.extend(quote! {
                    Op::#add { host, #(#arg_names),* } => {
                        let mut d = f.debug_struct(stringify!(#add));
                        d.field("host", host);
                        #(d.field(stringify!(#arg_names), #arg_names);)*
                        d.finish()
                    }
                    Op::#remove { host, idx } => {
                        f.debug_struct(stringify!(#remove))
                            .field("host", host)
                            .field("idx", idx)
                            .finish()
                    }
                });
            }
        }
    }
    for i in &m.inserts {
        let variant = format_ident!("Insert{}", camel(&i.arrow.to_string()));
        match &i.host {
            Some(_) => op_debug_arms.extend(quote! {
                Op::#variant { a, b } => {
                    f.debug_struct(stringify!(#variant)).field("a", a).field("b", b).finish()
                }
            }),
            None => op_debug_arms.extend(quote! {
                Op::#variant { b } => {
                    f.debug_struct(stringify!(#variant)).field("b", b).finish()
                }
            }),
        }
    }
    for s in &m.sets {
        let variant = format_ident!("Set{}", camel(&s.arrow.to_string()));
        op_debug_arms.extend(quote! {
            Op::#variant { a, b } => {
                f.debug_struct(stringify!(#variant)).field("a", a).field("b", b).finish()
            }
        });
    }
    for kk in &m.keyed {
        let variant = format_ident!("Insert{}", camel(&kk.arrow.to_string()));
        match (&kk.host, &kk.member_host) {
            (None, None) => op_debug_arms.extend(quote! {
                Op::#variant { k, b } => {
                    f.debug_struct(stringify!(#variant)).field("k", k).field("b", b).finish()
                }
            }),
            (Some(_), None) => op_debug_arms.extend(quote! {
                Op::#variant { a, k, b } => {
                    f.debug_struct(stringify!(#variant)).field("a", a).field("k", k).field("b", b).finish()
                }
            }),
            (Some(_), Some(_)) => op_debug_arms.extend(quote! {
                Op::#variant { a, k, b } => {
                    f.debug_struct(stringify!(#variant)).field("a", a).field("k", k).field("b", b).finish()
                }
            }),
            (None, Some(_)) => unreachable!("gated off at classification"),
        }
    }
    let ops_debug_impl = quote! {
        impl ::core::fmt::Debug for Op {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                match self {
                    #op_debug_arms
                    Op::Custom { named, reserve, .. } => {
                        f.debug_struct("Custom")
                            .field("named", named)
                            .field("reserve", reserve)
                            .finish()
                    }
                }
            }
        }
    };
    let ops_enum = quote! {
        #[doc(hidden)]
        pub enum Op {
            #op_variants
            #custom_variant
        }

        #ops_debug_impl
    };

    // -- staging methods ----------------------------------------------------
    let mut staging = TokenStream2::new();
    for e in &m.entities {
        let snake_name = snake(&e.name.to_string());
        let add_op = format_ident!("add_{}", snake_name);
        let remove_op = format_ident!("remove_{}", snake_name);
        let add_variant = format_ident!("Add{}", e.name);
        let remove_variant = format_ident!("Remove{}", e.name);
        let payload = format_ident!("{}", snake_name);
        if e.host.is_none() {
            let ref_alias = format_ident!("{}Ref", e.name);
            let into_trait = format_ident!("Into{}Arg", e.name);
            let into_fn = format_ident!("into_{}_ref", snake_name);
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
            let arg_use = e.ctor_args.iter().map(|c| &c.arg).collect::<Vec<_>>();
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
        } else {
            // Nested removal stays on `Tx`: it names its target by host ref
            // + old embedded-arena index (the same information as a path,
            // over refs — a path id is a resolved id, exactly what staging
            // must not require). A `Pending` host panics: removal is
            // set-theoretic over old-store records, and a not-yet-flushed
            // host has none.
            let host = e.host.as_ref().unwrap();
            let host_into_trait = format_ident!("Into{}Arg", host);
            let host_into_fn = format_ident!("into_{}_ref", snake(&host.to_string()));
            let doc_remove_nested = format!(
                "Stage the removal of one old `{}` record, by host ref and old \
                 embedded-arena index. A `Pending` host panics here — removal is \
                 set-theoretic over old-store records; staging under a \
                 not-yet-flushed host may only add.",
                e.name
            );
            staging.extend(quote! {
                #[doc = #doc_remove_nested]
                pub fn #remove_op(&mut self, host: impl #host_into_trait, idx: u32) {
                    let host = host.#host_into_fn(self);
                    match host.kind {
                        #rt::tx::RefKind::Existing(_) => {
                            self.ops.push(Op::#remove_variant { host, idx });
                        }
                        #rt::tx::RefKind::Pending(_) => panic!(
                            "remove_{} of a Pending host: removal is set-theoretic \
                             over old-store records; staging under a not-yet-flushed \
                             host may only add",
                            #snake_name
                        ),
                    }
                }
            });
        }
    }

    // -- nested staging scopes (`within` + `SubTx`) -------------------------
    // One scope type per host entity that has embedded arenas (depth 1:
    // the path prefix is the single host REF, captured verbatim — the host
    // may be `Pending`, resolution is deferred to flush). The scope holds
    // NO borrow of the transaction — root and nested staging interleave
    // freely on the same queue — so each nested staging call threads
    // `&mut Tx` explicitly; the captured token is re-checked per call, so
    // a scope from a dead transaction cannot stage either.
    let host_entities: Vec<&Ident2> = {
        let mut v: Vec<&Ident2> = Vec::new();
        for e in &m.entities {
            if let Some(h) = &e.host {
                if !v.iter().any(|x| *x == h) {
                    v.push(h);
                }
            }
        }
        v
    };
    let multi_host = host_entities.len() > 1;
    let mut subtx_defs = TokenStream2::new();
    for h in &host_entities {
        let scope_ty = if multi_host {
            format_ident!("{}SubTx", h)
        } else {
            format_ident!("SubTx")
        };
        let within_fn = if multi_host {
            format_ident!("within_{}", snake(&h.to_string()))
        } else {
            format_ident!("within")
        };
        let host_ref = format_ident!("{}Ref", h);
        let host_into_trait = format_ident!("Into{}Arg", h);
        let host_into_fn = format_ident!("into_{}_ref", snake(&h.to_string()));
        let mut scope_verbs = TokenStream2::new();
        for e in m.entities.iter().filter(|e| e.host.as_ref() == Some(*h)) {
            let snake_name = snake(&e.name.to_string());
            let add_op = format_ident!("add_{}", snake_name);
            let add_variant = format_ident!("Add{}", e.name);
            let arg_defs = e.ctor_args.iter().map(|c| {
                let a = &c.arg;
                let t = &c.target_into_trait;
                quote!(#a: impl #t,)
            });
            let arg_conv = e.ctor_args.iter().map(|c| {
                let a = &c.arg;
                let f = &c.target_into_fn;
                quote!(let #a = #a.#f(tx);)
            });
            let arg_use = e.ctor_args.iter().map(|c| &c.arg).collect::<Vec<_>>();
            let doc_add = format!(
                "Stage a new `{}` record under this scope's host: queues the op \
                 (with the path prefix — the host REF, not a resolved id) on the \
                 threaded transaction. Returns nothing: a nested record's path id \
                 is a resolved id, and staged records resolve only at flush.",
                e.name
            );
            scope_verbs.extend(quote! {
                #[doc = #doc_add]
                pub fn #add_op(&self, tx: &mut Tx<'_>, #(#arg_defs)*) {
                    assert!(
                        self.token == tx.token,
                        "SubTx minted by a different transaction: it stages onto a \
                         queue its path was not recorded against"
                    );
                    #(#arg_conv)*
                    tx.ops.push(Op::#add_variant { host: self.host, #(#arg_use),* });
                }
            });
        }
        let doc_within = format!(
            "Open a nested staging scope under `host`, recording the path prefix \
             — the REF, never a resolved id (the host may be `Pending`; \
             resolution is deferred to flush). The scope holds no borrow of the \
             transaction; thread `&mut Tx` into each nested staging call."
        );
        subtx_defs.extend(quote! {
            #[doc = concat!("A nested staging scope: the path prefix a set of nested ops shares (the host ref, captured verbatim). Holds no borrow of the transaction — the token is re-checked per call.")]
            #[derive(Clone, Debug)]
            pub struct #scope_ty {
                token: #rt::tx::TxToken,
                host: #host_ref,
            }

            impl #scope_ty {
                #scope_verbs
            }
        });
        staging.extend(quote! {
            #[doc = #doc_within]
            pub fn #within_fn(&self, host: impl #host_into_trait) -> #scope_ty {
                let host = host.#host_into_fn(self);
                #scope_ty { token: self.token, host }
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
        if let Some(remove_trait) = &e.remove_trait {
            write_impls.extend(quote! {
                impl<'g> #remove_trait<#marker_path> for Tx<'g> {
                    type #ret_assoc = #ref_alias;
                    fn #remove_method(&mut self, rec: Self::#ret_assoc) {
                        Tx::#remove_method(self, rec)
                    }
                }
            });
        }
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
        if let Some(verb) = &s.verb {
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
    }
    // The hosted keyed verbs' `Insert<Arrow>` impls — a per-record map's
    // self ref makes the canonical arity available, wired exactly like
    // the hosted primitive inserts. An embedded-target verb is inherent-
    // only: the canonical arity takes a target REF, and the probe pins
    // the raw-path-id staging shape (no ref can name an embedded record).
    for kk in &m.keyed {
        if kk.member_host.is_some() {
            continue;
        }
        if let Some(verb) = &kk.verb {
            let trait_path = &verb.trait_path;
            let marker_path = &verb.marker_path;
            let method = &verb.method;
            let inherent = format_ident!("insert_{}", kk.arrow);
            let assoc_decls = verb.assocs.iter().map(|(an, av)| quote!(type #an = #av;));
            let (params, args): (Vec<Ident2>, Vec<Ident2>) = match &kk.host {
                Some(_) => (
                    vec![
                        Ident2::new("a", proc_macro2::Span::call_site()),
                        Ident2::new("k", proc_macro2::Span::call_site()),
                        Ident2::new("b", proc_macro2::Span::call_site()),
                    ],
                    vec![
                        Ident2::new("a", proc_macro2::Span::call_site()),
                        Ident2::new("k", proc_macro2::Span::call_site()),
                        Ident2::new("b", proc_macro2::Span::call_site()),
                    ],
                ),
                None => {
                    let k = Ident2::new("k", proc_macro2::Span::call_site());
                    let b = Ident2::new("b", proc_macro2::Span::call_site());
                    (vec![k.clone(), b.clone()], vec![k, b])
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

    // -- insert_<arrow> staging (keyed arrows, both placements) -----------
    // A keyed arrow is a function: the staging call rejects a live key —
    // against the borrowed old store (a staged removal frees no key;
    // re-pointing one is two flushes) and against the inserts already
    // staged in this transaction. `Existing` and `Pending` keys never
    // collide: the ref equality compares the kind, and an `Existing` ref
    // carries the unstamped sentinel where a `Pending` one carries this
    // transaction's token (the staged-key probe's t3 pin). Store-level
    // maps check the key against the whole map; a per-record map checks
    // it within the NAMED HOST's map only (the duplicate-key probe's
    // two arms, old-map and staged-vs-staged, both host-scoped), and a
    // `Pending` host has no old map at all — the staged arm is its only
    // live-key check. The staged-vs-staged arm compares the host ref too:
    // the same key under a different host is a different slot. An
    // embedded target rides as a raw path id (no conversion — the
    // probe's Existing-only pin). Only store-level arrows answer custom
    // ops' key reservations: a reservation names a key alone, and a
    // per-record slot is (host, key) — custom ops stay root-level, so
    // per-record reservations are out of the acceptance shape.
    let mut keyed_staging = TokenStream2::new();
    for kk in &m.keyed {
        let method = format_ident!("insert_{}", kk.arrow);
        let variant = format_ident!("Insert{}", camel(&kk.arrow.to_string()));
        let arrow_str = kk.arrow.to_string();
        let key_into_trait = format_ident!("Into{}Arg", kk.key_entity);
        let key_into_fn = format_ident!("into_{}_ref", snake(&kk.key_entity.to_string()));
        let key_live = &kk.key_live;
        let key_ref = format_ident!("{}Ref", kk.key_entity);
        let doc = format!(
            "Stage a keyed insert through arrow `{}`: a live key is rejected here — checked against the old store's map and the inserts already staged in this transaction (a staged removal frees no key; re-pointing one is two flushes). The flush carries the map (entries whose key and target both survive, remapped) and replays the op through the backing's insert hook on top of the carry.",
            kk.arrow
        );
        match &kk.host {
            None => {
                let member_into_trait = format_ident!("Into{}Arg", kk.member);
                let member_into_fn =
                    format_ident!("into_{}_ref", snake(&kk.member.to_string()));
                let matches_fn = format_ident!("__{}_key_matches", kk.arrow);
                let reserve_variant = format_ident!("{}", camel(&kk.arrow.to_string()));
                keyed_staging.extend(quote! {
                    #[doc(hidden)]
                    #[doc = concat!("Submission-space key equality for arrow `", stringify!(#arrow_str), "`: an `Existing` ref matches only an `Old` key (and a `Pending` ref only a `Pending` one) — the two kinds never collide.")]
                    fn #matches_fn(k: &#key_ref, key: &__SubmissionKey) -> bool {
                        match (k.kind, key) {
                            (
                                #rt::tx::RefKind::Existing(id),
                                __SubmissionKey::Old(id2),
                            ) => (id).0 == *id2,
                            (
                                #rt::tx::RefKind::Pending(i),
                                __SubmissionKey::Pending(i2),
                            ) => i == *i2,
                            _ => false,
                        }
                    }

                    #[doc = #doc]
                    pub fn #method(&mut self, k: impl #key_into_trait, b: impl #member_into_trait) {
                        let k = k.#key_into_fn(self);
                        let b = b.#member_into_fn(self);
                        if let #rt::tx::RefKind::Existing(id) = k.kind {
                            assert!(
                                !self.old.__store().#key_live(id),
                                "keyed insert onto a live key (`{}`): {:?} exists in the \
                                 borrowed old store; a staged removal frees no key, so \
                                 re-pointing one is two flushes",
                                #arrow_str,
                                id,
                            );
                        }
                        for op in &self.ops {
                            match op {
                                Op::#variant { k: k2, .. } => {
                                    assert!(
                                        k2 != &k,
                                        "keyed insert onto a live key (`{}`): {:?} already \
                                         staged in this transaction",
                                        #arrow_str,
                                        k,
                                    );
                                }
                                Op::Custom { reserve, .. } => {
                                    for r in reserve {
                                        if let ReservedKey::#reserve_variant(key2) = r {
                                            assert!(
                                                !Self::#matches_fn(&k, key2),
                                                "keyed insert onto a live key (`{}`): {:?} is \
                                                 reserved by an already-staged inducing op in \
                                                 this transaction",
                                                #arrow_str,
                                                k,
                                            );
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        self.ops.push(Op::#variant { k, b });
                    }
                });
            }
            Some(host) => {
                let h = ent_of(host);
                let host_into_trait = format_ident!("Into{}Arg", host);
                let host_into_fn = format_ident!("into_{}_ref", snake(&host.to_string()));
                let h_arena = &h.arena;
                let (b_param, b_conv): (TokenStream2, TokenStream2) = match &kk.member_host {
                    None => {
                        let member_into_trait =
                            format_ident!("Into{}Arg", kk.member);
                        let member_into_fn = format_ident!(
                            "into_{}_ref",
                            snake(&kk.member.to_string())
                        );
                        (
                            quote!(b: impl #member_into_trait),
                            quote!(let b = b.#member_into_fn(self);),
                        )
                    }
                    Some(_) => {
                        let b_id = &ent_of(&kk.member).id_ty;
                        (quote!(b: #b_id), TokenStream2::new())
                    }
                };
                keyed_staging.extend(quote! {
                    #[doc = #doc]
                    pub fn #method(
                        &mut self,
                        a: impl #host_into_trait,
                        k: impl #key_into_trait,
                        #b_param,
                    ) {
                        let a = a.#host_into_fn(self);
                        let k = k.#key_into_fn(self);
                        #b_conv
                        // The old-map arm: only an Existing host names an
                        // old record's map, and only an Existing key can
                        // be live there. Range-guarded — an out-of-range
                        // host id is a step-1 UnknownOldId at flush, never
                        // a staging panic; a Pending host has no old map.
                        if let (
                            #rt::tx::RefKind::Existing(hid),
                            #rt::tx::RefKind::Existing(kid),
                        ) = (a.kind, k.kind)
                        {
                            if ((hid).0 as usize) < self.old.__store().#h_arena.len() {
                                assert!(
                                    !self.old.__store().#key_live(hid, kid),
                                    "keyed insert onto a live key (`{}`): {:?} is already \
                                     live in host {:?}'s map; a staged removal frees no key, \
                                     so re-pointing one is two flushes",
                                    #arrow_str,
                                    kid,
                                    hid,
                                );
                            }
                        }
                        for op in &self.ops {
                            if let Op::#variant { a: a2, k: k2, .. } = op {
                                assert!(
                                    !(a2 == &a && k2 == &k),
                                    "keyed insert onto a live key (`{}`): {:?} is already \
                                     claimed by a staged insert in this host",
                                    #arrow_str,
                                    k,
                                );
                            }
                        }
                        self.ops.push(Op::#variant { a, k, b });
                    }
                });
            }
        }
    }

    // -- __stage_custom: the induction escape hatch's staging call -----------
    // A user-written extension method (an `impl Tx<'_>` block in the same
    // crate) stages a custom op: an opaque closure the flush replays
    // against the rebuilt store, the old-store ids the op names (validated
    // in steps 1-2 like any other refs), and the keyed slots it will
    // insert (counted by later keyed staging calls' live-key checks).
    // `__ops` is the readonly queue accessor the extension computes
    // induction keys from at staging time (a staged `AddEdge`'s endpoint
    // refs, for a `Pending` captured ref).
    let custom_staging = quote! {
        #[doc(hidden)]
        #[doc = "Stage a custom op (the induction escape hatch): `run` replays against the rebuilt store during the flush's submission-order replay — user extensions implement inducing constructors this way, computing induced keys from `old()` + `__ops()` at staging and resolving captured refs through the `Resolve` argument at replay. `named` lists the old-store ids the op names; steps 1-2 validate them like any other refs. `reserve` lists the keyed slots the op will insert; a later keyed staging call sees them as live."]
        pub fn __stage_custom(
            &mut self,
            run: ::std::boxed::Box<dyn ::core::ops::FnOnce(&mut #store_build, &Resolve<'_>)>,
            named: ::std::vec::Vec<__NamedRef>,
            reserve: ::std::vec::Vec<ReservedKey>,
        ) {
            self.ops.push(Op::Custom {
                run: ::core::option::Option::Some(run),
                named,
                reserve,
            });
        }

        #[doc(hidden)]
        #[doc = "The staged op queue, readonly — what a custom-op extension computes induction keys from at staging time (e.g. a staged `AddEdge`'s endpoint refs, for a `Pending` captured ref)."]
        pub fn __ops(&self) -> &[Op] {
            &self.ops
        }
    };

    // -- the induction escape hatch (step 10) --------------------------------
    // User-staged custom ops: `Op::Custom` (above) plus the types it carries
    // and the doc-hidden staging accessors a user extension impls against.
    // `__NamedRef` names the old-store ids a custom op captures (step 1
    // range-checks, step 2 rejects removed ones at the op); root-level
    // entities only — a custom op replays at root level, where an embedded
    // record's path id has no meaning. `ReservedKey` names the keyed slot
    // an already-staged inducing op will insert, per store-level keyed
    // arrow; `__SubmissionKey` is a key in submission space (`Old` id |
    // `Pending` op index) — `Existing` and `Pending` never collide (the
    // staged-key probe's t3 pin), so the comparison is kind-aware.
    let named_ref_variants = root_entities.iter().map(|e| {
        let v = &e.name;
        quote!(#v(u32),)
    });
    let named_ref_enum = quote! {
        #[doc(hidden)]
        #[doc = "An old-store id named by a staged custom op (the induction escape hatch). Steps 1-2 validate the named ids like any other `Existing` ref: a range violation is `UnknownOldId`, a removed one a `DanglingEndpoint` attributed to the custom op (the naming-op rule). Root-level entities only — a custom op replays at root level."]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum __NamedRef {
            #(#named_ref_variants)*
        }
    };
    // Reservations name a key alone; a per-record slot is (host, key), and
    // custom ops are root-level — only store-level keyed arrows get a
    // ReservedKey variant (per-record reservations are out of the
    // acceptance shape; noted in the step-11 notes).
    let reserved_key_variants = m
        .keyed
        .iter()
        .filter(|kk| kk.host.is_none())
        .map(|kk| {
            let v = format_ident!("{}", camel(&kk.arrow.to_string()));
            quote!(#v(__SubmissionKey),)
        });
    let reserved_key_enum = quote! {
        #[doc(hidden)]
        #[doc = "A keyed slot an already-staged inducing (custom) op will insert, per store-level keyed arrow. The keyed staging verb's live-key check counts these — an insert keyed after a staged induction is a live-key staging panic, exactly as if the keyed insert op itself were already staged (the induct-flush probe's induced-key staging check)."]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum ReservedKey {
            #(#reserved_key_variants)*
        }
    };
    let submission_key_enum = quote! {
        #[doc(hidden)]
        #[doc = "A key in submission space: an old-store id, or the op index of a record this change set will create. `Old` and `Pending` never collide (a `Pending` ref carries this transaction's token where an `Existing` one carries the unstamped sentinel)."]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum __SubmissionKey {
            Old(u32),
            Pending(u32),
        }
    };

    // -- flush: per-entity derived names -----------------------------------
    struct Ent {
        name: Ident2,
        snake: Ident2,
        arena: Ident2,
        data: Ident2,
        id_ty: Ident2,
        arena_str: String,
        /// Embedding host's index in `ents` (a nested-store entity).
        host: Option<usize>,
        count: Ident2,   // old_<snake>_count
        map: Ident2,     // <snake>_map
        removed: Ident2, // removed_<snake>
        removed_by: Ident2,
        pending: Ident2, // pending_<snake>
    }
    let mut ents = ents;
    for (i, e) in m.entities.iter().enumerate() {
        if let Some(h) = &e.host {
            let hi = ents.iter().position(|x| &x.name == h).expect("host entity");
            ents[i].host = Some(hi);
        }
    }
    let ents = ents;
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
            hook: s.hook.clone(),
        })
        .collect();

    // The nested-store surface (emitted only when the store has embedded
    // arenas): path-coherence and per-host nested-index violations, and the
    // kept-record descriptor that names a sub record by (host, idx).
    let has_nested = ents.iter().any(|x| x.host.is_some());
    let nested_desc_variant = has_nested.then(|| {
        quote!(
            #[doc = "An old-store record in an embedded arena, kept from the previous store — named by its arena, its host's old id, and its old embedded-arena index."]
            KeptNested { arena: &'static str, host: u32, idx: u32 },
        )
    });
    let nested_violation_variants = has_nested.then(|| {
        quote!(
            #[doc = "A nested removal named an embedded-arena index outside the named host's old arena."]
            UnknownNestedIdx { op: usize, arena: &'static str, host: u32, idx: u32 },
            #[doc = "The same old embedded-arena index under the same host was removed twice."]
            DoubleNestedRemoval { op: usize, arena: &'static str, host: u32, idx: u32 },
            #[doc = "Ops staged under a nested path whose host the change set removes. The change set must be coherent; a silent discard would hide staging bugs. `nested_ops` is ascending (staged adds AND removals under the host both qualify — a nested removal targets a record the change set is already deleting); `removal_op` names the host's removal op. Attribution: each removed host is claimed by its FIRST removal op; hosts are reported in removal-op submission order."]
            RemovedPathHost { arena: &'static str, host: u32, nested_ops: ::std::vec::Vec<usize>, removal_op: usize },
        )
    });
    // The keyed-carry violations (step 11). Key-as-reference: removing a
    // key still named from a surviving place is a flush error, never a
    // silent entry drop. "Live" means SURVIVING THE CHANGE SET at every
    // reference site (the keyed-carry probe's pin): kept-owner entries
    // whose target survives, and this change set's own staged inserts
    // keyed by the removed key — a staged insert names the key from a
    // new place, so it counts as live (and is attributed to the REMOVAL
    // op, not the insert). Doomed owners do not count (the map dies with
    // its owner — the cascade exception), nor do entries whose target
    // the change set removes (target-as-membership clearing applies
    // first). Entries are sorted by (owner, key) — never HashMap
    // iteration order. The owner column names the map-owning record's
    // old id; a store-level map reports 0 (the store owns its entries).
    let has_keyed = !m.keyed.is_empty();
    let keyed_violation_variants = has_keyed.then(|| {
        quote!(
            #[doc = "Key-as-reference (keyed carry): removing a key that still has live entries — kept-owner entries whose target survives, or this change set's own staged inserts keyed by it (they name the key from a new place, so they count as live). `entries` is sorted by (owner, key); the owner column is the map-owning record's old id (0 for a store-level map). Attributed to the removal op."]
            KeyRemovedWithLiveEntries { op: usize, arena: &'static str, key: u32, entries: ::std::vec::Vec<(u32, u32)>, staged_inserts: ::std::vec::Vec<usize> },
        )
    });
    // A per-record keyed map whose target is embedded: a staged insert's
    // value-host agreement and target liveness are checked at step 2 in
    // old-id space (the probe's pin: construction panics on mismatch, but
    // a staged insert is plan-checked and reports alongside RemovedPathHost).
    let has_path_keyed = m.keyed.iter().any(|kk| kk.member_host.is_some());
    let host_name_enum = has_path_keyed.then(|| {
        quote!(
            #[doc = "How a staged keyed insert's host was named before resolution (path-id targets only): an old owner id, or the op index of a record this change set will create."]
            #[derive(Clone, Copy, PartialEq, Eq, Debug)]
            pub enum HostName {
                Old(u32),
                Pending(u32),
            }
        )
    });
    let path_keyed_violation_variants = has_path_keyed.then(|| {
        quote!(
            #[doc = "A staged keyed insert through a path-id target whose host component cannot equal the entry's host after remap: the target names a different old owner, or the entry's host is Pending (a fresh id — an Existing path-id target always names an old owner, and remap is identity-preserving, so agreement is unreachable). Attributed to the insert op; checked at step 2 in old-id space."]
            InsertHostTargetMismatch { op: usize, entry_host: HostName, target_host: u32, target_idx: u32 },
            #[doc = "A staged keyed insert through a path-id target the change set removes. Attributed to the insert op."]
            DanglingInsertTarget { op: usize, target_host: u32, target_idx: u32 },
        )
    });

    // The custom-op replay context (the induction escape hatch): the
    // compaction maps and replay tables of every ROOT-LEVEL entity, so a
    // staged closure can resolve its captured refs against the rebuilt
    // store. Root-level only — a custom op replays at root level, and an
    // embedded record's path id has no meaning there. No old-store
    // reference: by replay the owned path has consumed the old store (the
    // closure reads the rebuilt store — carried field values are the old
    // ones remapped; staging-time reads go through `Tx::old`). The pending
    // lookup is bounds-checked with the internal-bug panic (decision 5's
    // shape: a `Pending` ref resolves only through a same-queue add op
    // preceding its use).
    let root_ents: Vec<&Ent> = ents.iter().filter(|x| x.host.is_none()).collect();
    let resolve_fields = root_ents.iter().map(|x| {
        let (map, pending, id_ty) = (&x.map, &x.pending, &x.id_ty);
        quote!(
            #map: &'a [::core::option::Option<u32>],
            #pending: &'a [::core::option::Option<#id_ty>],
        )
    });
    let resolve_methods = root_ents.iter().map(|x| {
        let (map, pending, id_ty) = (&x.map, &x.pending, &x.id_ty);
        let method = format_ident!("resolve_{}", x.snake);
        let add_name = format_ident!("add_{}", x.snake);
        let ref_alias = format_ident!("{}Ref", x.name);
        quote! {
            #[doc = concat!("Resolve a `", stringify!(#ref_alias), "` captured at staging: `Existing` through the compaction map (validated in steps 1-2), `Pending` through this change set's replay table.")]
            pub fn #method(&self, r: #ref_alias) -> #id_ty {
                match r.kind {
                    #rt::tx::RefKind::Existing(id) =>
                        #id_ty(self.#map[(id).0 as usize]
                            .expect("Existing id validated in steps 1-2")),
                    #rt::tx::RefKind::Pending(i) =>
                        match self.#pending.get(i as usize).copied().flatten() {
                            ::core::option::Option::Some(id) => id,
                            ::core::option::Option::None => panic!(
                                "flush: replay table miss for Pending {} op {}: internal bug — \
                                 Pending {} refs are minted only by {} preceding its use \
                                 in submission order",
                                stringify!(#id_ty),
                                i,
                                stringify!(#id_ty),
                                stringify!(#add_name),
                            ),
                        },
                }
            }
        }
    });
    let resolve_struct = quote! {
        #[doc(hidden)]
        #[doc = concat!("The custom op's replay context over `", stringify!(#store), "`: the compaction maps and replay tables of every root-level entity. A staged custom op's closure resolves its captured refs through this; the store argument is the rebuilt skeleton mid-replay. Constructed by `flush` — never by user code.")]
        pub struct Resolve<'a> {
            #(#resolve_fields)*
        }

        impl Resolve<'_> {
            #(#resolve_methods)*
        }
    };

    // -- flush: step 1, custom ops -------------------------------------------
    // The old-store ids a custom op names range-validate exactly like any
    // other `Existing` ref (a flagged id is never indexed downstream).
    let s1_custom_arm = {
        let checks = root_ents.iter().map(|x| {
            let v = &x.name;
            let (c, s) = (&x.count, &x.arena_str);
            quote! {
                __NamedRef::#v(id) => {
                    if *id as usize >= #c {
                        violations.push(Violation::UnknownOldId {
                            op: op_index,
                            arena: #s,
                            id: *id,
                        });
                    }
                }
            }
        });
        quote! {
            Op::Custom { named, .. } => {
                for n in named {
                    match n {
                        #(#checks)*
                    }
                }
            }
        }
    };

    // -- flush: step 2, custom ops -------------------------------------------
    // The naming-op rule for a custom op: an old-store id it names that the
    // change set removes dangles at the CUSTOM op (like a staged add naming
    // a removed endpoint), not at the removal. Range-guarded: a flagged id
    // is never indexed.
    let s2_custom_arm = {
        let checks = root_ents.iter().map(|x| {
            let v = &x.name;
            let (c, removed, s) = (&x.count, &x.removed, &x.arena_str);
            quote! {
                __NamedRef::#v(id) => {
                    if (*id as usize) < #c && #removed[*id as usize] {
                        violations.push(Violation::DanglingEndpoint {
                            op: op_index,
                            record: RecordDesc::Staged(op_index as u32),
                            endpoint_arena: #s,
                            endpoint: *id,
                        });
                    }
                }
            }
        });
        quote! {
            if let Op::Custom { named, .. } = op {
                for n in named {
                    match n {
                        #(#checks)*
                    }
                }
            }
        }
    };

    // -- flush: the error/report surface ------------------------------------
    let report_fields = ents.iter().map(|x| {
        let f = &x.map;
        if x.host.is_some() {
            quote!(#[doc = concat!("Old->new index maps of `", stringify!(#f), "'s embedded arenas: indexed by OLD host id, then old embedded-arena index; `None` where removed. A removed host's map is all-`None` (the arena dies with its host). Staged records append per host in staged order.")] pub #f: ::std::vec::Vec<::std::vec::Vec<::core::option::Option<u32>>>,)
        } else {
            quote!(#[doc = concat!("Old->new id map of `", stringify!(#f), "'s arena: old-id-indexed, `None` where removed.")] pub #f: ::std::vec::Vec<::core::option::Option<u32>>,)
        }
    });
    let report_init = ents.iter().map(|x| {
        let f = &x.map;
        quote!(#f: #f,)
    });
    // The analysis struct's per-entity members, in the pipeline's local
    // names, so the destructure in `flush` hands steps 3-7 the same
    // bindings the inlined pipeline used to declare. An embedded entity
    // carries ONE per-host removal set (indexed by old host id, then old
    // embedded-arena index) — no global count or kept set: its arenas are
    // per-record.
    let analysis_struct_fields = ents.iter().flat_map(|x| {
        let (c, removed, removed_by) = (&x.count, &x.removed, &x.removed_by);
        if x.host.is_some() {
            vec![quote!(#removed_by: ::std::vec::Vec<::std::vec::Vec<::core::option::Option<usize>>>,)]
        } else {
            vec![
                quote!(#c: usize,),
                quote!(#removed: ::std::vec::Vec<bool>,),
                quote!(#removed_by: ::std::vec::Vec<::core::option::Option<usize>>,),
            ]
        }
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
        .flat_map(|x| {
            if x.host.is_some() {
                vec![x.removed_by.clone()]
            } else {
                vec![x.count.clone(), x.removed.clone(), x.removed_by.clone()]
            }
        })
        .chain(sets.iter().map(|s| s.eff.clone()))
        .collect();
    let analysis_fields = analysis_members.iter().map(|m| quote!(#m,));
    let analysis_names = analysis_members.iter().map(|m| quote!(#m,));
    let flush_types = quote! {
        #[doc = "How a record was named in a [`Violation::DanglingEndpoint`]: an old-store record kept from the previous store (its arena name and old id), or a record staged in this change set (its `add` op's index)."]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum RecordDesc {
            Kept { arena: &'static str, id: u32 },
            #nested_desc_variant
            Staged(u32),
        }

        #host_name_enum

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
            #nested_violation_variants
            #keyed_violation_variants
            #path_keyed_violation_variants
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
            pub store: #store_frozen,
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
    let s1_counts = ents.iter().filter(|x| x.host.is_none()).map(|x| {
        let c = &x.count;
        let a = &x.arena;
        quote!(let #c = old.#a.len();)
    });
    let s1_state = ents.iter().map(|x| {
        let (removed, removed_by, c) = (&x.removed, &x.removed_by, &x.count);
        match x.host {
            None => quote!(
                let mut #removed = ::std::vec![false; #c];
                let mut #removed_by: ::std::vec::Vec<::core::option::Option<usize>> =
                    ::std::vec![::core::option::Option::None; #c];
            ),
            // Per old host, one removal set for its embedded arena (kept
            // sets are strictly per arena — a host removal does not edit
            // them; the cascade is applied at map time and record-copy
            // time, and both places agree).
            Some(hi) => {
                let h = &ents[hi];
                let (h_arena, e_arena) = (&h.arena, &x.arena);
                quote!(
                    let mut #removed_by: ::std::vec::Vec<::std::vec::Vec<::core::option::Option<usize>>> =
                        old.#h_arena
                            .iter()
                            .map(|hrec| ::std::vec![::core::option::Option::None; hrec.#e_arena.len()])
                            .collect();
                )
            }
        }
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
        // A nested add's host segment range-validates like any other
        // `Existing` ref (against the HOST arena's old count).
        let host_check = e.host.as_ref().map(|h| {
            let hx = ent_of(h);
            let (h_count, h_str) = (&hx.count, &hx.arena_str);
            quote! {
                if let #rt::tx::RefKind::Existing(id) = host.kind {
                    if (id).0 as usize >= #h_count {
                        violations.push(Violation::UnknownOldId {
                            op: op_index,
                            arena: #h_str,
                            id: (id).0,
                        });
                    }
                }
            }
        });
        let host_pat = e.host.as_ref().map(|_| quote!(host,));
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
            Op::#variant { #host_pat #(#arg_names),* } => {
                #host_check
                #(#checks)*
            }
        }
    });
    let s1_remove_arms = m.entities.iter().map(|e| {
        let variant = format_ident!("Remove{}", e.name);
        let x = ent_of(&e.name);
        let payload = &x.snake;
        let (removed, removed_by, arena_str) = (&x.removed, &x.removed_by, &x.arena_str);
        match x.host {
            None => quote! {
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
            },
            // A nested removal: the host range-validates against its own
            // arena (a flagged host must never index the per-host sets —
            // the spike7-oob rule), then the index against that host's old
            // embedded arena.
            Some(hi) => {
                let h = &ents[hi];
                let (h_count, h_str, h_arena) = (&h.count, &h.arena_str, &h.arena);
                let e_arena = &x.arena;
                quote! {
                    Op::#variant { host, idx } => {
                        if let #rt::tx::RefKind::Existing(hid) = host.kind {
                            let hidx = (hid).0 as usize;
                            if hidx >= #h_count {
                                violations.push(Violation::UnknownOldId {
                                    op: op_index,
                                    arena: #h_str,
                                    id: (hid).0,
                                });
                            } else if (*idx as usize) >= old.#h_arena[hidx].#e_arena.len() {
                                violations.push(Violation::UnknownNestedIdx {
                                    op: op_index,
                                    arena: #arena_str,
                                    host: (hid).0,
                                    idx: *idx,
                                });
                            } else {
                                match #removed_by[hidx][*idx as usize] {
                                    ::core::option::Option::Some(_) => {
                                        violations.push(Violation::DoubleNestedRemoval {
                                            op: op_index,
                                            arena: #arena_str,
                                            host: (hid).0,
                                            idx: *idx,
                                        });
                                    }
                                    ::core::option::Option::None => {
                                        #removed_by[hidx][*idx as usize] =
                                            ::core::option::Option::Some(op_index);
                                    }
                                }
                            }
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

    // Step 1, keyed insert ops — the key and target refs range-validate
    // like every other `Existing` ref (an unvalidated id would be indexed
    // at replay — a panic on user error, which the pipeline's contract
    // forbids). A per-record map also range-validates the NAMED HOST;
    // an embedded target is a raw path id whose host component
    // range-validates as an old host id and whose index range-validates
    // against that old host's embedded arena (UnknownNestedIdx — step 9's
    // shape).
    let s1_keyed_arms = m.keyed.iter().map(|kk| {
        let variant = format_ident!("Insert{}", camel(&kk.arrow.to_string()));
        let k = ent_of(&kk.key_entity);
        let k_check = {
            let (k_count, k_str) = (&k.count, &k.arena_str);
            quote! {
                if let #rt::tx::RefKind::Existing(id) = k.kind {
                    if (id).0 as usize >= #k_count {
                        violations.push(Violation::UnknownOldId {
                            op: op_index,
                            arena: #k_str,
                            id: (id).0,
                        });
                    }
                }
            }
        };
        match (&kk.host, &kk.member_host) {
            (None, None) => {
                let b = ent_of(&kk.member);
                let (b_count, b_str) = (&b.count, &b.arena_str);
                quote! {
                    Op::#variant { k, b } => {
                        #k_check
                        if let #rt::tx::RefKind::Existing(id) = b.kind {
                            if (id).0 as usize >= #b_count {
                                violations.push(Violation::UnknownOldId {
                                    op: op_index,
                                    arena: #b_str,
                                    id: (id).0,
                                });
                            }
                        }
                    }
                }
            }
            (Some(host), None) => {
                let h = ent_of(host);
                let b = ent_of(&kk.member);
                let (h_count, h_str) = (&h.count, &h.arena_str);
                let (b_count, b_str) = (&b.count, &b.arena_str);
                quote! {
                    Op::#variant { a, k, b } => {
                        if let #rt::tx::RefKind::Existing(id) = a.kind {
                            if (id).0 as usize >= #h_count {
                                violations.push(Violation::UnknownOldId {
                                    op: op_index,
                                    arena: #h_str,
                                    id: (id).0,
                                });
                            }
                        }
                        #k_check
                        if let #rt::tx::RefKind::Existing(id) = b.kind {
                            if (id).0 as usize >= #b_count {
                                violations.push(Violation::UnknownOldId {
                                    op: op_index,
                                    arena: #b_str,
                                    id: (id).0,
                                });
                            }
                        }
                    }
                }
            }
            (Some(host), Some(mhost)) => {
                let h = ent_of(host);
                let mh = ent_of(mhost);
                let m = ent_of(&kk.member);
                let (h_count, h_str) = (&h.count, &h.arena_str);
                let (mh_count, mh_str) = (&mh.count, &mh.arena_str);
                let (m_arena, m_str) = (&m.arena, &m.arena_str);
                let mh_arena = &mh.arena;
                quote! {
                    Op::#variant { a, k, b } => {
                        if let #rt::tx::RefKind::Existing(id) = a.kind {
                            if (id).0 as usize >= #h_count {
                                violations.push(Violation::UnknownOldId {
                                    op: op_index,
                                    arena: #h_str,
                                    id: (id).0,
                                });
                            }
                        }
                        #k_check
                        if ((b.0).0 as usize) >= #mh_count {
                            violations.push(Violation::UnknownOldId {
                                op: op_index,
                                arena: #mh_str,
                                id: (b.0).0,
                            });
                        } else if (b.1 as usize)
                            >= old.#mh_arena[(b.0).0 as usize].#m_arena.len()
                        {
                            violations.push(Violation::UnknownNestedIdx {
                                op: op_index,
                                arena: #m_str,
                                host: (b.0).0,
                                idx: b.1,
                            });
                        }
                    }
                }
            }
            (None, Some(_)) => unreachable!("gated off at classification"),
        }
    });

    // Step 2, pass A — kept-record danglers over every ctor-arg field (a
    // field-backed `one` arrow): a kept record whose field target was
    // removed dangles, attributed to the removal op — unless a staged
    // `set_` re-pointed the field: the HONOR reading judges the record at
    // its EFFECTIVE endpoint (the set op's endpoint was checked at the op,
    // attributed to the naming op), so a re-pointed field skips the
    // old-target check entirely. Embedded records are walked per kept host
    // (a removed host's contents die with it — the cascade — so only ops
    // STAGED under it are errors, see RemovedPathHost) and named by
    // (arena, host, idx); embedded fields have no set verb this step, so
    // they check the old target unconditionally.
    let s2_kept = m.entities.iter().filter(|e| !e.ctor_args.is_empty()).map(|e| {
        let x = ent_of(&e.name);
        let (arena_str, e_arena) = (&x.arena_str, &x.arena);
        // The record descriptor and loop shape differ by placement; the
        // per-field check body is shared.
        let (record_desc, honor_index): (TokenStream2, bool) = match x.host {
            None => (
                quote!(RecordDesc::Kept { arena: #arena_str, id: old_id as u32 }),
                true,
            ),
            Some(_) => (
                quote!(RecordDesc::KeptNested {
                    arena: #arena_str,
                    host: old_host as u32,
                    idx: old_idx as u32,
                }),
                false,
            ),
        };
        let checks = e.ctor_args.iter().map(|ca| {
            let t = ent_of(&ca.target_entity);
            let (t_removed, t_removed_by, t_str) = (&t.removed, &t.removed_by, &t.arena_str);
            let f = &ca.arg; // the field name; the honor table is basis-named
            let has_set = honor_index
                && m.sets
                    .iter()
                    .any(|s| s.self_entity == e.name && s.arrow == ca.basis);
            let honor_guard = has_set.then(|| {
                let eff = format_ident!("eff_{}", ca.basis);
                quote!(if #eff[old_id].is_none())
            });
            quote!({
                #honor_guard {
                    let v = rec.#f;
                    if #t_removed[(v).0 as usize] {
                        violations.push(Violation::DanglingEndpoint {
                            op: #t_removed_by[(v).0 as usize].unwrap(),
                            record: #record_desc,
                            endpoint_arena: #t_str,
                            endpoint: (v).0,
                        });
                    }
                }
            })
        });
        match x.host {
            None => {
                let (c, removed, arena) = (&x.count, &x.removed, &x.arena);
                quote! {
                    for old_id in 0..#c {
                        if #removed[old_id] {
                            continue;
                        }
                        let rec = &old.#arena[old_id];
                        #(#checks)*
                    }
                }
            }
            Some(hi) => {
                let h = &ents[hi];
                let (h_count, h_removed, h_arena) = (&h.count, &h.removed, &h.arena);
                let e_removed_by = &x.removed_by;
                quote! {
                    for old_host in 0..#h_count {
                        if #h_removed[old_host] {
                            continue; // the cascade: contents die with the host
                        }
                        for old_idx in 0..old.#h_arena[old_host].#e_arena.len() {
                            if #e_removed_by[old_host][old_idx].is_some() {
                                continue;
                            }
                            let rec = &old.#h_arena[old_host].#e_arena[old_idx];
                            #(#checks)*
                        }
                    }
                }
            }
        }
    });

    // Step 2, path coherence (nested stores) — an op staged under a
    // REMOVED `Existing` host names a record the change set itself is
    // deleting: report it, never silently discard. Per host removal (first
    // removal wins — a second removal of the same id was already flagged
    // as DoubleRemoval), every nested add AND nested removal staged under
    // it is collected ascending (a nested removal targets a record already
    // going away — the plan's "remove of a removed element" in sub-level
    // clothing). Range-guarded: a flagged host id must never reach an
    // indexed read. Runs before compaction so the report names old ids.
    let s2_removed_path_hosts = host_entities.iter().map(|h| {
        let hx = ent_of(h);
        let (h_count, h_removed_by, h_str) = (&hx.count, &hx.removed_by, &hx.arena_str);
        let remove_variant = format_ident!("Remove{}", h);
        let payload = &hx.snake;
        let child_matches = m
            .entities
            .iter()
            .filter(|e| e.host.as_ref() == Some(*h))
            .map(|e| {
                let add_variant = format_ident!("Add{}", e.name);
                let remove_child = format_ident!("Remove{}", e.name);
                quote! {
                    Op::#add_variant { host, .. } | Op::#remove_child { host, .. } => {
                        ::core::matches!(host.kind, #rt::tx::RefKind::Existing(hh) if hh == hid)
                    }
                }
            })
            .chain(m.keyed.iter().filter(|kk| kk.host.as_ref() == Some(*h)).map(|kk| {
                // A keyed insert under a removed owner names a map that
                // dies with the owner — staged-under coherence covers it
                // (the probe's InsertEntry in the nested_ops list).
                let insert_variant = format_ident!("Insert{}", camel(&kk.arrow.to_string()));
                quote! {
                    Op::#insert_variant { a, .. } => {
                        ::core::matches!(a.kind, #rt::tx::RefKind::Existing(hh) if hh == hid)
                    }
                }
            }));
        quote! {
            for (removal_op, op) in ops.iter().enumerate() {
                if let Op::#remove_variant { #payload } = op {
                    if let #rt::tx::RefKind::Existing(hid) = #payload.kind {
                        let hidx = (hid).0 as usize;
                        if hidx < #h_count && #h_removed_by[hidx] == ::core::option::Option::Some(removal_op) {
                            let nested_ops: ::std::vec::Vec<usize> = ops
                                .iter()
                                .enumerate()
                                .filter(|(_, other)| match other {
                                    #(#child_matches)*
                                    _ => false,
                                })
                                .map(|(i, _)| i)
                                .collect();
                            if !nested_ops.is_empty() {
                                violations.push(Violation::RemovedPathHost {
                                    arena: #h_str,
                                    host: (hid).0,
                                    nested_ops,
                                    removal_op,
                                });
                            }
                        }
                    }
                }
            }
        }
    });

    // Step 2, key-as-reference — for each removed key, the surviving
    // entries naming it and this change set's own staged inserts keyed
    // by it. "Live" means SURVIVING THE CHANGE SET (the keyed-carry
    // probe's pin): a kept owner's entry whose target survives counts; a
    // doomed owner's entry does not (the map dies with its owner); an
    // entry whose target is removed does not (target-as-membership
    // clearing); a removed note-style referrer does not (the kept-record
    // pass skips removed records). A staged insert keyed by the removed
    // key DOES count — the change set names the key from a new place —
    // and is attributed to the REMOVAL op, not the insert (one violation
    // per problem: inserts already reported under RemovedPathHost are
    // excluded from staged_inserts). Entries are collected and sorted by
    // (owner, key) — never HashMap iteration order (the determinism pin).
    let s2_key_removed = m.keyed.iter().map(|kk| {
        let variant = format_ident!("Insert{}", camel(&kk.arrow.to_string()));
        let k = ent_of(&kk.key_entity);
        let (k_count, k_removed_by, k_str, k_id) = (&k.count, &k.removed_by, &k.arena_str, &k.id_ty);
        let remove_variant = format_ident!("Remove{}", kk.key_entity);
        let payload = format_ident!("{}", snake(&kk.key_entity.to_string()));
        let mf = &kk.map_field;
        let entry_loop = match (&kk.host, &kk.member_host) {
            (None, None) => {
                let b = ent_of(&kk.member);
                let b_removed = &b.removed;
                quote! {
                    let mut entries: ::std::vec::Vec<(u32, u32)> = ::std::vec::Vec::new();
                    for (key, val) in old.#mf.iter() {
                        if (key).0 == (id).0 && !#b_removed[(val).0 as usize] {
                            entries.push((0, (id).0));
                        }
                    }
                }
            }
            (Some(host), None) => {
                let h = ent_of(host);
                let b = ent_of(&kk.member);
                let (h_count, h_removed, h_arena) = (&h.count, &h.removed, &h.arena);
                let b_removed = &b.removed;
                quote! {
                    let mut entries: ::std::vec::Vec<(u32, u32)> = ::std::vec::Vec::new();
                    for old_owner in 0..#h_count {
                        if #h_removed[old_owner] {
                            continue; // doomed owner: the map dies with it
                        }
                        if let ::core::option::Option::Some(val) =
                            old.#h_arena[old_owner].#mf.get(&#k_id((id).0))
                        {
                            if !#b_removed[(val).0 as usize] {
                                entries.push((old_owner as u32, (id).0));
                            }
                        }
                    }
                }
            }
            (Some(host), Some(_)) => {
                let h = ent_of(host);
                let m = ent_of(&kk.member);
                let (h_count, h_removed, h_arena) = (&h.count, &h.removed, &h.arena);
                let m_removed_by = &m.removed_by;
                quote! {
                    let mut entries: ::std::vec::Vec<(u32, u32)> = ::std::vec::Vec::new();
                    for old_owner in 0..#h_count {
                        if #h_removed[old_owner] {
                            continue; // doomed owner: the map dies with it
                        }
                        if let ::core::option::Option::Some(val) =
                            old.#h_arena[old_owner].#mf.get(&#k_id((id).0))
                        {
                            // Value-host agreement (construction's
                            // invariant, checked by validate) puts the
                            // target in the owner's own embedded arena.
                            if #m_removed_by[old_owner][(val).1 as usize].is_none() {
                                entries.push((old_owner as u32, (id).0));
                            }
                        }
                    }
                }
            }
            (None, Some(_)) => unreachable!("gated off at classification"),
        };
        // One violation per problem: inserts already reported under
        // RemovedPathHost (a doomed entry owner) are not live references
        // — their map dies with the owner. The exclusion rides only on
        // nested stores (the variant does not exist elsewhere, and a
        // non-nested store can never produce it).
        let staged_scan = if has_nested {
            quote! {
                let staged_inserts: ::std::vec::Vec<usize> = ops
                    .iter()
                    .enumerate()
                    .filter(|(i, other)| {
                        let is_keyed = ::core::matches!(
                            other,
                            Op::#variant { k: k2, .. }
                                if ::core::matches!(k2.kind,
                                    #rt::tx::RefKind::Existing(k3) if k3 == id)
                        );
                        if !is_keyed {
                            return false;
                        }
                        !violations.iter().any(|v| {
                            ::core::matches!(v, Violation::RemovedPathHost { nested_ops, .. }
                                if nested_ops.contains(i))
                        })
                    })
                    .map(|(i, _)| i)
                    .collect();
            }
        } else {
            quote! {
                let staged_inserts: ::std::vec::Vec<usize> = ops
                    .iter()
                    .enumerate()
                    .filter(|(_, other)| {
                        ::core::matches!(
                            other,
                            Op::#variant { k: k2, .. }
                                if ::core::matches!(k2.kind,
                                    #rt::tx::RefKind::Existing(k3) if k3 == id)
                        )
                    })
                    .map(|(i, _)| i)
                    .collect();
            }
        };
        quote! {
            for (op_index, op) in ops.iter().enumerate() {
                if let Op::#remove_variant { #payload } = op {
                    if let #rt::tx::RefKind::Existing(id) = #payload.kind {
                        if ((id).0 as usize) < #k_count
                            && #k_removed_by[(id).0 as usize] == ::core::option::Option::Some(op_index)
                        {
                            #entry_loop
                            entries.sort();
                            #staged_scan
                            if !entries.is_empty() || !staged_inserts.is_empty() {
                                violations.push(Violation::KeyRemovedWithLiveEntries {
                                    op: op_index,
                                    arena: #k_str,
                                    key: (id).0,
                                    entries,
                                    staged_inserts,
                                });
                            }
                        }
                    }
                }
            }
        }
    });

    // Step 2, staged keyed inserts through a PATH-ID target — value-host
    // agreement and target liveness in old-id space (the keyed-carry
    // probe's 2c). A Pending entry owner is agreement-unreachable by
    // construction (an Existing path id names an old owner; remap is
    // identity-preserving; the Pending owner gets a fresh id) — the
    // probe's InsertHostTargetMismatch, not a staging panic. A doomed
    // entry owner is RemovedPathHost's report. Range-guarded: a flagged
    // component is never indexed.
    let s2_path_keyed = m
        .keyed
        .iter()
        .filter(|kk| kk.member_host.is_some())
        .map(|kk| {
            let variant = format_ident!("Insert{}", camel(&kk.arrow.to_string()));
            let host = kk.host.as_ref().expect("checked: a path-id target implies a host");
            let mhost = kk.member_host.as_ref().expect("checked: filtered");
            let h = ent_of(host);
            let mh = ent_of(mhost);
            let m = ent_of(&kk.member);
            let (h_count, h_removed) = (&h.count, &h.removed);
            let m_removed_by = &m.removed_by;
            let (mh_arena, m_arena) = (&mh.arena, &m.arena);
            quote! {
                for (op_index, op) in ops.iter().enumerate() {
                    if let Op::#variant { a, k: _, b } = op {
                        match a.kind {
                            #rt::tx::RefKind::Pending(i) => {
                                violations.push(Violation::InsertHostTargetMismatch {
                                    op: op_index,
                                    entry_host: HostName::Pending(i),
                                    target_host: (b.0).0,
                                    target_idx: b.1,
                                });
                            }
                            #rt::tx::RefKind::Existing(hid) => {
                                let hidx = (hid).0 as usize;
                                if hidx >= #h_count || #h_removed[hidx] {
                                    continue; // step 1 or RemovedPathHost owns this
                                }
                                if (b.0).0 != (hid).0 {
                                    violations.push(Violation::InsertHostTargetMismatch {
                                        op: op_index,
                                        entry_host: HostName::Old((hid).0),
                                        target_host: (b.0).0,
                                        target_idx: b.1,
                                    });
                                } else if (b.1 as usize)
                                    >= old.#mh_arena[hidx].#m_arena.len()
                                {
                                    // step 1's UnknownNestedIdx already owns
                                    // this; the guard only protects the
                                    // indexed read below.
                                } else if #m_removed_by[hidx][b.1 as usize].is_some() {
                                    violations.push(Violation::DanglingInsertTarget {
                                        op: op_index,
                                        target_host: (b.0).0,
                                        target_idx: b.1,
                                    });
                                }
                            }
                        }
                    }
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
            if let Op::#variant { #(#arg_names,)* .. } = op {
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
    // Step 2, keyed insert ops — a staged insert naming a removed TARGET
    // dangles at the insert op (a removed record cannot accept
    // membership), exactly like the primitive insert arms. The removed-KEY
    // case is NOT a dangler: a staged insert keyed by a removed key names
    // the key from a new place — it counts as a LIVE reference and rides
    // the removal op's `KeyRemovedWithLiveEntries` (the survivors-only
    // pin). A path-id target's liveness and value-host agreement are the
    // hosted path pass below. Range-guarded: a flagged id is never
    // indexed.
    let s2_keyed_arms = m.keyed.iter().filter(|kk| kk.member_host.is_none()).map(|kk| {
        let variant = format_ident!("Insert{}", camel(&kk.arrow.to_string()));
        let b = ent_of(&kk.member);
        let (b_count, b_removed, b_str) = (&b.count, &b.removed, &b.arena_str);
        let b_check = quote! {
            if let #rt::tx::RefKind::Existing(id) = b.kind {
                if ((id).0 as usize) < #b_count && #b_removed[(id).0 as usize] {
                    violations.push(Violation::DanglingEndpoint {
                        op: op_index,
                        record: RecordDesc::Staged(op_index as u32),
                        endpoint_arena: #b_str,
                        endpoint: (id).0,
                    });
                }
            }
        };
        match &kk.host {
            None => quote! {
                if let Op::#variant { k: _, b } = op {
                    #b_check
                }
            },
            Some(host) => {
                let h = ent_of(host);
                let (h_count, h_removed) = (&h.count, &h.removed);
                quote! {
                    if let Op::#variant { a, k: _, b } = op {
                        if let #rt::tx::RefKind::Existing(id) = a.kind {
                            if ((id).0 as usize) < #h_count && #h_removed[(id).0 as usize] {
                                // a removed host is RemovedPathHost's
                                // report, not a dangler at the insert op
                            } else {
                                #b_check
                            }
                        } else {
                            #b_check
                        }
                    }
                }
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

    // Step 3 — compaction maps. An embedded arena compacts PER OLD HOST:
    // kept hosts compact their kept sub records in old order; a removed
    // host's map is all-`None` (the kept sets are strictly per-arena — the
    // cascade is applied here, at map time, and at record-copy time, and
    // both places agree).
    let s3_maps = ents.iter().map(|x| {
        let (map, removed, c) = (&x.map, &x.removed, &x.count);
        match x.host {
            None => quote! {
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
            },
            Some(hi) => {
                let h = &ents[hi];
                let (h_count, h_removed, h_arena) = (&h.count, &h.removed, &h.arena);
                let (e_arena, e_removed_by) = (&x.arena, &x.removed_by);
                quote! {
                    let mut #map: ::std::vec::Vec<::std::vec::Vec<::core::option::Option<u32>>> =
                        ::std::vec::Vec::with_capacity(#h_count);
                    for old_host in 0..#h_count {
                        let old_len = old.__store().#h_arena[old_host].#e_arena.len();
                        if #h_removed[old_host] {
                            // Host removed: its embedded arena dies with it.
                            // Every old sub reports as removed — nothing was
                            // kept, because there is no host to keep it in.
                            #map.push(::std::vec![::core::option::Option::None; old_len]);
                            continue;
                        }
                        let mut per_host = ::std::vec![::core::option::Option::None; old_len];
                        let mut next = 0u32;
                        for (old_idx, slot) in per_host.iter_mut().enumerate() {
                            if #e_removed_by[old_host][old_idx].is_none() {
                                *slot = ::core::option::Option::Some(next);
                                next += 1;
                            }
                        }
                        #map.push(per_host);
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
    // Step 4b-pre — the keyed carries, derived beside the walk arrays:
    // each store-level map filters to entries whose key and target both
    // survive the change set and remaps both through the compaction maps
    // (keyed maps are carried as state, never rebuilt — a keyed arrow
    // (a `FlushError` naming them) is the keyed-carry step's — the
    // KeyRemovedWithLiveEntries pass above. A per-record map is not a
    // store field: its carry rides the host record's field init
    // (record_field_init's KeyedMap arm), not this store-level loop.
    let s4_keyed_maps = m.keyed.iter().filter(|kk| kk.host.is_none()).map(|kk| {
        let carried = format_ident!("__{}_keyed_map", kk.arrow);
        let k = ent_of(&kk.key_entity);
        let b = ent_of(&kk.member);
        let (k_map, b_map) = (&k.map, &b.map);
        let (k_id, b_id) = (&k.id_ty, &b.id_ty);
        let mf = &kk.map_field;
        quote! {
            let mut #carried: ::std::collections::HashMap<#k_id, #b_id> =
                ::std::collections::HashMap::new();
            for (key, val) in __old.#mf.iter() {
                if let (
                    ::core::option::Option::Some(nk),
                    ::core::option::Option::Some(nv),
                ) = (#k_map[(key).0 as usize], #b_map[(val).0 as usize]) {
                    #carried.insert(#k_id(nk), #b_id(nv));
                }
            }
        }
    });
    // Step 4c-pre — the csr (phased) carries, run AFTER the skeleton: the
    // hosts' build-phase buckets exist (the skeleton fires each kept
    // record's `on_add`, which opens one bucket per kept host, in
    // compaction order). Each walk READS only the old store's frozen
    // ranges and WRITES only the new store's buckets through the backing's
    // `on_insert` hook (a pure append) — kept per-host forward order
    // preserved exactly, removed members skipped without reordering the
    // rest (per-field remap would truncate at a removed mid-range member).
    // Runs before replay so staged inserts append to the carried tails.
    let s4_csr_carries = m.csr.iter().map(|c| {
        let h = ent_of(&c.host);
        let b = ent_of(&c.member);
        let (offsets, adj, hook) = (&c.offsets_field, &c.adj_field, &c.insert_hook);
        let (h_count, h_map, h_id) = (&h.count, &h.map, &h.id_ty);
        let (b_map, b_id) = (&b.map, &b.id_ty);
        quote! {
            for old_host in 0..#h_count {
                if let ::core::option::Option::Some(new_host) = #h_map[old_host] {
                    for __slot in (__old.#offsets[old_host] as usize)
                        ..(__old.#offsets[old_host + 1] as usize)
                    {
                        let __old_b = __old.#adj[__slot];
                        if let ::core::option::Option::Some(new_b) =
                            #b_map[__old_b.0 as usize]
                        {
                            new_store.#hook(#h_id(new_host), #b_id(new_b));
                        }
                    }
                }
            }
        }
    });
    // A field of a kept record's skeleton literal: ctor-arg ids remapped
    // through the target's map; a derived head/link from its array
    // (indexed by THIS record's new id); containers Default; an embedded
    // arena rebuilt per host (kept subs in old order, their own fields
    // remapped) from the OLD host record being read as `rec` at old id
    // `old_id` (both loop paths bind those names).
    let record_field_init = |entity: &Ident2, field: &TxField| -> TokenStream2 {
        let n = &field.emitted;
        // A per-record keyed map rides as state, never rebuilt: the kept
        // owner's new map receives exactly its surviving old entries —
        // the target survives (target-as-membership clearing), with the
        // key remapped through the key entity's map and the target
        // remapped through the compaction maps (a path-id target through
        // the COMPOSED maps — this owner's new id and its own per-host
        // sub map; value-host agreement, a construction invariant checked
        // by validate, puts the target in this owner's embedded arena).
        // Step 2 proved a surviving entry's key is not removed (else
        // KeyRemovedWithLiveEntries fired), so the key unwrap is an
        // internal bug, and the target unwrap likewise (the filter above
        // is exactly step 2's survival test).
        if let Some(kk) = m
            .keyed
            .iter()
            .find(|kk| kk.host.as_ref() == Some(entity) && kk.map_field == field.emitted)
        {
            let k = ent_of(&kk.key_entity);
            let b = ent_of(&kk.member);
            let (k_map, k_id) = (&k.map, &k.id_ty);
            let (b_map, b_id) = (&b.map, &b.id_ty);
            let (survives, new_target) = match &kk.member_host {
                None => {
                    let b_removed = &b.removed;
                    (
                        quote!(if #b_removed[(val).0 as usize] { continue; }),
                        quote!(#b_id(#b_map[(val).0 as usize]
                            .expect("carried entry's target kept: step 2 proved it"))),
                    )
                }
                Some(_) => {
                    let b_removed_by = &b.removed_by;
                    let h_id = &ent_of(entity).id_ty;
                    (
                        quote!(if #b_removed_by[old_id][(val).1 as usize].is_some() { continue; }),
                        quote!(#b_id(
                            #h_id(new_id),
                            #b_map[old_id][(val).1 as usize]
                                .expect("carried entry's target kept: step 2 proved it"),
                        )),
                    )
                }
            };
            return quote! {
                #n: {
                    let mut __carried: ::std::collections::HashMap<#k_id, #b_id> =
                        ::std::collections::HashMap::new();
                    for (key, val) in rec.#n.iter() {
                        #survives
                        __carried.insert(
                            #k_id(#k_map[(key).0 as usize]
                                .expect("carried entry's key kept: step 2 proved it")),
                            #new_target,
                        );
                    }
                    __carried
                },
            };
        }
        match &field.init {
            TxFieldInit::NestedArena(emb_name) => {
                let emb_e = m
                    .entities
                    .iter()
                    .find(|e| e.name == *emb_name)
                    .expect("the embedded entity is a model entity");
                let emb = ent_of(emb_name);
                let (emb_map, emb_data) = (&emb.map, &emb.data);
                // The embedded record's own fields: ctor-arg ids remapped
                // through the (root-level) target's map — no set verb
                // exists on an embedded self this step, so the plain branch
                // is the only one; `Option` fields born None; containers
                // Default.
                let sub_inits = emb_e.fields.iter().map(|sf| {
                    let sn = &sf.emitted;
                    match &sf.init {
                        TxFieldInit::CtorArg { target, .. } => {
                            let t = ent_of(target);
                            let (t_map, t_id) = (&t.map, &t.id_ty);
                            quote!(#sn: #t_id(#t_map[(srec.#sn).0 as usize]
                                .expect("field target kept: step 2 proved it")),)
                        }
                        TxFieldInit::None_ => quote!(#sn: ::core::option::Option::None,),
                        TxFieldInit::New => quote!(#sn: ::std::default::Default::default(),),
                        TxFieldInit::NestedArena(_) => unreachable!(
                            "embedding depth 1: an embedded record hosts no arenas"
                        ),
                    }
                });
                return quote! {
                    #n: {
                        let mut v = ::std::vec::Vec::with_capacity(rec.#n.len());
                        for old_idx in 0..rec.#n.len() {
                            if let ::core::option::Option::Some(_new_idx) = #emb_map[old_id][old_idx] {
                                let srec = &rec.#n[old_idx];
                                v.push(#emb_data { #(#sub_inits)* });
                            }
                        }
                        v
                    },
                };
            }
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
    // A store-level field: a carried keyed map, a root-hosted arrow's
    // single head value, else the uniform init.
    let store_field_expr = |field: &TxField| -> TokenStream2 {
        if let Some(kk) = m.keyed.iter().find(|kk| kk.map_field == field.emitted) {
            let carried = format_ident!("__{}_keyed_map", kk.arrow);
            return quote!(#carried);
        }
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
    // fired per kept record. Embedded entities have NO top-level arena —
    // their records are rebuilt inside the host record's NestedArena field
    // init — so every root-arena loop skips them.
    let s4_borrowed_arena_inits = m.entities.iter().filter(|e| e.host.is_none()).map(|e| {
        let x = ent_of(&e.name);
        let (arena, c) = (&x.arena, &x.count);
        quote!(#arena: ::std::vec::Vec::with_capacity(#c),)
    });
    let s4_borrowed_store_inits = m.store_fields.iter().map(|f| {
        let n = &f.emitted;
        let v = store_field_expr(f);
        quote!(#n: #v,)
    });
    let s4_borrowed_record_loops = m.entities.iter().filter(|e| e.host.is_none()).map(|e| {
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
    let s4_owned_compactions = m.entities.iter().filter(|e| e.host.is_none()).map(|e| {
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

    // Step 4c — replay constructive ops through the REAL constructors
    // (on_add growth + insert hooks fire as construction always fires
    // them), GROUPED root-then-nested: pass 1 replays every root-level op
    // in submission order and fills the per-entity replay tables; pass 2
    // replays nested adds in submission order, resolving each op's host
    // segment — `Existing` through the host's compaction map (host ids
    // SHIFT under removal; the translation is load-bearing), `Pending`
    // through the host's replay table, which pass 1 has already filled
    // completely. Removals never replay. At depth 1 the grouping is
    // resolution-redundant (a `Pending` host's add precedes its consumers
    // in submission order anyway); it is the shape the plan pins and
    // becomes load-bearing at depth 2+. The grouping reorders nothing
    // observable: no arena receives records from both passes (root arenas
    // append only in pass 1, embedded arenas only in pass 2), and
    // submission order is preserved within each pass.
    let s4_honor_preds = sets.iter().map(|s| {
        let pred = &s.pred;
        quote!(
            let mut #pred: ::std::vec::Vec<::core::option::Option<u32>> =
                ::std::vec![::core::option::Option::None; ops.len()];
        )
    });
    let s4c_pending = ents.iter().filter(|x| x.host.is_none()).map(|x| {
        let (p, id_ty) = (&x.pending, &x.id_ty);
        quote!(let mut #p: ::std::vec::Vec<::core::option::Option<#id_ty>> =
            ::std::vec![::core::option::Option::None; ops.len()];)
    });
    let s4c_add_arms = m.entities.iter().filter(|e| e.host.is_none()).map(|e| {
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
    // Nested adds (pass 2): resolve the host segment top-down — `Existing`
    // through the host's step-3 map, `Pending` through the replay table
    // pass 1 filled. The replay-table lookup is BOUNDS-CHECKED (`get`):
    // the table is `ops.len()` long, and a forged `Pending` index (a
    // same-crate test can build one; the public API cannot — `Pending`
    // host refs are minted only by the host's add, at their own index)
    // must panic with the internal-bug message, not a bare index panic.
    // A nested add fills no replay table slot: nothing stages a ref to a
    // nested record this step.
    let s4c_nested_add_arms = m.entities.iter().filter(|e| e.host.is_some()).map(|e| {
        let x = ent_of(&e.name);
        let h = &ents[x.host.unwrap()];
        let variant = format_ident!("Add{}", e.name);
        let add_fn = format_ident!("add_{}", &x.snake);
        let (h_map, h_pending, h_id) = (&h.map, &h.pending, &h.id_ty);
        let h_snake_str = h.snake.to_string();
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
            if let Op::#variant { host, #(#arg_use),* } = op {
                let host = match host.kind {
                    #rt::tx::RefKind::Existing(id) =>
                        #h_id(#h_map[(id).0 as usize]
                            .expect("Existing host validated in step 1, kept past step 2")),
                    #rt::tx::RefKind::Pending(k) => match #h_pending.get(k as usize).copied().flatten() {
                        ::core::option::Option::Some(h) => h,
                        ::core::option::Option::None => panic!(
                            "flush: replay table miss for Pending {} op {}: internal bug — \
                             Pending {} refs are minted only by add_{}, at their own op index, \
                             and root constructive ops replay before nested ops",
                            stringify!(#h_id),
                            k,
                            stringify!(#h_id),
                            #h_snake_str,
                        ),
                    },
                };
                #(#resolves)*
                new_store.#add_fn(host, #(#arg_use),*);
            }
        }
    });
    // Pass 1 skips nested adds (pass 2's) alongside the removals.
    let nested_add_patterns = m.entities.iter().filter(|e| e.host.is_some()).map(|e| {
        let variant = format_ident!("Add{}", e.name);
        quote!(Op::#variant { .. })
    });
    // The nested arms exist only when the store has embedded arenas —
    // emitting the empty pattern arm would be a syntax error, and an
    // empty pass-2 loop would leave `op` unused.
    let pass1_nested_arm = has_nested.then(|| {
        let pats: Vec<_> = nested_add_patterns.collect();
        quote!(#(#pats)|* => {})
    });
    // Pass 2 itself is nested-stores-only.
    let pass2 = has_nested.then(|| {
        quote! {
            // Pass 2: nested constructive ops in submission order, each
            // resolving its host segment — Existing through the host's
            // compaction map, Pending through the replay table pass 1
            // filled.
            for op in ops.iter() {
                #(#s4c_nested_add_arms)*
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
        let hook = &i.hook;
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

    // Keyed insert replays resolve the key and target refs (Existing
    // through the compaction map, Pending through the replay table) and
    // call the backing's check-then-write hook on the new store — the
    // insert lands on top of the carried map, and the staging live-key
    // check proved the key free, so the hook's own duplicate check cannot
    // trip. A per-record map resolves the NAMED HOST the same way (an
    // add precedes its keyed inserts in submission order, so pass 1's
    // replay tables are already filled). An embedded target is a raw
    // path id remapped through the COMPOSED maps — the same translation
    // the carry applies (the probe's two-call-site pin); step 2 proved
    // both components remap (value-host agreement + liveness), so the
    // unwraps are internal bugs.
    let s4c_keyed_arms = m.keyed.iter().map(|kk| {
        let variant = format_ident!("Insert{}", camel(&kk.arrow.to_string()));
        let hook = &kk.hook;
        let k = ent_of(&kk.key_entity);
        let b = ent_of(&kk.member);
        let (k_map, k_p, k_id) = (&k.map, &k.pending, &k.id_ty);
        let (b_map, b_p, b_id) = (&b.map, &b.pending, &b.id_ty);
        let k_resolve = quote! {
            let k = match k.kind {
                #rt::tx::RefKind::Existing(id) =>
                    #k_id(#k_map[(id).0 as usize].expect("Existing id validated in steps 1-2")),
                #rt::tx::RefKind::Pending(i) => #k_p[i as usize]
                    .expect("an add precedes its use: submission order"),
            };
        };
        match (&kk.host, &kk.member_host) {
            (None, None) => quote! {
                Op::#variant { k, b } => {
                    #k_resolve
                    let b = match b.kind {
                        #rt::tx::RefKind::Existing(id) =>
                            #b_id(#b_map[(id).0 as usize].expect("Existing id validated in steps 1-2")),
                        #rt::tx::RefKind::Pending(i) => #b_p[i as usize]
                            .expect("an add precedes its use: submission order"),
                    };
                    new_store.#hook(k, b);
                }
            },
            (Some(host), None) => {
                let h = ent_of(host);
                let (h_map, h_p, h_id) = (&h.map, &h.pending, &h.id_ty);
                quote! {
                    Op::#variant { a, k, b } => {
                        let a = match a.kind {
                            #rt::tx::RefKind::Existing(id) =>
                                #h_id(#h_map[(id).0 as usize].expect("Existing id validated in steps 1-2")),
                            #rt::tx::RefKind::Pending(i) => #h_p[i as usize]
                                .expect("an add precedes its use: submission order"),
                        };
                        #k_resolve
                        let b = match b.kind {
                            #rt::tx::RefKind::Existing(id) =>
                                #b_id(#b_map[(id).0 as usize].expect("Existing id validated in steps 1-2")),
                            #rt::tx::RefKind::Pending(i) => #b_p[i as usize]
                                .expect("an add precedes its use: submission order"),
                        };
                        new_store.#hook(a, k, b);
                    }
                }
            }
            (Some(host), Some(mhost)) => {
                let h = ent_of(host);
                let mh = ent_of(mhost);
                let (h_map, h_p, h_id) = (&h.map, &h.pending, &h.id_ty);
                let (mh_map, mh_id) = (&mh.map, &mh.id_ty);
                quote! {
                    Op::#variant { a, k, b } => {
                        let a = match a.kind {
                            #rt::tx::RefKind::Existing(id) =>
                                #h_id(#h_map[(id).0 as usize].expect("Existing id validated in steps 1-2")),
                            #rt::tx::RefKind::Pending(i) => #h_p[i as usize]
                                .expect("an add precedes its use: submission order"),
                        };
                        #k_resolve
                        let b = #b_id(
                            #mh_id(#mh_map[(b.0).0 as usize]
                                .expect("staged entry target host kept: step 2 proved it")),
                            #b_map[(b.0).0 as usize][b.1 as usize]
                                .expect("staged entry target kept: step 2 proved it"),
                        );
                        new_store.#hook(a, k, b);
                    }
                }
            }
            (None, Some(_)) => unreachable!("gated off at classification"),
        }
    });

    // Custom op replay: run the staged closure against the rebuilt store,
    // handing it the `Resolve` context (maps + replay tables). The closure
    // is taken from its `Option` — the queue is borrowed mutably so the
    // pass can take it, and a custom op replays exactly once. A custom op
    // fills no replay table itself: refs to staged records mint through
    // the add verbs (whose own ops fill the tables), and records the
    // closure inserts directly are reachable only through the store.
    let s4c_custom_arm = {
        let resolve_inits = root_ents.iter().map(|x| {
            let (map, pending) = (&x.map, &x.pending);
            quote!(#map: &#map, #pending: &#pending,)
        });
        quote! {
            Op::Custom { run, .. } => {
                let run = run.take().expect("flush: a custom op replays exactly once");
                let __resolve = Resolve { #(#resolve_inits)* };
                run(&mut new_store, &__resolve);
            }
        }
    };

    // The analysis' old-store parameter: `Store<Frozen>` on a phased store
    // (the csr carry presupposes frozen old state), bare otherwise.
    let s4_analyze_sig = quote! {
        fn __analyze(
            old: &#store_frozen,
            ops: &[Op],
        ) -> ::core::result::Result<__TxAnalysis, FlushError>
    };
    // The skeleton write (step 4a/4b): a non-phased store keeps the two
    // paths — a fresh literal on the borrowed transaction, in-place
    // forward compaction on the owned one. A phased store builds a fresh
    // `Store<Build>` literal (the `_phase` marker field rides along; the
    // build-phase accumulators start empty and grow through the same
    // hooks construction uses) — the owned path is not emitted for phased
    // stores, so the match degenerates to the borrowed arm.
    let s4_skeleton = if m.phased {
        quote! {
            let OldStore::Borrowed(old) = old;
            let mut new_store = #store::<Build> {
                #(#s4_borrowed_arena_inits)*
                #(#s4_borrowed_store_inits)*
                _phase: ::core::marker::PhantomData,
            };
            #(#s4_borrowed_record_loops)*
        }
    } else {
        quote! {
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
        }
    };
    // The ending (steps 6-7): a phased store freezes the built store and
    // then validates (`finish()` — finalize BEFORE validate, the ordering
    // the m3 probe established; the outcome carries the frozen store). A
    // non-phased store has no finalization step and validates the
    // skeleton in place.
    let s4_finish = if m.phased {
        quote! {
            // -- steps 6-7: backing finalization, then validate -----------
            let frozen = new_store.finish().map_err(|detail| FlushError {
                violations: ::std::vec![Violation::ValidateFailed { detail }],
            })?;
            ::core::result::Result::Ok(FlushOutcome {
                store: frozen,
                report: FlushReport { #(#report_init)* },
            })
        }
    } else {
        quote! {
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
    let flush_method = quote! {
        #[doc(hidden)]
        #[doc = "Steps 1-2 as one pure analysis over (old store, ops): the resolved removal sets (with attributing op indices), per-arena counts, and every violation found in old-id space. `check` and `flush` consume the same function, so a green check and a committing flush can never disagree."]
        #s4_analyze_sig
        {
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
                    #(#s1_keyed_arms)*
                    #s1_custom_arm
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
            // Path coherence: ops staged under a removed host (nested stores).
            #(#s2_removed_path_hosts)*
            // Key-as-reference: removed keys with live entries or staged
            // inserts (the survivors-only cascade).
            #(#s2_key_removed)*
            // Path-id keyed inserts: value-host agreement + target liveness.
            #(#s2_path_keyed)*
            for (op_index, op) in ops.iter().enumerate() {
                #(#s2_staged_arms)*
                #(#s2_insert_arms)*
                #(#s2_keyed_arms)*
                #s2_custom_arm
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

        #[doc = "Run the flush pipeline and consume the transaction: (1) resolve removals and range-validate every `Existing` ref, (2) dangler detection over kept records and staged adds in old-id space — steps 1-2 are the same `check` analysis, (3) compaction with per-arena old->new id maps, (4) the kept skeleton (records with remapped id fields, intrusive lists carried by arrays derived from the old chains BEFORE any write, phased backings carried by walks of the old store's frozen ranges) then submission-order replay through the same constructors construction uses, (5) induction (constructor logic — none staged by the built-ins), (6) backing finalization (a phased store's freeze — non-phased stores have none), (7) `validate()` — steps 6-7 are a phased store's `finish()`. A borrowed transaction builds fresh arenas (a `Store<Build>` skeleton for a phased store); an owned one (`into_edit`, non-phased stores only) forward-compacts in place — every write slot at or before its read slot — reusing the old store's allocations. The old store (when borrowed) is untouched; a rejection leaves nothing half-applied."]
        pub fn flush(self) -> ::core::result::Result<FlushOutcome, FlushError> {
            let Tx { old, token: _, ops } = self;
            let mut ops = ops;
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
            #(#s4_keyed_maps)*

            // -- step 4a/4b: the kept skeleton ----------------------------
            #s4_skeleton

            // -- step 4c-pre: the csr carries (phased backings) ------------
            // Each walk feeds the new store's build-phase buckets through
            // the backing's on_insert hook, in the old store's frozen
            // forward order — after the skeleton (the hosts' buckets
            // exist) and before replay (staged inserts append to the
            // carried tails).
            #(#s4_csr_carries)*

            // -- step 4c: replay, grouped root-then-nested -----------------
            #(#s4c_pending)*
            // Pass 1: every root-level constructive op in submission order
            // (adds fill the per-entity replay tables; inserts, sets, and
            // keyed inserts replay against the rebuilt skeleton; custom ops
            // run their staged closures with the `Resolve` context). The
            // queue is borrowed MUTABLY so a custom op's closure can be
            // taken; every other arm only reads its op. Nested adds replay
            // in pass 2.
            for (op_index, op) in ops.iter_mut().enumerate() {
                match op {
                    #(#s4c_add_arms)*
                    #(#s4c_insert_arms)*
                    #(#s4c_set_arms)*
                    #(#s4c_keyed_arms)*
                    #s4c_custom_arm
                    #(#remove_patterns)|* => {}
                    #pass1_nested_arm
                }
            }
            #pass2

            // -- steps 5-6: induction and backing finalization -------------
            // Induction is constructor logic and replays through the same
            // constructors above; a non-phased store has no finalization
            // step (a phased store's freeze rides in `finish()` below).

            #s4_finish
        }
    };

    // What a Tx stages against. A non-phased store has both paths (the
    // owned one in-place forward-compacts, reusing the old store's arena
    // allocations); a phased store borrows the frozen store only — the
    // owned in-place path against live build-phase accumulators is not
    // pinned by any probe, so `into_edit` is not emitted and the enum has
    // a single variant (the flush's skeleton match is irrefutable).
    let oldstore_def = if m.phased {
        quote! {
            #[doc(hidden)]
            #[doc = concat!("What a `Tx` stages against: the shared borrow of the frozen store (a phased store's transaction is borrowed-only — `into_edit` is not emitted for phased stores).")]
            enum OldStore<'a> {
                Borrowed(&'a #store_frozen),
            }

            impl OldStore<'_> {
                fn __store(&self) -> &#store_frozen {
                    let OldStore::Borrowed(s) = self;
                    s
                }
            }
        }
    } else {
        quote! {
            #[doc(hidden)]
            #[doc = concat!("What a `Tx` stages against: the shared borrow (`edit`) or the consumed store (`into_edit` — flush then reuses its arena allocations).")]
            enum OldStore<'a> {
                Borrowed(&'a #store_frozen),
                Owned(::std::boxed::Box<#store_frozen>),
            }

            impl OldStore<'_> {
                fn __store(&self) -> &#store_frozen {
                    match self {
                        OldStore::Borrowed(s) => s,
                        OldStore::Owned(b) => b,
                    }
                }
            }
        }
    };
    let into_edit_method = if m.phased {
        TokenStream2::new()
    } else {
        quote! {
            #[doc = "Consume the store and start staging. The borrow checker then requires every old handle to be dead before the pass can end, and `flush` reuses the store's arena allocations (in-place compaction) instead of cloning. Old-store reads during staging are by raw id — handles into the moved store cannot exist."]
            pub fn into_edit(self) -> Tx<'static> {
                Tx {
                    old: OldStore::Owned(::std::boxed::Box::new(self)),
                    token: #rt::tx::mint_token(),
                    ops: ::std::vec::Vec::new(),
                }
            }
        }
    };

    // -- the transaction ---------------------------------------------------
    quote! {
        #aliases
        #ops_enum
        #into_traits
        #handle_impls
        #named_ref_enum
        #reserved_key_enum
        #submission_key_enum
        #resolve_struct
        #flush_types

        #subtx_defs

        #oldstore_def

        #[doc = concat!("Transaction over `", stringify!(#store), "`: stages a change set against the old store and flushes it as a full rebuild. Staging never touches the store — the change set is a plain queue, a first-class value.")]
        pub struct Tx<'a> {
            pub(crate) old: OldStore<'a>,
            pub(crate) token: #rt::tx::TxToken,
            pub(crate) ops: ::std::vec::Vec<Op>,
        }

        impl #store_frozen {
            #[doc = "Borrow the store sharedly and start staging. Reads and old handles keep working while staging and after the flush; the old store is never mutated (the escape hatch — `into_edit`, non-phased stores only, enforces the pass discipline at compile time)."]
            pub fn edit(&self) -> Tx<'_> {
                Tx {
                    old: OldStore::Borrowed(self),
                    token: #rt::tx::mint_token(),
                    ops: ::std::vec::Vec::new(),
                }
            }

            #into_edit_method
        }

        impl Tx<'_> {
            /// The old store this transaction stages against — the read
            /// side of the pass discipline. On a borrowed transaction
            /// (`edit`) this is the original store and old handles still
            /// work; on an owned one (`into_edit`) it borrows the store
            /// the transaction now owns (reads by raw id; handles into it
            /// cannot exist by construction).
            pub fn old(&self) -> &#store_frozen {
                self.old.__store()
            }

            #staging

            #insert_staging

            #set_staging

            #keyed_staging

            #custom_staging

            #flush_method
        }

        #write_impls
    }
}
