//! Composed-store codegen — the codegen stage of `__implement_gen!` for a
//! composed pattern (plans/milestone-2), entered once every member's root
//! metadata has been collected. Port of the spike's `expand_gen_composed`
//! onto the M1 baseline (registration-courier metadata, named hooks, the
//! combined constructor rule, re-scoped home/addressability checks).
//!
//! Model:
//! - Fusion classes: equivalence closure of `<=>` over (instance, entity)
//!   pairs. A class's record sums its members' capability fields; the class
//!   is named by its first element member's post-collision name.
//! - `<->` unifies id types only: the open side resolves to the closed
//!   side's class for every id/handle use (the *partner map*). No records
//!   merge, no arenas move.
//! - Placement: an instance's arenas embed in the host class's records
//!   exactly when the instance's root-marked entity is colocation-fused
//!   with an element entity of the host instance; otherwise they live at
//!   the store's root. Reference fusion never affects placement. The
//!   root-marked entity owns no arena, flat or composed.
//! - Handles: root store reference plus index path — `{store}` for a root
//!   handle, `{store, id}` at root level, `{store, host, idx}` embedded.
//!   `Eq`/`Hash` hand-forward to (store pointer, index path).
//! - Constructors merge the union of every member's total `one`-arrow
//!   arguments (composition-instance order, then member schema order);
//!   argument-name collisions across instances of one fusion class share
//!   one argument when the resolved target classes agree, and take the
//!   instance prefix when they differ (the combined rule).
//! - Backings bind per instance (`instance.arrow` keys).

use std::collections::HashMap;

use proc_macro2::{Ident as Ident2, TokenStream as TokenStream2};
use quote::{format_ident, quote};

use crate::grammar::{
    camel, marker_ty, path_tokens, snake, Binding, Cardinality, ComposedSchema, RootMeta,
};
use crate::ChainConfig;

/// A resolved arrow of one member instance: declaration, root metadata,
/// backing (`None` for derived `:= sum` arrows).
pub(crate) struct ArrowInfo<'a> {
    pub(crate) decl: &'a crate::grammar::ArrowDecl,
    pub(crate) meta: &'a RootMeta,
    pub(crate) backing: Option<String>,
}

/// Location of a fusion class's arena.
#[derive(Clone, Copy, PartialEq, Debug)]
enum ClassLoc {
    /// Arena is a field of the root store.
    Root,
    /// Arena is a `Vec` field of the host class's record. Frontier limit:
    /// embedding depth 1.
    Embedded(usize),
}

struct ClassData {
    /// Non-open members: (instance index, entity index), canonical element
    /// member first.
    members: Vec<(usize, usize)>,
    /// The class contains an instance's root-marked entity.
    has_root: bool,
    /// The class owns records (has an element member).
    owns_records: bool,
    loc: ClassLoc,
    /// Post-collision type name (`OuterNode`, `SubNode`).
    name: Ident2,
    /// `{Name}Id` — a path id `(HostId, u32)` when embedded.
    id_name: Ident2,
    /// `{Name}Data`.
    data_name: Ident2,
    /// Always instance-qualified: `outer_node_arena`, `sub_subnode_arena`.
    arena_fld: Ident2,
    /// Field contributions to this class's record, in collection order.
    fields: Vec<FieldContribution>,
    /// hash_index map fields on this class's record (keyed arrows sourced
    /// here): per-self-object key->target maps, hook-maintained.
    maps: Vec<(Ident2, Ident2, Ident2)>,
}

/// One field contribution to a class's record: what the record stores and
/// (for `field`-backed arrows) which constructor argument feeds it.
struct FieldContribution {
    /// Field name as spelled in the record (post collision-resolution).
    field: Ident2,
    /// Constructor argument feeding the field; `None` for intrusive
    /// heads/links (initialized empty, relinked by hooks).
    arg: Option<Ident2>,
    /// Stored id type (resolved through the partner map for open targets).
    ty: Ident2,
}

pub(crate) fn expand_composed(config: &ChainConfig) -> syn::Result<TokenStream2> {
    let c: &ComposedSchema = match &config.schema {
        crate::grammar::GenSchema::Composed(c) => c,
        _ => unreachable!("dispatched by expand"),
    };
    let store = &config.store;
    let members = &c.members;
    let n_inst = members.len();

    // ---- metadata lookup, keyed by member-package-resolved root paths ----
    let mut meta_of: HashMap<String, &RootMeta> = HashMap::new();
    {
        let mut queue: Vec<syn::Path> = Vec::new();
        for (_inst, pkg, body) in members {
            for a in &body.arrows {
                let p = a.inst.as_ref().expect("members are instantiation patterns");
                let ts = crate::resolve_schema_path(p, pkg)?;
                let key = ts.to_string();
                if !queue.iter().any(|r| path_tokens(r).to_string() == key) {
                    queue.push(syn::parse2(ts).expect("resolver emits a parseable path"));
                }
            }
        }
        if queue.len() != config.metas.len() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "internal: resolved {} root metadata tuples for {} roots",
                    config.metas.len(),
                    queue.len()
                ),
            ));
        }
        for (p, m) in queue.iter().zip(&config.metas) {
            meta_of.insert(path_tokens(p).to_string(), m);
        }
    }
    let meta_for = |inst: usize, decl: &crate::grammar::ArrowDecl| -> syn::Result<&RootMeta> {
        let pkg = &members[inst].1;
        let ts = crate::resolve_schema_path(decl.inst.as_ref().unwrap(), pkg)?;
        Ok(meta_of
            .get(&ts.to_string())
            .expect("stage1 queued every member arrow's root"))
    };

    // ---- roots per instance (pattern! guarantees exactly one) ----
    let root_of: Vec<usize> = members
        .iter()
        .map(|(_, _, b)| {
            b.entities
                .iter()
                .position(|e| e.root)
                .expect("pattern! validated exactly one root-marked entity")
        })
        .collect();

    // ---- union-find over `<=>`; partner map over `<->` ----
    let mut parent: Vec<usize> = (0..n_inst * 64).collect();
    fn find(parent: &mut Vec<usize>, x: usize) -> usize {
        if parent[x] != x {
            let r = find(parent, parent[x]);
            parent[x] = r;
            r
        } else {
            x
        }
    }
    let key = |i: usize, e: usize| i * 64 + e;
    for (li, (_, _, body)) in members.iter().enumerate() {
        let inst = &members[li].0;
        if body.entities.len() >= 64 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("instance `{inst}` declares {} entities; composed codegen supports at most 63 per instance", body.entities.len()),
            ));
        }
    }
    let mut ref_partner: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
    for f in &c.fusions {
        let li = members.iter().position(|(i, _, _)| *i == f.lhs.0).expect("validated");
        let ri = members.iter().position(|(i, _, _)| *i == f.rhs.0).expect("validated");
        let le = members[li]
            .2
            .entities
            .iter()
            .position(|e| e.name == f.lhs.1)
            .expect("validated");
        let re = members[ri]
            .2
            .entities
            .iter()
            .position(|e| e.name == f.rhs.1)
            .expect("validated");
        match f.op {
            crate::grammar::FusionOp::Coloc => {
                let (a, b) = (find(&mut parent, key(li, le)), find(&mut parent, key(ri, re)));
                parent[a.max(b)] = a.min(b);
            }
            crate::grammar::FusionOp::Ref => {
                if members[li].2.entities[le].open {
                    ref_partner.insert((li, le), (ri, re));
                } else {
                    ref_partner.insert((ri, re), (li, le));
                }
            }
        }
    }
    // Fully compress so later lookups need only `&parent`.
    for x in 0..parent.len() {
        find(&mut parent, x);
    }
    // Entity key -> owning record class key (open entities resolve through
    // their `<->` partner).
    let resolve = |i: usize, e: usize| -> usize {
        let (i, e) = ref_partner.get(&(i, e)).copied().unwrap_or((i, e));
        let mut x = key(i, e);
        while parent[x] != x {
            x = parent[x];
        }
        x
    };

    // ---- gather classes over non-open entities ----
    let mut class_keys: Vec<usize> = Vec::new();
    let mut class_map: HashMap<usize, usize> = HashMap::new();
    let mut classes: Vec<ClassData> = Vec::new();
    let mut class_of_key: HashMap<(usize, usize), usize> = HashMap::new();
    for i in 0..n_inst {
        for (e, ent) in members[i].2.entities.iter().enumerate() {
            if ent.open {
                continue;
            }
            let k = resolve(i, e);
            let idx = *class_map.entry(k).or_insert_with(|| {
                class_keys.push(k);
                classes.push(ClassData {
                    members: Vec::new(),
                    has_root: false,
                    owns_records: false,
                    loc: ClassLoc::Root,
                    name: Ident2::new("_pending", proc_macro2::Span::call_site()),
                    id_name: Ident2::new("_pending", proc_macro2::Span::call_site()),
                    data_name: Ident2::new("_pending", proc_macro2::Span::call_site()),
                    arena_fld: Ident2::new("_pending", proc_macro2::Span::call_site()),
                    fields: Vec::new(),
                    maps: Vec::new(),
                });
                classes.len() - 1
            });
            classes[idx].members.push((i, e));
            class_of_key.insert((i, e), idx);
        }
    }

    // has_root / owns_records; canonical member first (the class is *named*
    // by its first element member).
    for ci in 0..classes.len() {
        classes[ci].has_root = classes[ci]
            .members
            .iter()
            .any(|&(i, e)| root_of[i] == e);
        classes[ci].owns_records = classes[ci]
            .members
            .iter()
            .any(|&(i, e)| root_of[i] != e);
        if let Some(pos) = classes[ci]
            .members
            .iter()
            .position(|&(i, e)| root_of[i] != e)
        {
            classes[ci].members.swap(0, pos);
        }
    }

    // ---- names: post-collision, from the canonical member ----
    let decl_count = |name: &str| members.iter().filter(|(_, _, b)| b.entities.iter().any(|x| x.name == name)).count();
    for cl in classes.iter_mut() {
        let (i, e) = cl.members[0];
        let ent = &members[i].2.entities[e];
        let mut name = ent.name.to_string();
        if decl_count(&name) > 1 {
            name = format!("{}{}", camel(&members[i].0.to_string()), name);
        }
        cl.name = Ident2::new(&name, ent.name.span());
        cl.id_name = format_ident!("{}Id", name);
        cl.data_name = format_ident!("{}Data", name);
        cl.arena_fld = format_ident!(
            "{}_{}_arena",
            snake(&members[i].0.to_string()),
            snake(&ent.name.to_string())
        );
    }

    // ---- instance placement: embedded iff the instance's root class has
    // an element member (the root is `<=>`-fused with a host element) ----
    let embedded_host: Vec<Option<usize>> = (0..n_inst)
        .map(|i| {
            let rk = resolve(i, root_of[i]);
            let ci = class_map[&rk];
            classes[ci].owns_records.then_some(ci)
        })
        .collect();
    for ci in 0..classes.len() {
        if !classes[ci].owns_records {
            continue; // pure-root class: no arena anywhere
        }
        if classes[ci].has_root {
            // Host class: its element members must belong to root-placed
            // instances (embedding depth 1).
            for &(i, e) in &classes[ci].members {
                if root_of[i] != e && embedded_host[i].is_some() {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!(
                            "embedding depth > 1: instance `{}` hosts arenas and is itself embedded (frontier limit)",
                            members[i].0
                        ),
                    ));
                }
            }
            classes[ci].loc = ClassLoc::Root;
        } else {
            let host_inst = classes[ci].members[0].0;
            classes[ci].loc = match embedded_host[host_inst] {
                Some(host) => ClassLoc::Embedded(host),
                None => ClassLoc::Root,
            };
        }
    }

    // ---- per-instance arrow resolution (qualified binding keys) ----
    let mut inst_infos: Vec<Vec<ArrowInfo>> = Vec::new();
    for (inst_name, _, body) in members.iter() {
        let mut infos: Vec<ArrowInfo> = Vec::new();
        for decl in &body.arrows {
            let meta = meta_for(members.iter().position(|(i, _, _)| i == inst_name).unwrap(), decl)?;
            match (meta.card, &decl.key) {
                (Cardinality::Keyed, None) => {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!(
                            "arrow `{inst_name}.{}` instantiates the keyed root `{}` but does not bind a key sort (`key(LocalEntity)`)",
                            decl.name,
                            path_tokens(decl.inst.as_ref().unwrap())
                        ),
                    ));
                }
                (card, Some(k)) if card != Cardinality::Keyed => {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!(
                            "arrow `{inst_name}.{}` declares `key({k})` but its root `{}` is not a keyed root",
                            decl.name,
                            path_tokens(decl.inst.as_ref().unwrap())
                        ),
                    ));
                }
                _ => {}
            }
            let explicit = config
                .bindings
                .iter()
                .find(|b| b.instance.as_ref() == Some(inst_name) && b.arrow == decl.name)
                .map(|b| b.backing.to_string());
            let backing = if decl.sum.is_some() {
                None
            } else if let Some(b) = explicit {
                let ok = match b.as_str() {
                    "field" => meta.card == Cardinality::One,
                    "arena" | "intrusive_list" => meta.card == Cardinality::Many,
                    "hash_index" => meta.card == Cardinality::Keyed,
                    _ => true,
                };
                if !ok {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!(
                            "backing `{b}` has wrong cardinality for arrow `{inst_name}.{}` (a `{}` arrow)",
                            decl.name,
                            match meta.card {
                                Cardinality::Many => "many",
                                Cardinality::One => "one",
                                Cardinality::Keyed => "keyed",
                            }
                        ),
                    ));
                }
                Some(b)
            } else if meta.card == Cardinality::Many {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "missing backing binding for `many` arrow `{inst_name}.{}` (`arena` vs `intrusive_list` is an explicit choice)",
                        decl.name
                    ),
                ));
            } else if meta.card == Cardinality::Keyed {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "missing backing binding for keyed arrow `{inst_name}.{}` (`hash_index` is the only keyed-legal backing)",
                        decl.name
                    ),
                ));
            } else {
                Some("field".to_string())
            };
            infos.push(ArrowInfo { decl, meta, backing });
        }
        inst_infos.push(infos);
    }
    for b in &config.bindings {
        if !["arena", "field", "intrusive_list", "hash_index"]
            .contains(&b.backing.to_string().as_str())
        {
            return Err(syn::Error::new(
                b.arrow.span(),
                format!(
                    "unknown backing `{}` for arrow `{}` (built-ins: `arena`, `field`, `intrusive_list`, `hash_index`)",
                    b.backing,
                    b.key_str()
                ),
            ));
        }
    }

    // ---- resolved class helpers ----
    let class_index_of = |i: usize, e: usize| -> usize {
        let k = resolve(i, e);
        class_of_key
            .get(&(i, e))
            .copied()
            .unwrap_or_else(|| class_map[&k])
    };
    let ent_index = |i: usize, name: &Ident2| -> syn::Result<usize> {
        members[i]
            .2
            .entities
            .iter()
            .position(|e| e.name == *name)
            .ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("internal: entity `{name}` missing from instance"),
                )
            })
    };
    // ---- field contributions per class, with the combined ctor rule ----
    // (member i, arrow name) -> field name in the class record; per class,
    // contributions ride in ClassData::fields in collection order
    // (composition-instance order, then member schema order).
    // An open entity closed by `<->` is never a class member — its records
    // ARE the partner's — so its arrow fields contribute to the hosting
    // class (e.g. the `next_elems` link of a chain rides the inner-edge
    // records that close `Elem`).
    let hosted: Vec<Vec<(usize, usize)>> = {
        let mut hosted: Vec<Vec<(usize, usize)>> =
            classes.iter().map(|c| c.members.clone()).collect();
        for i in 0..n_inst {
            for (e, ent) in members[i].2.entities.iter().enumerate() {
                if ent.open {
                    let ci = class_index_of(i, e);
                    if classes[ci].owns_records {
                        hosted[ci].push((i, e));
                    }
                }
            }
        }
        hosted
    };
    let mut field_of_arrow: HashMap<(usize, String), Ident2> = HashMap::new();
    for ci in 0..classes.len() {
        if !classes[ci].owns_records {
            continue;
        }
        let cls_members = hosted[ci].clone();
        for (i, e) in cls_members {
            for info in &inst_infos[i] {
                let d = info.decl;
                match info.backing.as_deref() {
                    Some("field") if members[i].2.entities[e].name == d.source => {
                        let tid = classes[class_index_of(i, ent_index(i, &d.target)?)].id_name.clone();
                        let want = d.name.to_string();
                        if let Some(pos) = classes[ci].fields
                            .iter()
                            .position(|fc| fc.field.to_string() == want && fc.arg.is_some())
                        {
                            // Intra-class argument-name collision: share when
                            // the resolved target classes agree (the partner
                            // map usually guarantees this), else prefix.
                            if classes[ci].fields[pos].ty == tid {
                                field_of_arrow.insert((i, want.clone()), classes[ci].fields[pos].field.clone());
                                continue;
                            }
                            let prefixed = format_ident!(
                                "{}_{}",
                                snake(&members[i].0.to_string()),
                                want
                            );
                            field_of_arrow.insert((i, want.clone()), prefixed.clone());
                            classes[ci].fields.push(FieldContribution {
                                field: prefixed.clone(),
                                arg: Some(prefixed),
                                ty: tid,
                            });
                            continue;
                        }
                        let f = d.name.clone();
                        field_of_arrow.insert((i, want), f.clone());
                        classes[ci].fields.push(FieldContribution {
                            field: f.clone(),
                            arg: Some(f),
                            ty: tid,
                        });
                    }
                    Some("intrusive_list") => {
                        let head = format_ident!("first_{}", crate::grammar::link_stem(&d.name));
                        let next = format_ident!("next_{}", crate::grammar::link_stem(&d.name));
                        let tid = classes[class_index_of(i, ent_index(i, &d.target)?)].id_name.clone();
                        if members[i].2.entities[e].name == d.source {
                            if classes[ci].fields.iter().any(|fc| fc.field == head) {
                                let p = format_ident!(
                                    "{}_{}",
                                    snake(&members[i].0.to_string()),
                                    head
                                );
                                classes[ci].fields.push(FieldContribution { field: p, arg: None, ty: tid.clone() });
                            } else {
                                classes[ci].fields.push(FieldContribution {
                                    field: head,
                                    arg: None,
                                    ty: tid.clone(),
                                });
                            }
                        }
                        if members[i].2.entities[e].name == d.target {
                            if classes[ci].fields.iter().any(|fc| fc.field == next) {
                                let p = format_ident!(
                                    "{}_{}",
                                    snake(&members[i].0.to_string()),
                                    next
                                );
                                classes[ci].fields.push(FieldContribution { field: p, arg: None, ty: tid });
                            } else {
                                classes[ci].fields.push(FieldContribution {
                                    field: next,
                                    arg: None,
                                    ty: tid,
                                });
                            }
                        }
                    }
                    // hash_index map on the self record (per-self-object
                    // map; a pure-root self contributes at store level
                    // instead — handled with the store fields below).
                    Some("hash_index") if members[i].2.entities[e].name == d.source => {
                        let kid = classes
                            [class_index_of(i, ent_index(i, d.key.as_ref().unwrap())?)]
                        .id_name
                        .clone();
                        let vt = classes
                            [class_index_of(i, ent_index(i, &d.target)?)]
                        .id_name
                        .clone();
                        let base = format_ident!("{}_index", d.name);
                        let idx = if classes[ci].maps.iter().any(|(f, _, _)| *f == base) {
                            format_ident!("{}_{}", snake(&members[i].0.to_string()), base)
                        } else {
                            base
                        };
                        classes[ci].maps.push((idx, kid, vt));
                    }
                    _ => {}
                }
            }
        }
    }

    let id_ty_of =
        |i: usize, e: usize| -> Ident2 { classes[class_index_of(i, e)].id_name.clone() };

    // ---- re-scoped home/addressability checks (implement! time) ----
    for (i, infos) in inst_infos.iter().enumerate() {
        for info in infos {
            let d = info.decl;
            let sc = class_index_of(i, ent_index(i, &d.source)?);
            let tc = class_index_of(i, ent_index(i, &d.target)?);
            let named = format!("{}.{}", members[i].0, d.name);
            match info.backing.as_deref() {
                Some("field") => {
                    if !classes[sc].owns_records {
                        return Err(syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!(
                                "arrow `{named}` is sourced at the root-marked entity `{}`, which is unfused; the `field` backing needs a self record, and an unfused root owns none (it *is* the store)",
                                members[i].2.entities[ent_index(i, &d.source)?].name
                            ),
                        ));
                    }
                    if let ClassLoc::Embedded(_) = classes[tc].loc {
                        return Err(syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!(
                                "arrow `{named}` is `field`-backed into the embedded class `{}`; an embedded id needs a host path the constructor argument cannot supply",
                                classes[tc].name
                            ),
                        ));
                    }
                }
                Some("intrusive_list") => {
                    if !classes[sc].owns_records {
                        return Err(syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!(
                                "arrow `{named}` is sourced at the root-marked entity `{}`, which is unfused; the `intrusive_list` backing needs a head field on the self record, and an unfused root owns none (it *is* the store)",
                                members[i].2.entities[ent_index(i, &d.source)?].name
                            ),
                        ));
                    }
                    if let ClassLoc::Embedded(_) = classes[sc].loc {
                        return Err(syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!(
                                "arrow `{named}` is `intrusive_list`-backed and sourced at the embedded class `{}` (frontier limit: intrusive arrows sourced at embedded classes)",
                                classes[sc].name
                            ),
                        ));
                    }
                }
                Some("hash_index") => {
                    // Hook-maintained derived lookup state: exempt from the
                    // self-record home rule (a root-marked self contributes
                    // at store level; spike 11 pinned path-id values legal).
                }
                _ => {}
            }
        }
    }

    // ---- access-path helpers ----
    // Record-data reference through a HANDLE binding.
    let rec_ref_handle = |ci: usize, h: TokenStream2| -> TokenStream2 {
        match classes[ci].loc {
            ClassLoc::Root => {
                let a = &classes[ci].arena_fld;
                quote!(#h.store.#a[(#h).id.0 as usize])
            }
            ClassLoc::Embedded(host) => {
                let ha = &classes[host].arena_fld;
                let a = &classes[ci].arena_fld;
                quote!(#h.store.#ha[(#h).host.0 as usize].#a[(#h).id as usize])
            }
        }
    };

    let mut out = TokenStream2::new();

    // ---- id types ----
    for ci in 0..classes.len() {
        if !classes[ci].owns_records {
            continue;
        }
        let idt = &classes[ci].id_name;
        let doc_name = &classes[ci].name;
        let def = match classes[ci].loc {
            ClassLoc::Root => quote!(pub struct #idt(pub u32);),
            ClassLoc::Embedded(host) => {
                let hid = &classes[host].id_name;
                quote!(pub struct #idt(pub #hid, pub u32);)
            }
        };
        out.extend(quote! {
            #[doc = concat!("Index of a `", stringify!(#doc_name), "` in its arena (a host-anchored path when embedded).")]
            #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
            #def
        });
    }

    // ---- record data structs (embedded classes first: hosts hold their Data) ----
    let mut order: Vec<usize> = (0..classes.len()).filter(|&ci| classes[ci].owns_records).collect();
    order.sort_by_key(|&ci| match classes[ci].loc {
        ClassLoc::Embedded(_) => 0,
        ClassLoc::Root => 1,
    });
    for ci in order {
        // Field-backed one-arrow fields hold the id directly; intrusive
        // heads/links are Options (initialized empty, relinked by hooks).
        let mut fields = TokenStream2::new();
        for fc in &classes[ci].fields {
            let f = &fc.field;
            let ty = &fc.ty;
            if fc.arg.is_some() {
                fields.extend(quote!(#f: #ty,));
            } else {
                fields.extend(quote!(#f: ::core::option::Option<#ty>,));
            }
        }
        // Embedded arenas contributed by instances placed in this class,
        // and the class's hash_index maps.
        for d in classes.iter() {
            if let ClassLoc::Embedded(h) = d.loc {
                if h == ci {
                    let a = &d.arena_fld;
                    let dat = &d.data_name;
                    fields.extend(quote!(#a: ::std::vec::Vec<#dat>,));
                }
            }
        }
        for (f, k, v) in &classes[ci].maps {
            fields.extend(quote!(#f: ::std::collections::HashMap<#k, #v>,));
        }
        let dat = &classes[ci].data_name;
        let doc_name = &classes[ci].name;
        out.extend(quote! {
            #[doc = concat!("Record of a `", stringify!(#doc_name), "` (fusion class; fields sum every member's contributions).")]
            #[derive(Debug)]
            struct #dat { #fields }
        });
    }

    // ---- store struct ----
    let root_arenas: Vec<TokenStream2> = classes
        .iter()
        .filter(|cl| cl.owns_records && cl.loc == ClassLoc::Root)
        .map(|cl| {
            let a = &cl.arena_fld;
            let dat = &cl.data_name;
            quote!(#a: ::std::vec::Vec<#dat>,)
        })
        .collect();
    // Store-level hash_index maps: a keyed arrow whose self is an unfused
    // root contributes its map here (exactly one self per store, no record
    // to hold it — the store-level contribution site). The field name rides
    // in `root_map_of` so the hooks and read impls name the same field;
    // colliding arrow names across instances take the instance prefix.
    let mut root_map_of: HashMap<(usize, String), Ident2> = HashMap::new();
    {
        let mut used: Vec<String> = Vec::new();
        for i in 0..n_inst {
            for info in &inst_infos[i] {
                if info.backing.as_deref() != Some("hash_index") {
                    continue;
                }
                let d = info.decl;
                let sc = class_index_of(i, ent_index(i, &d.source)?);
                if classes[sc].owns_records {
                    continue; // per-self-object maps ride on the record
                }
                let base = format_ident!("{}_index", d.name);
                let f = if used.contains(&base.to_string()) {
                    format_ident!("{}_{}", snake(&members[i].0.to_string()), base)
                } else {
                    base
                };
                used.push(f.to_string());
                root_map_of.insert((i, d.name.to_string()), f);
            }
        }
    }
    let mut root_maps: Vec<TokenStream2> = Vec::new();
    for i in 0..n_inst {
        for info in &inst_infos[i] {
            let d = info.decl;
            let Some(f) = root_map_of.get(&(i, d.name.to_string())) else {
                continue;
            };
            let kid = classes[class_index_of(i, ent_index(i, d.key.as_ref().unwrap())?)]
                .id_name
                .clone();
            let vt = classes[class_index_of(i, ent_index(i, &d.target)?)].id_name.clone();
            root_maps.push(quote!(#f: ::std::collections::HashMap<#kid, #vt>,));
        }
    }
    let store_pattern_name = config
        .pattern_path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default();
    out.extend(quote! {
        #[doc = concat!("Store generated by `implement!` for the composed pattern `", #store_pattern_name, "`.")]
        #[derive(Default, Debug)]
        pub struct #store {
            #(#root_arenas)*
            #(#root_maps)*
        }

        impl #store {
            #[doc = "Empty store."]
            pub fn new() -> Self {
                Self::default()
            }
        }
    });

    // ---- constructors, hooks, write ops, counts, root accessors ----
    let mut store_impl = TokenStream2::new();
    let mut op_names: Vec<String> = Vec::new(); // generated-name uniqueness
    let mut iter_names_used: Vec<String> = Vec::new();

    // Cross-instance arrow-name collision counts (iterators and write ops).
    let arrow_name_users: HashMap<String, usize> = {
        let mut m: HashMap<String, usize> = HashMap::new();
        for (_, _, b) in members.iter() {
            for a in &b.arrows {
                *m.entry(a.name.to_string()).or_insert(0) += 1;
            }
        }
        m
    };
    let iter_name = |i: usize, a: &Ident2| -> Ident2 {
        let base = camel(&a.to_string());
        if arrow_name_users[&a.to_string()] > 1 {
            format_ident!("{}{}Iter", camel(&members[i].0.to_string()), base)
        } else {
            format_ident!("{}Iter", base)
        }
    };

    for ci in 0..classes.len() {
        if !classes[ci].owns_records {
            continue;
        }
        let cl = &classes[ci];
        let add_name = format_ident!("add_{}", snake(&cl.name.to_string()));
        op_names.push(add_name.to_string());
        let dat = &cl.data_name;
        let arena = &cl.arena_fld;
        let idt = &cl.id_name;
        // Host argument for embedded classes comes first.
        let (host_arg_def, host_arg_use): (TokenStream2, Option<Ident2>) = match cl.loc {
            ClassLoc::Root => (quote!(), None),
            ClassLoc::Embedded(host) => {
                let hid = &classes[host].id_name;
                (
                    quote!(host: #hid,),
                    Some(Ident2::new("host", proc_macro2::Span::call_site())),
                )
            }
        };
        // Merged ctor args: the class's field contributions with args, in
        // contribution order (composition-instance order, then member
        // schema order) — the combined rule was applied at collection.
        let arg_defs = classes[ci].fields
            .iter()
            .filter(|fc| fc.arg.is_some())
            .map(|fc| {
                let a = fc.arg.as_ref().unwrap();
                let ty = &fc.ty;
                quote!(#a: #ty,)
            });
        // Record literal: field args move in; intrusive links start empty
        // (the hooks relink on insert); embedded arenas start empty.
        let mut record_fields = TokenStream2::new();
        for fc in &classes[ci].fields {
            let f = &fc.field;
            match &fc.arg {
                Some(a) => record_fields.extend(quote!(#f: #a,)),
                None => record_fields.extend(quote!(#f: ::core::option::Option::None,)),
            }
        }
        for d in classes.iter() {
            if let ClassLoc::Embedded(h) = d.loc {
                if h == ci {
                    let a = &d.arena_fld;
                    record_fields.extend(quote!(#a: ::std::vec::Vec::new(),));
                }
            }
        }
        // hash_index maps on this class's record start empty; inserts go
        // through the keyed hook.
        for (f, _, _) in &classes[ci].maps {
            record_fields.extend(quote!(#f: ::std::collections::HashMap::new(),));
        }
        // `by`-derived hooks firing at construction of this class: the
        // determinant `one` arrow originates at a member entity.
        let mut hook_calls = TokenStream2::new();
        for &(i, e) in &cl.members {
            for info in &inst_infos[i] {
                let d = info.decl;
                let Some(det_name) = &d.by else { continue };
                if info.backing.as_deref() != Some("intrusive_list") {
                    continue;
                }
                let det = members[i]
                    .2
                    .arrows
                    .iter()
                    .find(|x| x.name == *det_name)
                    .expect("pattern! validated the determinant");
                if det.source != members[i].2.entities[e].name {
                    continue;
                }
                let hook = format_ident!("__insert_{}_{}", snake(&members[i].0.to_string()), d.name);
                let det_field = field_of_arrow
                    .get(&(i, det.name.to_string()))
                    .expect("determinant is field-backed (total one arrow)");
                hook_calls.extend(quote!(self.#hook(#det_field, id);));
            }
        }
        // Id binding + push.
        let (id_bind, push): (TokenStream2, TokenStream2) = match cl.loc {
            ClassLoc::Root => (
                quote!(let id = #idt(self.#arena.len() as u32);),
                quote!(self.#arena.push(#dat { #record_fields });),
            ),
            ClassLoc::Embedded(host) => {
                let ha = &classes[host].arena_fld;
                let hostv = host_arg_use.clone().unwrap();
                (
                    quote!(let id = #idt(#hostv, self.#ha[#hostv.0 as usize].#arena.len() as u32);),
                    quote!(self.#ha[#hostv.0 as usize].#arena.push(#dat { #record_fields });),
                )
            }
        };
        let ctor_doc = format!(
            "Constructor for `{}`: the host record (when embedded), then one argument per total `one` arrow of its fusion class, then backing insert hooks.",
            cl.name
        );
        store_impl.extend(quote! {
            #[doc = #ctor_doc]
            pub fn #add_name(&mut self, #host_arg_def #(#arg_defs)*) -> #idt {
                #id_bind
                #push
                #hook_calls
                id
            }
        });

        // Count fn: global at root level, per-host when embedded.
        let count_name = format_ident!("{}_count", snake(&cl.name.to_string()));
        op_names.push(count_name.to_string());
        match cl.loc {
            ClassLoc::Root => store_impl.extend(quote! {
                #[doc = concat!("Number of `", stringify!(#idt), " records.")]
                pub fn #count_name(&self) -> usize {
                    self.#arena.len()
                }
            }),
            ClassLoc::Embedded(host) => {
                let hid = &classes[host].id_name;
                let ha = &classes[host].arena_fld;
                store_impl.extend(quote! {
                    #[doc = concat!("Number of `", stringify!(#idt), "` records embedded in `host`'s record.")]
                    pub fn #count_name(&self, host: #hid) -> usize {
                        self.#ha[host.0 as usize].#arena.len()
                    }
                });
            }
        }
    }

    // Intrusive hooks + primitive insert ops; field-write hooks + set ops.
    // Hook names are always instance-qualified (internal); public op names
    // take the instance prefix only on cross-instance arrow-name collision.
    for (i, infos) in inst_infos.iter().enumerate() {
        let inst_snake = snake(&members[i].0.to_string());
        let inst_name = &members[i].0;
        for info in infos {
            let d = info.decl;
            let arrow_users = arrow_name_users[&d.name.to_string()];
            match info.backing.as_deref() {
                Some("intrusive_list") => {
                    let hook = format_ident!("__insert_{}_{}", inst_snake, d.name);
                    let head = format_ident!("first_{}", crate::grammar::link_stem(&d.name));
                    let next = format_ident!("next_{}", crate::grammar::link_stem(&d.name));
                    // Intra-class collisions were prefixed at collection;
                    // find the actual field names in the self/target class.
                    let sc = class_index_of(i, ent_index(i, &d.source)?);
                    let tc = class_index_of(i, ent_index(i, &d.target)?);
                    let head_field = classes[sc].fields
                        .iter()
                        .find(|fc| {
                            fc.field == head
                                || fc.field
                                    == format_ident!("{}_{}", inst_snake, head)
                        })
                        .map(|fc| fc.field.clone())
                        .unwrap_or(head.clone());
                    let next_field = classes[tc].fields
                        .iter()
                        .find(|fc| {
                            fc.field == next
                                || fc.field
                                    == format_ident!("{}_{}", inst_snake, next)
                        })
                        .map(|fc| fc.field.clone())
                        .unwrap_or(next.clone());
                    let sid = classes[sc].id_name.clone();
                    let tid = classes[tc].id_name.clone();
                    let (sref, tref) = (
                        rec_ref_ts(&classes, sc, quote!(self_id)),
                        rec_ref_ts(&classes, tc, quote!(member)),
                    );
                    let arrow_doc = &d.name;
                    store_impl.extend(quote! {
                        #[doc(hidden)]
                        #[doc = concat!("intrusive_list insert hook of arrow `", stringify!(#arrow_doc), "` (instance ", stringify!(#inst_name), "): push `member` onto the front of `self_id`'s list")]
                        fn #hook(&mut self, self_id: #sid, member: #tid) {
                            let next = #sref.#head_field;
                            #tref.#next_field = next;
                            #sref.#head_field = ::core::option::Option::Some(member);
                        }
                    });
                    // Primitive membership (no `by`): explicit insert op.
                    if d.by.is_none() {
                        let op = if arrow_users > 1 {
                            format_ident!("insert_{}_{}", inst_snake, d.name)
                        } else {
                            format_ident!("insert_{}", d.name)
                        };
                        op_names.push(op.to_string());
                        store_impl.extend(quote! {
                            #[doc = concat!("Insert `member` into `self_id`'s `", stringify!(#arrow_doc), "` list (primitive membership; push-front).")]
                            pub fn #op(&mut self, self_id: #sid, member: #tid) {
                                self.#hook(self_id, member);
                            }
                        });
                    }
                }
                Some("field") => {
                    let sc = class_index_of(i, ent_index(i, &d.source)?);
                    let sid = classes[sc].id_name.clone();
                    let tid = id_ty_of(i, ent_index(i, &d.target)?);
                    let field = field_of_arrow
                        .get(&(i, d.name.to_string()))
                        .expect("field contributions collected")
                        .clone();
                    let hook = format_ident!("__set_{}_{}", inst_snake, d.name);
                    let sref = rec_ref_ts(&classes, sc, quote!(self_id));
                    let arrow_doc = &d.name;
                    store_impl.extend(quote! {
                        #[doc(hidden)]
                        #[doc = concat!("field-write hook of arrow `", stringify!(#arrow_doc), "` (instance ", stringify!(#inst_name), ")")]
                        fn #hook(&mut self, self_id: #sid, value: #tid) {
                            #sref.#field = value;
                        }
                    });
                    let op = if arrow_users > 1 {
                        format_ident!("set_{}_{}", inst_snake, d.name)
                    } else {
                        format_ident!("set_{}", d.name)
                    };
                    op_names.push(op.to_string());
                    store_impl.extend(quote! {
                        #[doc = concat!("Set `self_id`'s `", stringify!(#arrow_doc), "` to `value` (field-backed `one` arrow).")]
                        pub fn #op(&mut self, self_id: #sid, value: #tid) {
                            self.#hook(self_id, value);
                        }
                    });
                }
                Some("hash_index") => {
                    let sc = class_index_of(i, ent_index(i, &d.source)?);
                    let kid = id_ty_of(i, ent_index(i, d.key.as_ref().unwrap())?);
                    let vt = id_ty_of(i, ent_index(i, &d.target)?);
                    let arrow_doc = &d.name;
                    let pred = format_ident!("__{}_{}_key_live", inst_snake, d.name);
                    let hook = format_ident!("__insert_{}_{}", inst_snake, d.name);
                    let op = if arrow_users > 1 {
                        format_ident!("insert_{}_{}", inst_snake, d.name)
                    } else {
                        format_ident!("insert_{}", d.name)
                    };
                    op_names.push(op.to_string());
                    if !classes[sc].owns_records {
                        // Unfused-root self: exactly one self per store and
                        // no id to pass; the map is store-level and the op
                        // omits the self argument (the review probe's shape).
                        let map = root_map_of[&(i, d.name.to_string())].clone();
                        store_impl.extend(quote! {
                            #[doc(hidden)]
                            #[doc = concat!("live-key predicate of arrow `", stringify!(#arrow_doc), "` (instance ", stringify!(#inst_name), "; store-level map)")]
                            fn #pred(&self, key: #kid) -> bool {
                                self.#map.contains_key(&key)
                            }
                            #[doc(hidden)]
                            #[doc = concat!("hash_index insert hook of arrow `", stringify!(#arrow_doc), "` (instance ", stringify!(#inst_name), "; store-level map)")]
                            fn #hook(&mut self, key: #kid, target: #vt) {
                                if self.#pred(key) {
                                    ::core::panic!("duplicate key {:?} for arrow `{}`: a keyed arrow is a function", key, stringify!(#arrow_doc));
                                }
                                self.#map.insert(key, target);
                            }
                            #[doc = concat!("Map `key` to `target` in the `", stringify!(#arrow_doc), "` index (root-sourced keyed arrow; no self id).")]
                            pub fn #op(&mut self, key: #kid, target: #vt) {
                                self.#hook(key, target);
                            }
                        });
                    } else {
                        // Per-self-object map on the fused record; the
                        // insert hook takes `&mut` store state and addresses
                        // through the nesting (the arena append and the map
                        // insert cannot share one `&mut` host-record borrow).
                        let sid = classes[sc].id_name.clone();
                        let map = classes[sc]
                            .maps
                            .iter()
                            .find(|(f, k, v)| {
                                *k == kid && *v == vt
                                    && f.to_string().starts_with(&format!("{}_index", d.name))
                            })
                            .map(|(f, _, _)| f.clone())
                            .unwrap_or_else(|| format_ident!("{}_index", d.name));
                        let sref = rec_ref_ts(&classes, sc, quote!(self_id));
                        store_impl.extend(quote! {
                            #[doc(hidden)]
                            #[doc = concat!("live-key predicate of arrow `", stringify!(#arrow_doc), "` (instance ", stringify!(#inst_name), ") on one self object")]
                            fn #pred(&self, self_id: #sid, key: #kid) -> bool {
                                #sref.#map.contains_key(&key)
                            }
                            #[doc(hidden)]
                            #[doc = concat!("hash_index insert hook of arrow `", stringify!(#arrow_doc), "` (instance ", stringify!(#inst_name), ")")]
                            fn #hook(&mut self, self_id: #sid, key: #kid, target: #vt) {
                                if self.#pred(self_id, key) {
                                    ::core::panic!("duplicate key {:?} for arrow `{}` on {:?}: a keyed arrow is a function", key, stringify!(#arrow_doc), self_id);
                                }
                                #sref.#map.insert(key, target);
                            }
                            #[doc = concat!("Map `key` to `target` in `self_id`'s `", stringify!(#arrow_doc), "` index.")]
                            pub fn #op(&mut self, self_id: #sid, key: #kid, target: #vt) {
                                self.#hook(self_id, key, target);
                            }
                        });
                    }
                }
                _ => {}
            }
        }
    }

    // The builder's read view: id-based getters on the store, taking and
    // returning ids and copies — never handles (no handle can coexist with
    // the `&mut` builder). One field-read getter per bound `one` arrow
    // (named after the arrow whose field it reads; instance-prefixed on
    // cross-instance arrow-name collision), plus a cross-boundary form
    // `<a>_<b>` for each `<->` closure: `a` is the read-through arrow
    // targeting the open entity, `b` the underlying read on the partner
    // instance (the probe's `main_edge_source(me)` spelling). One keyed
    // lookup per keyed arrow (`<arrow>_by_key`). The names are contract:
    // user-written constructor logic (induction, dedup) reads through them.
    for (i, infos) in inst_infos.iter().enumerate() {
        let inst_snake = snake(&members[i].0.to_string());
        let inst_name = &members[i].0;
        for info in infos {
            let d = info.decl;
            match info.backing.as_deref() {
                Some("field") => {
                    let sc = class_index_of(i, ent_index(i, &d.source)?);
                    let sid = classes[sc].id_name.clone();
                    let tid = id_ty_of(i, ent_index(i, &d.target)?);
                    let field = field_of_arrow[&(i, d.name.to_string())].clone();
                    let getter = if arrow_name_users[&d.name.to_string()] > 1 {
                        format_ident!("{}_{}", inst_snake, d.name)
                    } else {
                        d.name.clone()
                    };
                    op_names.push(getter.to_string());
                    let sref = rec_ref_ts(&classes, sc, quote!(self_id));
                    let arrow_doc = &d.name;
                    store_impl.extend(quote! {
                        #[doc = concat!("Builder read view: the `", stringify!(#arrow_doc), "` field of `self_id` (instance ", stringify!(#inst_name), ").")]
                        pub fn #getter(&self, self_id: #sid) -> #tid {
                            #sref.#field
                        }
                    });
                }
                Some("hash_index") => {
                    let sc = class_index_of(i, ent_index(i, &d.source)?);
                    let kid = id_ty_of(i, ent_index(i, d.key.as_ref().unwrap())?);
                    let vt = id_ty_of(i, ent_index(i, &d.target)?);
                    let getter = if arrow_name_users[&d.name.to_string()] > 1 {
                        format_ident!("{}_{}_by_key", inst_snake, d.name)
                    } else {
                        format_ident!("{}_by_key", d.name)
                    };
                    op_names.push(getter.to_string());
                    let arrow_doc = &d.name;
                    if !classes[sc].owns_records {
                        let map = root_map_of[&(i, d.name.to_string())].clone();
                        store_impl.extend(quote! {
                            #[doc = concat!("Builder read view: `", stringify!(#arrow_doc), "` keyed lookup (root-sourced; no self id).")]
                            pub fn #getter(&self, key: #kid) -> ::core::option::Option<#vt> {
                                self.#map.get(&key).copied()
                            }
                        });
                    } else {
                        let sid = classes[sc].id_name.clone();
                        let map = classes[sc]
                            .maps
                            .iter()
                            .find(|(f, _, _)| {
                                let want = format_ident!("{}_index", d.name);
                                let prefixed =
                                    format_ident!("{}_{}", inst_snake, want);
                                *f == want || *f == prefixed
                            })
                            .map(|(f, _, _)| f.clone())
                            .expect("hash_index map contributed at collection");
                        let sref = rec_ref_ts(&classes, sc, quote!(self_id));
                        store_impl.extend(quote! {
                            #[doc = concat!("Builder read view: `", stringify!(#arrow_doc), "` keyed lookup on one self object.")]
                            pub fn #getter(&self, self_id: #sid, key: #kid) -> ::core::option::Option<#vt> {
                                #sref.#map.get(&key).copied()
                            }
                        });
                    }
                }
                _ => {}
            }
        }
    }
    // Cross-boundary field reads: for each `<->` closure (open X.O, partner
    // Y.E), each X-arrow `a` targeting O pairs with each bound field-backed
    // `one` arrow `b` sourced at E in Y: `<a>_<b>(id) -> target`, reading
    // b's field where the caller's id arrived through a's closure.
    for f in &c.fusions {
        if f.op != crate::grammar::FusionOp::Ref {
            continue;
        }
        // (open instance, open entity) and (closed instance, closed entity).
        let (xi, oi, yi, ei) = {
            let li = members.iter().position(|(m, _, _)| *m == f.lhs.0).unwrap();
            let ri = members.iter().position(|(m, _, _)| *m == f.rhs.0).unwrap();
            let le = members[li].2.entities.iter().position(|e| e.name == f.lhs.1).unwrap();
            let re = members[ri].2.entities.iter().position(|e| e.name == f.rhs.1).unwrap();
            if members[li].2.entities[le].open {
                (li, le, ri, re)
            } else {
                (ri, re, li, le)
            }
        };
        // Arrows of X targeting the open entity.
        for xa in members[xi].2.arrows.iter().filter(|a| {
            members[xi]
                .2
                .entities
                .iter()
                .position(|e| e.name == a.target)
                == Some(oi)
        }) {
            // Bound field-backed one arrows of Y sourced at the closed entity.
            for yb in members[yi].2.arrows.iter().filter(|b| {
                members[yi]
                    .2
                    .entities
                    .iter()
                    .position(|e| e.name == b.source)
                    == Some(ei)
            }) {
                let yinfo = inst_infos[yi]
                    .iter()
                    .find(|inf| inf.decl.name == yb.name)
                    .expect("every arrow resolved");
                if yinfo.backing.as_deref() != Some("field") {
                    continue;
                }
                let getter = format_ident!("{}_{}", xa.name, yb.name);
                op_names.push(getter.to_string());
                let sid = classes[class_index_of(yi, ei)].id_name.clone();
                let tid = id_ty_of(yi, ent_index(yi, &yb.target)?);
                let sc = class_index_of(yi, ei);
                let field = field_of_arrow[&(yi, yb.name.to_string())].clone();
                let sref = rec_ref_ts(&classes, sc, quote!(self_id));
                let through = &xa.name;
                let read = &yb.name;
                store_impl.extend(quote! {
                    #[doc = concat!("Builder read view: the `", stringify!(#read), "` field, where the id arrived through the `", stringify!(#through), "` reference closure.")]
                    pub fn #getter(&self, self_id: #sid) -> #tid {
                        #sref.#field
                    }
                });
            }
        }
    }

    // Root accessors: one per unfused root entity, instance-prefixed.
    for ci in 0..classes.len() {
        if classes[ci].owns_records || !classes[ci].has_root {
            continue;
        }
        let (i, e) = classes[ci].members[0];
        let acc = format_ident!(
            "{}_{}",
            snake(&members[i].0.to_string()),
            snake(&members[i].2.entities[e].name.to_string())
        );
        op_names.push(acc.to_string());
        let h = &classes[ci].name;
        store_impl.extend(quote! {
            #[doc = concat!("Root handle: the \"object\" view of this store for the unfused root `", stringify!(#h), "`.")]
            pub fn #acc(&self) -> #h<'_> {
                #h { store: self }
            }
        });
    }

    // validate() boundary (step 12 fills the canonical inventory).
    store_impl.extend(quote! {
        #[doc = "Structural validation. The canonical per-backing and keyed checks land with milestone-2 step 12; M4's flush calls this unconditionally."]
        pub fn validate(&self) -> ::core::result::Result<(), ::std::string::String> {
            ::core::result::Result::Ok(())
        }
    });

    out.extend(quote! {
        impl #store {
            #store_impl
        }
    });

    // ---- handles ----
    for ci in 0..classes.len() {
        let cl = &classes[ci];
        let h = &cl.name;
        let (body, id_fn, eq_body, hash_body, dbg_body) = if !cl.owns_records {
            (
                quote!(store: &'g #store),
                quote!(),
                quote!(::std::ptr::eq(self.store, other.store)),
                quote!((self.store as *const #store).hash(state);),
                {
                    quote!(f.debug_tuple(stringify!(#h)).finish())
                },
            )
        } else {
            match cl.loc {
                ClassLoc::Root => {
                    let idt = &cl.id_name;
                    (
                        quote!(store: &'g #store, id: #idt),
                        quote! {
                            #[doc = "Index of this entity in its arena."]
                            pub fn id(self) -> #idt {
                                self.id
                            }
                        },
                        quote!(self.id == other.id && ::std::ptr::eq(self.store, other.store)),
                        quote! {
                            (self.store as *const #store).hash(state);
                            self.id.hash(state);
                        },
                        quote!(write!(f, "{}({:?})", stringify!(#h), self.id)),
                    )
                }
                ClassLoc::Embedded(host) => {
                    let hid = &classes[host].id_name;
                    let idt = &cl.id_name;
                    (
                        quote!(store: &'g #store, host: #hid, id: u32),
                        quote! {
                            #[doc = "Index path of this entity: host record plus index in the embedded arena."]
                            pub fn id(self) -> #idt {
                                #idt(self.host, self.id)
                            }
                        },
                        quote!(
                            self.host == other.host
                                && self.id == other.id
                                && ::std::ptr::eq(self.store, other.store)
                        ),
                        quote! {
                            (self.store as *const #store).hash(state);
                            self.host.hash(state);
                            self.id.hash(state);
                        },
                        quote!(write!(f, "{}({:?}, {:?})", stringify!(#h), self.host, self.id)),
                    )
                }
            }
        };
        out.extend(quote! {
            #[doc = concat!("Handle on a `", stringify!(#h), "`.")]
            #[derive(Clone, Copy)]
            pub struct #h<'g> {
                #body
            }

            impl<'g> #h<'g> {
                #id_fn
            }

            // Equal only if pointing into the same store at the same index
            // path; a derive would bound `Store: Eq`/`Hash`, and the id
            // alone would equate handles across stores.
            impl<'g> ::core::cmp::PartialEq for #h<'g> {
                fn eq(&self, other: &Self) -> bool {
                    #eq_body
                }
            }
            impl<'g> ::core::cmp::Eq for #h<'g> {}
            impl<'g> ::core::hash::Hash for #h<'g> {
                fn hash<H: ::core::hash::Hasher>(&self, state: &mut H) {
                    #hash_body
                }
            }
            impl<'g> ::core::fmt::Debug for #h<'g> {
                fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                    #dbg_body
                }
            }
        });
    }

    // ---- iterators + trait impls ----
    let mut marker_base = config.pattern_path.clone();
    marker_base.segments.pop();
    let marker_base_ts = path_tokens(&marker_base);
    let marker_mod = Ident2::new(
        &snake(&config.pattern_path.segments.last().unwrap().ident.to_string()),
        proc_macro2::Span::call_site(),
    );

    let mut iter_defs = TokenStream2::new();
    let mut trait_impls = TokenStream2::new();

    for (i, infos) in inst_infos.iter().enumerate() {
        let inst_name = &members[i].0;
        for info in infos {
            let d = info.decl;
            let meta = info.meta;
            let arrow_doc_name = &d.name;
            let marker = marker_ty(&d.name);
            let marker_path = quote!(#marker_base_ts::#marker_mod::markers::#inst_name::#marker);
            let trait_path = {
                let pkg = &members[i].1;
                crate::resolve_schema_path(d.inst.as_ref().unwrap(), pkg)?
            };
            let sc = class_index_of(i, ent_index(i, &d.source)?);
            let tc = class_index_of(i, ent_index(i, &d.target)?);
            let self_h = &classes[sc].name;
            let tgt_h = &classes[tc].name;
            let m = &meta.method;
            let assoc = &meta.assoc;
            let it = iter_name(i, &d.name);
            iter_names_used.push(it.to_string());
            let tid = classes[tc].id_name.clone();

            // Derived (`:= sum`) arrows chain their siblings' iterators.
            if let Some((ua, ub)) = &d.sum {
                let da = members[i].2.arrows.iter().find(|x| x.name == *ua).ok_or_else(|| {
                    syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!("sum names unknown sibling `{ua}`"),
                    )
                })?;
                let db = members[i].2.arrows.iter().find(|x| x.name == *ub).ok_or_else(|| {
                    syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!("sum names unknown sibling `{ub}`"),
                    )
                })?;
                let ita = iter_name(i, &da.name);
                let itb = iter_name(i, &db.name);
                let ea = sum_sibling_expr(i, da, &config.bindings, &members, &classes, &class_index_of, &ent_index, quote!(self))?;
                let eb = sum_sibling_expr(i, db, &config.bindings, &members, &classes, &class_index_of, &ent_index, quote!(self))?;
                let gat = meta.gat.as_ref().unwrap();
                iter_defs.extend(quote! {
                    #[doc = concat!("Iterator of derived arrow `", stringify!(#arrow_doc_name), "` (instance ", stringify!(#inst_name), "): its siblings chained, self-loops twice.")]
                    pub struct #it<'g>(::core::iter::Chain<#ita<'g>, #itb<'g>>);
                    impl<'g> ::core::iter::Iterator for #it<'g> {
                        type Item = #tgt_h<'g>;
                        fn next(&mut self) -> ::core::option::Option<#tgt_h<'g>> {
                            self.0.next()
                        }
                    }
                });
                trait_impls.extend(quote! {
                    impl<'g> #trait_path<#marker_path> for #self_h<'g> {
                        type #assoc = #tgt_h<'g>;
                        type #gat<'a> = #it<'g> where Self: 'a;
                        fn #m(&self) -> #it<'g> {
                            #it((#ea).chain(#eb))
                        }
                    }
                });
                continue;
            }

            match info.backing.as_deref() {
                Some("arena") => match classes[tc].loc {
                    ClassLoc::Root => {
                        let ta = &classes[tc].arena_fld;
                        let gat = meta.gat.as_ref().unwrap();
                        iter_defs.extend(quote! {
                            #[doc = concat!("Iterator of arena-backed arrow `", stringify!(#arrow_doc_name), "' (instance ", stringify!(#inst_name), "): index range over the target arena.")]
                            pub struct #it<'g> {
                                store: &'g #store,
                                next: u32,
                                len: u32,
                            }
                            impl<'g> ::core::iter::Iterator for #it<'g> {
                                type Item = #tgt_h<'g>;
                                fn next(&mut self) -> ::core::option::Option<#tgt_h<'g>> {
                                    (self.next < self.len).then(|| {
                                        let n = #tgt_h { store: self.store, id: #tid(self.next) };
                                        self.next += 1;
                                        n
                                    })
                                }
                                fn size_hint(&self) -> (usize, ::core::option::Option<usize>) {
                                    let remaining = (self.len - self.next) as usize;
                                    (remaining, ::core::option::Option::Some(remaining))
                                }
                            }
                        });
                        let len_expr = quote!(self.store.#ta.len() as u32);
                        trait_impls.extend(quote! {
                            impl<'g> #trait_path<#marker_path> for #self_h<'g> {
                                type #assoc = #tgt_h<'g>;
                                type #gat<'a> = #it<'g> where Self: 'a;
                                fn #m(&self) -> #it<'g> {
                                    #it { store: self.store, next: 0, len: #len_expr }
                                }
                            }
                        });
                    }
                    ClassLoc::Embedded(host) => {
                        if sc != host {
                            return Err(syn::Error::new(
                                proc_macro2::Span::call_site(),
                                format!(
                                    "arena arrow `{inst_name}.{}` reads into the embedded class `{}`, which is hosted by another class (frontier: arena arrows into an embedded class hosted by another class)",
                                    d.name, classes[tc].name
                                ),
                            ));
                        }
                        let ha = &classes[host].arena_fld;
                        let ta = &classes[tc].arena_fld;
                        let host_id = &classes[host].id_name;
                        let gat = meta.gat.as_ref().unwrap();
                        iter_defs.extend(quote! {
                            #[doc = concat!("Iterator of arena-backed arrow `", stringify!(#arrow_doc_name), "' (instance ", stringify!(#inst_name), "): index range over the host record's embedded arena, yielding nested handles.")]
                            pub struct #it<'g> {
                                store: &'g #store,
                                host: #host_id,
                                next: u32,
                                len: u32,
                            }
                            impl<'g> ::core::iter::Iterator for #it<'g> {
                                type Item = #tgt_h<'g>;
                                fn next(&mut self) -> ::core::option::Option<#tgt_h<'g>> {
                                    (self.next < self.len).then(|| {
                                        let n = #tgt_h { store: self.store, host: self.host, id: self.next };
                                        self.next += 1;
                                        n
                                    })
                                }
                                fn size_hint(&self) -> (usize, ::core::option::Option<usize>) {
                                    let remaining = (self.len - self.next) as usize;
                                    (remaining, ::core::option::Option::Some(remaining))
                                }
                            }
                        });
                        trait_impls.extend(quote! {
                            impl<'g> #trait_path<#marker_path> for #self_h<'g> {
                                type #assoc = #tgt_h<'g>;
                                type #gat<'a> = #it<'g> where Self: 'a;
                                fn #m(&self) -> #it<'g> {
                                    #it {
                                        store: self.store,
                                        host: self.id,
                                        next: 0,
                                        len: self.store.#ha[self.id.0 as usize].#ta.len() as u32,
                                    }
                                }
                            }
                        });
                    }
                },
                Some("intrusive_list") => {
                    let sc_ref = rec_ref_handle(sc, quote!(self));
                    let head_field = {
                        let head = format_ident!("first_{}", crate::grammar::link_stem(&d.name));
                        let inst_snake = snake(&members[i].0.to_string());
                        classes[sc].fields
                            .iter()
                            .find(|fc| fc.field == head || fc.field == format_ident!("{}_{}", inst_snake, head))
                            .map(|fc| fc.field.clone())
                            .unwrap_or(head)
                    };
                    match classes[tc].loc {
                        ClassLoc::Root => {
                            let next_field = {
                                let next = format_ident!("next_{}", crate::grammar::link_stem(&d.name));
                                let inst_snake = snake(&members[i].0.to_string());
                                classes[tc].fields
                                    .iter()
                                    .find(|fc| fc.field == next || fc.field == format_ident!("{}_{}", inst_snake, next))
                                    .map(|fc| fc.field.clone())
                                    .unwrap_or(next)
                            };
                            let ta = &classes[tc].arena_fld;
                            let gat = meta.gat.as_ref().unwrap();
                            iter_defs.extend(quote! {
                                #[doc = concat!("Iterator of intrusive-backed arrow `", stringify!(#arrow_doc_name), "' (instance ", stringify!(#inst_name), "): linked-list walk, reverse insertion order.")]
                                pub struct #it<'g> {
                                    store: &'g #store,
                                    next: ::core::option::Option<#tid>,
                                }
                                impl<'g> ::core::iter::Iterator for #it<'g> {
                                    type Item = #tgt_h<'g>;
                                    fn next(&mut self) -> ::core::option::Option<#tgt_h<'g>> {
                                        let id = self.next?;
                                        let data = &self.store.#ta[id.0 as usize];
                                        self.next = data.#next_field;
                                        ::core::option::Option::Some(#tgt_h { store: self.store, id })
                                    }
                                }
                            });
                            trait_impls.extend(quote! {
                                impl<'g> #trait_path<#marker_path> for #self_h<'g> {
                                    type #assoc = #tgt_h<'g>;
                                    type #gat<'a> = #it<'g> where Self: 'a;
                                    fn #m(&self) -> #it<'g> {
                                        #it { store: self.store, next: #sc_ref.#head_field }
                                    }
                                }
                            });
                        }
                        ClassLoc::Embedded(host) => {
                            // The target's link field holds a path id; the
                            // walk addresses the embedded arena through it.
                            let next_field = {
                                let next = format_ident!("next_{}", crate::grammar::link_stem(&d.name));
                                let inst_snake = snake(&members[i].0.to_string());
                                classes[tc].fields
                                    .iter()
                                    .find(|fc| fc.field == next || fc.field == format_ident!("{}_{}", inst_snake, next))
                                    .map(|fc| fc.field.clone())
                                    .unwrap_or(next)
                            };
                            let ha = &classes[host].arena_fld;
                            let ta = &classes[tc].arena_fld;
                            let gat = meta.gat.as_ref().unwrap();
                            iter_defs.extend(quote! {
                                #[doc = concat!("Iterator of intrusive-backed arrow `", stringify!(#arrow_doc_name), "' (instance ", stringify!(#inst_name), ") into the embedded class `", stringify!(#tgt_h), "`: linked-list walk over path-id links.")]
                                pub struct #it<'g> {
                                    store: &'g #store,
                                    next: ::core::option::Option<#tid>,
                                }
                                impl<'g> ::core::iter::Iterator for #it<'g> {
                                    type Item = #tgt_h<'g>;
                                    fn next(&mut self) -> ::core::option::Option<#tgt_h<'g>> {
                                        let id = self.next?;
                                        let data = &self.store.#ha[id.0.0 as usize].#ta[id.1 as usize];
                                        self.next = data.#next_field;
                                        ::core::option::Option::Some(#tgt_h { store: self.store, host: id.0, id: id.1 })
                                    }
                                }
                            });
                            trait_impls.extend(quote! {
                                impl<'g> #trait_path<#marker_path> for #self_h<'g> {
                                    type #assoc = #tgt_h<'g>;
                                    type #gat<'a> = #it<'g> where Self: 'a;
                                    fn #m(&self) -> #it<'g> {
                                        #it { store: self.store, next: #sc_ref.#head_field }
                                    }
                                }
                            });
                        }
                    }
                }
                Some("field") => {
                    let field = field_of_arrow
                        .get(&(i, d.name.to_string()))
                        .expect("field contributions collected")
                        .clone();
                    let src_ref = rec_ref_handle(sc, quote!(self));
                    trait_impls.extend(quote! {
                        impl<'g> #trait_path<#marker_path> for #self_h<'g> {
                            type #assoc = #tgt_h<'g>;
                            fn #m(&self) -> #tgt_h<'g> {
                                #tgt_h {
                                    store: self.store,
                                    id: #src_ref.#field,
                                }
                            }
                        }
                    });
                }
                // Keyed partial: one hash lookup; the associated-type
                // bindings name the impl lifetime ('g — the E0637-pinned
                // emission), and a `<->`-closed key is the partner's plain
                // id (no wrapper newtype).
                Some("hash_index") => {
                    let kid = id_ty_of(i, ent_index(i, d.key.as_ref().unwrap())?);
                    let key_assoc = meta
                        .key
                        .as_ref()
                        .expect("keyed roots publish the key assoc name");
                    let lookup = if !classes[sc].owns_records {
                        let map = root_map_of[&(i, d.name.to_string())].clone();
                        quote!(self.store.#map.get(&key).copied())
                    } else {
                        let src_ref = rec_ref_handle(sc, quote!(self));
                        // The map field on the self record: name matches the
                        // arrow's `<arrow>_index` (instance-prefixed when it
                        // collided at collection).
                        let map = classes[sc]
                            .maps
                            .iter()
                            .find(|(f, _, _)| {
                                let want = format_ident!("{}_index", d.name);
                                let prefixed = format_ident!(
                                    "{}_{}",
                                    snake(&members[i].0.to_string()),
                                    want
                                );
                                *f == want || *f == prefixed
                            })
                            .map(|(f, _, _)| f.clone())
                            .expect("hash_index map contributed at collection");
                        quote!(#src_ref.#map.get(&key).copied())
                    };
                    let mk_handle = match classes[tc].loc {
                        ClassLoc::Root => quote!(#tgt_h { store: self.store, id }),
                        ClassLoc::Embedded(_) => quote!(#tgt_h { store: self.store, host: id.0, id: id.1 }),
                    };
                    trait_impls.extend(quote! {
                        impl<'g> #trait_path<#marker_path> for #self_h<'g> {
                            type #assoc = #tgt_h<'g>;
                            type #key_assoc = #kid;
                            fn #m(&self, key: #kid) -> ::core::option::Option<#tgt_h<'g>> {
                                #lookup.map(|id| #mk_handle)
                            }
                        }
                    });
                }
                other => {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!(
                            "backing `{}` is not implemented for arrow `{inst_name}.{}`",
                            other.unwrap_or("?"),
                            d.name
                        ),
                    ));
                }
            }
        }
    }

    // ---- generated-name uniqueness ----
    let mut seen_names: HashMap<String, String> = HashMap::new();
    let mut check_name = |kind: &str, name: &str| -> syn::Result<()> {
        if let Some(prev) = seen_names.get(name) {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "generated name `{name}` ({kind}) collides with `{name}` ({prev}); rename an entity, arrow, or instance",
                ),
            ));
        }
        seen_names.insert(name.to_string(), kind.to_string());
        Ok(())
    };
    for cl in &classes {
        if cl.owns_records {
            check_name("class type", &cl.name.to_string())?;
            check_name("id type", &cl.id_name.to_string())?;
            check_name("record type", &cl.data_name.to_string())?;
            check_name("arena field", &cl.arena_fld.to_string())?;
        }
    }
    for n in &op_names {
        check_name("store method", n)?;
    }
    for n in &iter_names_used {
        check_name("iterator type", n)?;
    }
    // Class type names also live in the same value/type space as handles:
    // handles share the class type name by design, so only cross-KIND
    // collisions are checked above.

    out.extend(iter_defs);
    out.extend(trait_impls);
    Ok(out)
}

/// Record-data access inside a hook (a `&mut self` store method), addressed
/// by an id *value*: `self.<arena>[id.0]` at root level, nested through the
/// host arena for path ids.
fn rec_ref_ts(classes: &[ClassData], ci: usize, idv: TokenStream2) -> TokenStream2 {
    match classes[ci].loc {
        ClassLoc::Root => {
            let a = &classes[ci].arena_fld;
            quote!(self.#a[(#idv).0 as usize])
        }
        ClassLoc::Embedded(host) => {
            let ha = &classes[host].arena_fld;
            let a = &classes[ci].arena_fld;
            quote!(self.#ha[(#idv).0.0 as usize].#a[(#idv).1 as usize])
        }
    }
}

/// Iterator-expression for a bound `many`-arrow sibling of a `:= sum` arrow,
/// read off a handle binding `h` — the shared shape of the arena and
/// intrusive iterator constructions, inlined so sums never depend on trait
/// imports.
#[allow(clippy::too_many_arguments)]
fn sum_sibling_expr(
    i: usize,
    d: &crate::grammar::ArrowDecl,
    bindings: &[Binding],
    members: &[(Ident2, String, crate::grammar::SchemaBody)],
    classes: &[ClassData],
    class_index_of: &dyn Fn(usize, usize) -> usize,
    ent_index: &dyn Fn(usize, &Ident2) -> syn::Result<usize>,
    h: TokenStream2,
) -> syn::Result<TokenStream2> {
    let inst_name = &members[i].0;
    let it = {
        let base = camel(&d.name.to_string());
        let users = members
            .iter()
            .filter(|(_, _, b)| b.arrows.iter().any(|a| a.name == d.name))
            .count();
        if users > 1 {
            format_ident!("{}{}Iter", camel(&inst_name.to_string()), base)
        } else {
            format_ident!("{}Iter", base)
        }
    };
    let backing = bindings
        .iter()
        .find(|b| b.instance.as_ref() == Some(inst_name) && b.arrow == d.name)
        .map(|b| b.backing.to_string());
    match backing.as_deref() {
        Some("arena") => {
            let tc = class_index_of(i, ent_index(i, &d.target)?);
            match classes[tc].loc {
                ClassLoc::Root => {
                    let ta = &classes[tc].arena_fld;
                    Ok(quote!(#it { store: #h.store, next: 0, len: #h.store.#ta.len() as u32 }))
                }
                ClassLoc::Embedded(host) => {
                    let sc = class_index_of(i, ent_index(i, &d.source)?);
                    if sc != host {
                        return Err(syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!(
                                "sum sibling `{}.{}` reads into an embedded class hosted by another class",
                                inst_name, d.name
                            ),
                        ));
                    }
                    let ha = &classes[host].arena_fld;
                    let ta = &classes[tc].arena_fld;
                    Ok(quote!(#it {
                        store: #h.store,
                        host: (#h).id,
                        next: 0,
                        len: #h.store.#ha[(#h).id.0 as usize].#ta.len() as u32,
                    }))
                }
            }
        }
        Some("intrusive_list") => {
            let sc = class_index_of(i, ent_index(i, &d.source)?);
            if !classes[sc].owns_records {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "sum sibling `{}.{}` is intrusive and sourced at an unfused root",
                        inst_name, d.name
                    ),
                ));
            }
            if let ClassLoc::Embedded(_) = classes[sc].loc {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "sum sibling `{}.{}` is intrusive and sourced at an embedded class (frontier limit)",
                        inst_name, d.name
                    ),
                ));
            }
            let head = format_ident!("first_{}", crate::grammar::link_stem(&d.name));
            let inst_snake = snake(&inst_name.to_string());
            let head_field = classes[sc]
                .fields
                .iter()
                .find(|fc| fc.field == head || fc.field == format_ident!("{}_{}", inst_snake, head))
                .map(|fc| fc.field.clone())
                .unwrap_or(head);
            let a = &classes[sc].arena_fld;
            Ok(quote!(#it {
                store: #h.store,
                next: #h.store.#a[(#h).id.0 as usize].#head_field,
            }))
        }
        _ => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "sum sibling `{}.{}` is not a bound `many` arrow",
                inst_name, d.name
            ),
        )),
    }
}
