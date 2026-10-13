//! Composed-store codegen — the codegen stage of `__implement_gen!` for a
//! composed pattern, under M2's placement machinery (union-find fusion
//! classes, `ClassLoc::Root/Embedded(host)`, the `<->` partner map,
//! collision prefixing) and M3's declaration-driven emission: every bound
//! arrow's storage is driven by its resolved `backing!` declaration —
//! contributed fields (renamed per the declaration's naming formulas),
//! hook bodies, the access protocols, and validate bodies are the
//! declaration's authored tokens, bound per placement by the
//! role-substitution engine ([`crate::roles`]). One authored body, per-
//! placement bindings; what stays uniform is M2's model:
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
//! - Backings bind per instance (`instance.arrow` keys); hooks and
//!   validators are instance-qualified (`__insert_<inst>_<arrow>`,
//!   `__validate_<inst>_<arrow>`), summoned in binding order.

use std::collections::HashMap;

use proc_macro2::{Ident as Ident2, Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};

use crate::backing::{formula_name, BackingDecl, Clause, OnTarget, ValidateVariant};
use crate::codegen::{body_of, card_of_decl, init_of, on_clause, on_clauses, scaffold_lets, FieldInit};
use crate::grammar::{
    camel, marker_ty, path_tokens, snake, Cardinality, ComposedSchema, RootMeta,
};
use crate::roles::{self, Ent, RoleBinding};
use crate::ChainConfig;

/// A resolved arrow of one member instance: declaration, root metadata,
/// backing declaration (`None` for derived `:= sum` arrows).
struct ArrowInfo<'a> {
    decl: &'a crate::grammar::ArrowDecl,
    meta: &'a RootMeta,
    backing: Option<&'a BackingDecl>,
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
    /// Field contributions to this class's record, in collection order
    /// (composition-instance order, then member schema order).
    fields: Vec<FieldContribution>,
}

/// One field contribution to a class's record (or, with no owner class, to
/// the store): the emitted name, the placement-rewritten type, how the
/// record literal initializes it, and what the validate pre-pass needs.
struct FieldContribution {
    /// Field name as spelled in the record (post collision-resolution).
    field: Ident2,
    /// Rewritten type (entity-variable id types through the partner map,
    /// container paths absolutized).
    ty: TokenStream2,
    /// The record-literal initializer (ctor arg / `None` / `Default`).
    init: FieldInit,
    /// The contributing instance (for qualified naming in docs/errors).
    inst: usize,
    /// The contributing arrow's local name.
    arrow: Ident2,
    /// `on A` (self-side: "head") vs `on B` (target-side: "link").
    side_a: bool,
    /// The type is `Option<id>` — the validate pre-pass range-walks it.
    option_id: bool,
    /// Phase attribution for store-level contributions: `on store build`
    /// fields are typed per phase (released at the type level on Frozen);
    /// `on store frozen` and unlabeled fields exist in all phases.
    phase: Option<crate::backing::PhaseLabel>,
}

// ---------------------------------------------------------------------------
// The composed role binding
// ---------------------------------------------------------------------------

/// Composed placement of one binding's roles: fusion classes, path ids,
/// host nesting, the partner map. `FlatRoles` is the flat twin; the role
/// table this implements is pinned in the M3 handoff (do not relitigate):
/// `rec`/`records`/`maps`/`host_of`/`idx`/`arena_of`/`map_from`/
/// `determining`/`handle` per placement, the vacuous host `()` wherever
/// ids are flat, and the Walk's reserved `host` at embedded targets.
struct ComposedRoles<'a> {
    classes: &'a [ClassData],
    store: TokenStream2,
    /// Classes of A, B, K (the partner map resolved open entities).
    class_of: [usize; 3],
    root_self: bool,
    a: Option<TokenStream2>,
    k: Option<TokenStream2>,
    w: Option<TokenStream2>,
    map_field: Option<Ident2>,
    det_field: Option<Ident2>,
    tgt_handle: Ident2,
    walk_ty: Option<Ident2>,
    walk_host: Option<TokenStream2>,
    renames: Vec<(String, Ident2)>,
    walk_fields: Vec<(Ident2, TokenStream2)>,
}

impl<'a> ComposedRoles<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        classes: &'a [ClassData],
        i: usize,
        store_field_names: &'a HashMap<(usize, String), HashMap<String, Ident2>>,
        decl: &BackingDecl,
        arrow: &crate::grammar::ArrowDecl,
        class_of: [usize; 3],
        root_self: bool,
        tgt_handle: Ident2,
    ) -> Self {
        let stem = crate::grammar::link_stem(&arrow.name);
        // Store-level fields keep the formula name bare when unambiguous;
        // colliding store fields were instance-prefixed at collection, and
        // the body renames must follow (the same override the flat codegen
        // threads).
        let store_overrides = store_field_names
            .get(&(i, arrow.name.to_string()))
            .cloned()
            .unwrap_or_default();
        let renames = crate::codegen::contributed_fields(decl)
            .map(|f| {
                let emitted = store_overrides
                    .get(&f.logical.to_string())
                    .cloned()
                    .unwrap_or_else(|| formula_name(f, f.formula, &arrow.name, &stem));
                (f.logical.to_string(), emitted)
            })
            .collect();
        let walk_fields = decl
            .clause(|c| matches!(c, Clause::Iter { .. }))
            .map(|c| match c {
                Clause::Iter { fields, .. } => fields
                    .iter()
                    .map(|(n, t)| (n.clone(), t.to_token_stream()))
                    .collect::<Vec<_>>(),
                _ => unreachable!(),
            })
            .unwrap_or_default();
        ComposedRoles {
            classes,
            store: quote!(self),
            class_of,
            root_self,
            a: None,
            k: None,
            w: None,
            map_field: None,
            det_field: None,
            tgt_handle,
            walk_ty: None,
            walk_host: None,
            renames,
            walk_fields,
        }
    }

    fn at(mut self, store: TokenStream2) -> Self {
        self.store = store;
        self
    }

    fn with_a(mut self, a: TokenStream2) -> Self {
        self.a = Some(a);
        self
    }

    fn with_k(mut self, k: TokenStream2) -> Self {
        self.k = Some(k);
        self
    }

    fn with_w(mut self, w: TokenStream2) -> Self {
        self.w = Some(w);
        self
    }

    fn with_walk_ty(mut self, w: Ident2) -> Self {
        self.walk_ty = Some(w);
        self
    }

    fn with_walk_host(mut self, h: TokenStream2) -> Self {
        self.walk_host = Some(h);
        self
    }

    fn with_det(mut self, det: Ident2) -> Self {
        self.det_field = Some(det);
        self
    }

    fn with_map(mut self, map: Ident2) -> Self {
        self.map_field = Some(map);
        self
    }

    fn class(&self, e: Ent) -> usize {
        self.class_of[match e {
            Ent::A => 0,
            Ent::B => 1,
            Ent::K => 2,
        }]
    }
}

impl RoleBinding for ComposedRoles<'_> {
    fn store(&self) -> TokenStream2 {
        self.store.clone()
    }
    fn param(&self, name: &str) -> Option<TokenStream2> {
        match name {
            "a" => self.a.clone().or_else(|| self.root_self.then(|| quote!(()))),
            "k" => self.k.clone(),
            "w" => self.w.clone(),
            _ => None,
        }
    }
    fn id_ty(&self, e: Ent) -> TokenStream2 {
        if e == Ent::A && self.root_self {
            // The unfused root owns no id type.
            return quote!(());
        }
        let idt = &self.classes[self.class(e)].id_name;
        quote!(#idt)
    }
    fn rec(&self, arg: TokenStream2, e: Ent) -> TokenStream2 {
        if self.root_self && e == Ent::A {
            // The unfused root's record is the store itself.
            return self.store();
        }
        rec_ref(self.classes, self.store(), self.class(e), arg)
    }
    fn records(&self, e: Ent) -> TokenStream2 {
        let ci = self.class(e);
        if !self.classes[ci].owns_records {
            return quote!(::core::iter::once(()));
        }
        let s = self.store();
        match self.classes[ci].loc {
            ClassLoc::Root => {
                let a = &self.classes[ci].arena_fld;
                let idt = &self.classes[ci].id_name;
                quote!((0..#s.#a.len() as u32).map(#idt))
            }
            ClassLoc::Embedded(hc) => {
                let ha = &self.classes[hc].arena_fld;
                let hid = &self.classes[hc].id_name;
                let a = &self.classes[ci].arena_fld;
                let idt = &self.classes[ci].id_name;
                quote!((0..#s.#ha.len() as u32).flat_map(move |hi| {
                    (0..#s.#ha[hi as usize].#a.len() as u32)
                        .map(move |ri| #idt(#hid(hi), ri))
                }))
            }
        }
    }
    fn maps(&self) -> TokenStream2 {
        let f = self.map_field.as_ref().expect("keyed backing contributes its map field");
        let sc = self.class(Ent::A);
        let tc = self.class(Ent::B);
        let s = self.store();
        if !self.classes[sc].owns_records {
            // Store-level: one pair, the vacuous host the owner.
            return quote!(::core::iter::once(((), &#s.#f)));
        }
        let sa = &self.classes[sc].arena_fld;
        match self.classes[tc].loc {
            ClassLoc::Root => {
                // Flat ids carry no host component: the vacuous host is each
                // pair's owner (the "on {:?}" self-id spelling is gone here).
                quote!((0..#s.#sa.len() as u32).map(|ri| ((), &#s.#sa[ri as usize].#f)))
            }
            ClassLoc::Embedded(hc) if sc == hc => {
                // The self class hosts the target: each record's id is the
                // host id its map's values must agree with.
                let sid = &self.classes[sc].id_name;
                quote!((0..#s.#sa.len() as u32).map(|ri| (#sid(ri), &#s.#sa[ri as usize].#f)))
            }
            ClassLoc::Embedded(hc) => {
                // Self embedded in the target's host (checked at implement
                // time): one pair per embedded record, the host id the owner.
                let ha = &self.classes[hc].arena_fld;
                let hid = &self.classes[hc].id_name;
                quote!((0..#s.#ha.len() as u32).flat_map(move |hi| {
                    (0..#s.#ha[hi as usize].#sa.len() as u32)
                        .map(move |ri| (#hid(hi), &#s.#ha[hi as usize].#sa[ri as usize].#f))
                }))
            }
        }
    }
    fn host_of(&self, arg: TokenStream2, e: Ent) -> TokenStream2 {
        match self.classes[self.class(e)].loc {
            ClassLoc::Root => quote!(()),
            ClassLoc::Embedded(_) => quote!((#arg).0),
        }
    }
    fn idx(&self, arg: TokenStream2, e: Ent) -> TokenStream2 {
        match self.classes[self.class(e)].loc {
            ClassLoc::Root => quote!((#arg).0 as usize),
            ClassLoc::Embedded(_) => quote!((#arg).1 as usize),
        }
    }
    fn arena_of(&self, e: Ent, arg: TokenStream2) -> TokenStream2 {
        let ci = self.class(e);
        let s = self.store();
        match self.classes[ci].loc {
            ClassLoc::Root => {
                let a = &self.classes[ci].arena_fld;
                quote!(#s.#a)
            }
            // The arg names a host-class record (the only shape the
            // declarations use: `b_arena_of(a)` with A the host, and
            // `*_arena_of(host_of(v))`).
            ClassLoc::Embedded(hc) => {
                let a = &self.classes[ci].arena_fld;
                let r = rec_ref(self.classes, s, hc, arg);
                quote!(#r.#a)
            }
        }
    }
    fn map_from(&self, arg: TokenStream2) -> TokenStream2 {
        let f = self.map_field.as_ref().expect("keyed backing contributes its map field");
        if !self.classes[self.class(Ent::A)].owns_records {
            let s = self.store();
            return quote!(#s.#f);
        }
        let r = self.rec(arg, Ent::A);
        quote!(#r.#f)
    }
    fn determining(&self, arg: TokenStream2) -> TokenStream2 {
        let f = self.det_field.as_ref().expect("validate by binds det_label and the determinant field");
        let r = self.rec(arg, Ent::B);
        quote!(#r.#f)
    }
    fn handle(&self, arg: TokenStream2, raw: bool) -> TokenStream2 {
        let h = &self.tgt_handle;
        let s = self.store();
        let tc = self.class(Ent::B);
        match (self.classes[tc].loc, raw) {
            (ClassLoc::Root, false) => quote!(#h { store: #s, id: #arg }),
            (ClassLoc::Root, true) => {
                let idt = &self.classes[tc].id_name;
                quote!(#h { store: #s, id: #idt(#arg) })
            }
            (ClassLoc::Embedded(_), false) => quote!(#h { store: #s, host: (#arg).0, id: (#arg).1 }),
            // The r4-advance index mint: the path id `#Id(host, x)` split
            // into the handle's fields, the host read from the Walk's
            // reserved field.
            (ClassLoc::Embedded(_), true) => {
                let host = self
                    .walk_host
                    .clone()
                    .expect("a raw handle at an embedded target needs the Walk's host");
                quote!(#h { store: #s, host: #host, id: #arg })
            }
        }
    }
    fn walk_ty(&self) -> Option<Ident2> {
        self.walk_ty.clone()
    }
    fn walk_host(&self) -> Option<TokenStream2> {
        self.walk_host.clone()
    }
    fn rename_field(&self, logical: &str) -> Option<Ident2> {
        self.renames
            .iter()
            .find(|(l, _)| l == logical)
            .map(|(_, e)| e.clone())
    }
    fn walk_field_is_raw(&self, field: &str) -> bool {
        self.walk_fields
            .iter()
            .any(|(n, t)| n.to_string() == field && t.to_string() == "u32")
    }
    fn is_walk_field(&self, field: &str) -> bool {
        self.walk_fields.iter().any(|(n, _)| n.to_string() == field)
    }
}

/// Record access by an id *value* through a store token: `store.<arena>[id.0]`
/// at root level, nested through the host arena for path ids.
fn rec_ref(classes: &[ClassData], store: TokenStream2, ci: usize, idv: TokenStream2) -> TokenStream2 {
    match classes[ci].loc {
        ClassLoc::Root => {
            let a = &classes[ci].arena_fld;
            quote!(#store.#a[(#idv).0 as usize])
        }
        ClassLoc::Embedded(host) => {
            let ha = &classes[host].arena_fld;
            let a = &classes[ci].arena_fld;
            quote!(#store.#ha[(#idv).0.0 as usize].#a[(#idv).1 as usize])
        }
    }
}

/// Collect one declared contributed field into its owner's bucket (a class
/// record; `owner == None` is the store, for an unfused-root self). The
/// declaration's naming formula, collision prefixing (the instance names
/// the colliding scope), the combined ctor rule (share one argument when
/// the resolved target classes agree, else prefix), and the uniform init
/// rule are decided here; the per-arrow registries (`field_of_arrow` for
/// ctor-fed fields, `map_of_arrow` for keyed maps) record the results.
#[allow(clippy::too_many_arguments)]
fn push_contribution(
    classes: &mut [ClassData],
    store_fields: &mut Vec<FieldContribution>,
    owner: Option<usize>,
    field_of_arrow: &mut HashMap<(usize, String), Ident2>,
    map_of_arrow: &mut HashMap<(usize, String), Ident2>,
    members: &[(Ident2, String, crate::grammar::SchemaBody)],
    i: usize,
    a: &crate::grammar::ArrowDecl,
    d: &BackingDecl,
    f: &crate::backing::ContribField,
    side_a: bool,
    class_of_arrow: &dyn Fn(usize, &crate::grammar::ArrowDecl) -> syn::Result<[usize; 3]>,
) -> syn::Result<()> {
    let stem = crate::grammar::link_stem(&a.name);
    let want = formula_name(f, f.formula, &a.name, &stem);
    let inst_snake = snake(&members[i].0.to_string());
    let ty = {
        let class_of = class_of_arrow(i, a)?;
        // Types never reference field names; the empty override map.
        let no_store_names: HashMap<(usize, String), HashMap<String, Ident2>> = HashMap::new();
        let roles = ComposedRoles::new(classes, i, &no_store_names, d, a, class_of, false, a.target.clone());
        roles::rewrite_ty(&f.ty.to_token_stream(), &roles)
    };
    let option_id = f
        .ty
        .to_token_stream()
        .to_string()
        .replace(' ', "")
        .starts_with("Option<");
    let bucket: &mut Vec<FieldContribution> = match owner {
        Some(c) => &mut classes[c].fields,
        None => store_fields,
    };
    // The combined ctor rule: intra-class argument-name collisions share
    // one argument when the resolved target classes agree, else prefix.
    if side_a && d.has(|c| matches!(c, Clause::CtorArg { .. })) {
        if let Some(pos) = bucket
            .iter()
            .position(|fc| fc.field == want && matches!(fc.init, FieldInit::CtorArg(_)))
        {
            if bucket[pos].ty.to_string() == ty.to_string() {
                field_of_arrow.insert((i, a.name.to_string()), bucket[pos].field.clone());
                return Ok(());
            }
            let prefixed = format_ident!("{}_{}", inst_snake, want);
            field_of_arrow.insert((i, a.name.to_string()), prefixed.clone());
            bucket.push(FieldContribution {
                field: prefixed.clone(),
                ty,
                init: FieldInit::CtorArg(prefixed),
                inst: i,
                arrow: a.name.clone(),
                side_a,
                option_id,
                phase: None,
            });
            return Ok(());
        }
        let arg = Ident2::new(&want.to_string(), a.name.span());
        field_of_arrow.insert((i, a.name.to_string()), arg.clone());
        bucket.push(FieldContribution {
            field: arg.clone(),
            ty,
            init: FieldInit::CtorArg(arg),
            inst: i,
            arrow: a.name.clone(),
            side_a,
            option_id,
            phase: None,
        });
        if card_of_decl(d) == Cardinality::Keyed {
            map_of_arrow.insert((i, a.name.to_string()), bucket.last().unwrap().field.clone());
        }
        return Ok(());
    }
    // Collision prefixing names the colliding scope (the instance).
    let emitted = if bucket.iter().any(|fc| fc.field == want) {
        format_ident!("{}_{}", inst_snake, want)
    } else {
        Ident2::new(&want.to_string(), f.logical.span())
    };
    if card_of_decl(d) == Cardinality::Keyed && side_a {
        map_of_arrow.insert((i, a.name.to_string()), emitted.clone());
    }
    let init = init_of(d, side_a, f, &a.name)?;
    bucket.push(FieldContribution {
        field: emitted,
        ty,
        init,
        inst: i,
        arrow: a.name.clone(),
        side_a,
        option_id,
        phase: None,
    });
    Ok(())
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

    // ---- per-instance arrow resolution (declaration-driven) ----
    let decl_of = |key: &str| -> Option<&BackingDecl> {
        config
            .backing_decls
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, p)| &p.decl)
    };
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
                .find(|b| b.instance.as_ref() == Some(inst_name) && b.arrow == decl.name);
            let backing = if decl.sum.is_some() {
                None
            } else if let Some(b) = explicit {
                let d = decl_of(&b.key_str())
                    .expect("stage1 queued every explicit binding's courier");
                if card_of_decl(d) != meta.card {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!(
                            "backing `{}` has wrong cardinality for arrow `{inst_name}.{}` (a `{}` arrow)",
                            d.name,
                            decl.name,
                            match meta.card {
                                Cardinality::Many => "many",
                                Cardinality::One => "one",
                                Cardinality::Keyed => "keyed",
                            }
                        ),
                    ));
                }
                Some(d)
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
                // `one` arrows default to `field`; the default-decl hop has
                // collected its declaration under the arrow's key.
                Some(decl_of(&format!("{inst_name}.{}", decl.name)).expect("defaulted one arrow's field declaration collected"))
            };
            infos.push(ArrowInfo { decl, meta, backing });
        }
        inst_infos.push(infos);
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
    // A/B/K classes of one arrow (the partner map resolved open entities).
    let class_of_arrow = |i: usize, a: &crate::grammar::ArrowDecl| -> syn::Result<[usize; 3]> {
        Ok([
            class_index_of(i, ent_index(i, &a.source)?),
            class_index_of(i, ent_index(i, &a.target)?),
            match &a.key {
                Some(k) => class_index_of(i, ent_index(i, k)?),
                None => class_index_of(i, ent_index(i, &a.target)?),
            },
        ])
    };

    // ---- per-declaration applicability, against computed placement ----
    // Triggers are declaration facts (clauses, cardinality, declared
    // predicates); the messages are the pinned ones (a backing's
    // declaration name is its binding spelling).
    // Placement-fact descriptions for the pinned error templates: an
    // endpoint's location, B's arena location, and A's self case per the
    // plan's three-case taxonomy.
    let loc_desc = |ci: usize| -> String {
        if !classes[ci].owns_records {
            format!("the store (unfused root `{}`)", classes[ci].name)
        } else {
            match classes[ci].loc {
                ClassLoc::Root => "at root level".to_string(),
                ClassLoc::Embedded(h) => format!("embedded in `{}`", classes[h].name),
            }
        }
    };
    let arena_loc_desc = |ci: usize| -> String {
        match classes[ci].loc {
            ClassLoc::Root => "a root-level arena of the store".to_string(),
            ClassLoc::Embedded(h) => format!("embedded in `{}`'s records", classes[h].name),
        }
    };
    let self_case_desc = |ci: usize| -> String {
        if !classes[ci].owns_records {
            "an unfused root (its record is the store; the predicate is satisfied by any root-level B arena)".to_string()
        } else if classes[ci].has_root {
            "a fused root-marked self (its record is the class record it is hosted by)".to_string()
        } else {
            "a plain self (an element record; the predicate needs B's arena embedded in this class's record)"
                .to_string()
        }
    };
    // `membership: existence` (declared predicate): B's entity arrow is
    // the unique arrow targeting B bound to an existence backing — a
    // duplicate binding is rejected naming both arrows (the plan's
    // evaluation table).
    let mut existence_by_target: HashMap<usize, Vec<String>> = HashMap::new();
    for (i, infos) in inst_infos.iter().enumerate() {
        for info in infos {
            let Some(d) = info.backing else { continue };
            if !d.has(|c| matches!(c, Clause::MembershipExistence { .. })) {
                continue;
            }
            let tc = class_index_of(i, ent_index(i, &info.decl.target)?);
            existence_by_target
                .entry(tc)
                .or_default()
                .push(format!("{}.{}", members[i].0, info.decl.name));
        }
    }
    for (tc, arrows) in &existence_by_target {
        if arrows.len() > 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "class `{}` has two entity-arrow candidates — arrows `{}` and `{}` both bind a `membership: existence` backing; the entity arrow of a target is unique, so one of them must bind a relation backing (`intrusive_list`)",
                    classes[*tc].name,
                    arrows[0],
                    arrows[1],
                ),
            ));
        }
    }
    for (i, infos) in inst_infos.iter().enumerate() {
        for info in infos {
            let Some(d) = info.backing else { continue };
            let a = info.decl;
            let named = format!("{}.{}", members[i].0, a.name);
            let sc = class_index_of(i, ent_index(i, &a.source)?);
            let tc = class_index_of(i, ent_index(i, &a.target)?);
            // Phased backings (csr): the entity-arrow / unfused-root
            // rejection. A phased backing maintains membership by insert
            // hooks, so it cannot bind its target's entity arrow —
            // mechanically, `hosts_arena(self, B)` must not hold: neither
            // the unfused-root self arm (no self record, no `a`) nor the
            // embedded-target-in-self-class arm (B's arena embeds in the
            // self record — the entity-arrow shape).
            if d.is_phased() {
                let hosts_arena_shape = match classes[tc].loc {
                    ClassLoc::Root => !classes[sc].owns_records,
                    ClassLoc::Embedded(h) => h == sc,
                };
                if hosts_arena_shape {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!(
                            "backing `{d_name}` (arrow `{named}`) is phased, but B's arena is hosted by the self record — the entity-arrow shape (self case: {self_case}; placement: target `{tc_name}` {target_loc}); a hook-maintained backing cannot declare existence (bind `arena` for the entity arrow, or bind the phased backing at a relation arrow whose B arena is root-level)",
                            d_name = d.name,
                            self_case = self_case_desc(sc),
                            tc_name = classes[tc].name,
                            target_loc = loc_desc(tc),
                        ),
                    ));
                }
            }
            // Constructor-orchestrated clauses need an `add_<A>`-orchestrated
            // self: an unfused root owns no constructor and no self id.
            let orchestrated = d.has(|c| matches!(c, Clause::CtorArg { .. }))
                || d.has(|c| matches!(c, Clause::OnAdd { .. }));
            if orchestrated && !classes[sc].owns_records {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "arrow `{named}` is sourced at the root-marked entity `{}`, which is unfused; the `{}` backing needs a self record, and an unfused root owns none (it *is* the store)",
                        members[i].2.entities[ent_index(i, &a.source)?].name,
                        d.name
                    ),
                ));
            }
            // The `one` access protocol returns an id the handle mint
            // wraps; an embedded (host-anchored) id cannot be supplied.
            if card_of_decl(d) == Cardinality::One {
                if let ClassLoc::Embedded(_) = classes[tc].loc {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!(
                            "arrow `{named}` is `{}`-backed into the embedded class `{}`; an embedded id needs a host path the constructor argument cannot supply",
                            d.name,
                            classes[tc].name
                        ),
                    ));
                }
            }
            // `membership: existence` (declared predicate): the entity
            // arrow's membership kind is primitive — a `by`-derived arrow
            // contradicts existence (the plan's "the arrow and its
            // membership kind otherwise" arm).
            if d.has(|c| matches!(c, Clause::MembershipExistence { .. })) {
                if let Some(det) = &a.by {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!(
                            "backing `{d_name}` on arrow `{named}` declares `membership: existence`, but `{named}` derives membership `by {det}` — existence is primitive; bind a relation backing (`intrusive_list`) or drop the `by`",
                            d_name = d.name,
                        ),
                    ));
                }
            }
            // `hosts_arena` (declared predicate): B's arena is hosted by
            // A's record — B's arena embeds in the self class's record, or
            // the self is an unfused root and B's arena is root-level.
            if d.has(|c| matches!(c, Clause::Requires { items, .. } if items.iter().any(|x| *x == "hosts_arena"))) {
                let holds = match classes[tc].loc {
                    ClassLoc::Root => !classes[sc].owns_records,
                    ClassLoc::Embedded(h) => h == sc,
                };
                if !holds {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!(
                            "arrow `{named}` declares `requires: hosts_arena` (backing `{d_name}`), but B's arena is {arena_loc}, not hosted by the self record (self case: {self_case}; placement: target `{tc_name}` {target_loc})",
                            d_name = d.name,
                            arena_loc = arena_loc_desc(tc),
                            self_case = self_case_desc(sc),
                            tc_name = classes[tc].name,
                            target_loc = loc_desc(tc),
                        ),
                    ));
                }
            }
            // `direct_records` (declared predicate): both endpoints
            // addressable without path indirection — root-level classes or
            // the store (an embedded *target* is fine: path ids flow
            // through the links; an embedded *self* has no anchor).
            if d.has(|c| matches!(c, Clause::Requires { items, .. } if items.iter().any(|x| *x == "direct_records")))
                && matches!(classes[sc].loc, ClassLoc::Embedded(_))
            {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "arrow `{named}` declares `requires: direct_records` (backing `{d_name}`), but its self class `{sc_name}` is {self_loc} — the predicate holds only at root-level classes and the store (placement: self `{sc_name}` {self_loc}, target `{tc_name}` {target_loc})",
                        d_name = d.name,
                        sc_name = classes[sc].name,
                        self_loc = loc_desc(sc),
                        tc_name = classes[tc].name,
                        target_loc = loc_desc(tc),
                    ),
                ));
            }
            // The keyed host-agreement gap: the authored
            // `host_of(v) != owner` check compares against a self-side
            // anchor of the host id type — the self record (it IS the host)
            // or the self's own host component. Any other placement has no
            // such anchor, so the uniform body cannot typecheck.
            if card_of_decl(d) == Cardinality::Keyed {
                if let ClassLoc::Embedded(hc) = classes[tc].loc {
                    let anchored =
                        sc == hc || matches!(classes[sc].loc, ClassLoc::Embedded(sch) if sch == hc);
                    if !anchored {
                        let self_loc = match classes[sc].loc {
                            ClassLoc::Root => "root level".to_string(),
                            ClassLoc::Embedded(sch) => {
                                format!("embedded in `{}`", classes[sch].name)
                            }
                        };
                        return Err(syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!(
                                "arrow `{named}` keys into the embedded class `{}` (hosted by `{}`), but its self `{}` is neither that host nor embedded in it — the authored value-host-agreement check has no self-side anchor of the host id type (placement: self `{}` at {self_loc}, target `{}` embedded in `{}`)",
                                classes[tc].name,
                                classes[hc].name,
                                classes[sc].name,
                                classes[sc].name,
                                classes[tc].name,
                                classes[hc].name
                            ),
                        ));
                    }
                }
            }
            // Validate-variant selection: the arrow's membership facts pick
            // the body; a declaration that does not author it is rejected. A
            // `Plain` body (membership-agnostic — csr's offsets check)
            // satisfies any membership kind.
            if card_of_decl(d) == Cardinality::Many {
                let wanted = if a.by.is_some() {
                    ValidateVariant::By
                } else {
                    ValidateVariant::Primitive
                };
                let has_it = d.has(|c| matches!(c, Clause::Validate { variant: v, .. } if *v == wanted))
                    || d.has(|c| matches!(c, Clause::Validate { variant: ValidateVariant::Plain, .. }));
                if !has_it && d.has(|c| matches!(c, Clause::Validate { .. })) {
                    let missing = match wanted {
                        ValidateVariant::By => "`validate by`",
                        ValidateVariant::Primitive => "`validate primitive`",
                        ValidateVariant::Plain => "`validate`",
                    };
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!(
                            "backing `{}` authors no {missing} body, but arrow `{named}` needs it (the declaration binds only the other membership kind)",
                            d.name
                        ),
                    ));
                }
            }
        }
    }

    // `by <det>`: the determining arrow must be total (its root says `one`).
    for (i, infos) in inst_infos.iter().enumerate() {
        for info in infos {
            let Some(det_name) = &info.decl.by else { continue };
            let det = infos.iter().find(|x| &x.decl.name == det_name).ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("`by {}` does not name a resolvable arrow of instance `{}`", det_name, members[i].0),
                )
            })?;
            if det.meta.card != Cardinality::One {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "`by {}` names arrow `{}.{}`, which is not total (`one`) — membership would be undefined",
                        det_name,
                        members[i].0,
                        det.decl.name
                    ),
                ));
            }
        }
    }

    // ---- field contributions per class, from the declarations ----
    // (instance, arrow name) -> emitted ctor-fed field / keyed map field.
    let mut field_of_arrow: HashMap<(usize, String), Ident2> = HashMap::new();
    let mut map_of_arrow: HashMap<(usize, String), Ident2> = HashMap::new();
    // Store-level contributions: an `on A` clause whose self is an unfused
    // root folds to the store (its record is the store).
    let mut store_fields: Vec<FieldContribution> = Vec::new();

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

    // Collect one declared contributed field into its owner's bucket (a
    // class record, or the store when the self is an unfused root). The
    // caller has already computed the placement-derived facts (the
    // formula-named base, the rewritten type); this decides naming,
    // collisions, and initialization.
    for ci in 0..classes.len() {
        if !classes[ci].owns_records {
            continue;
        }
        for (i, e) in hosted[ci].clone() {
            let ent = &members[i].2.entities[e].name;
            for info in &inst_infos[i] {
                let Some(d) = info.backing else { continue };
                let a = info.decl;
                if *ent == a.source {
                    if let Some(Clause::On { fields, .. }) = on_clause(d, OnTarget::A) {
                        for f in fields {
                            push_contribution(&mut classes, &mut store_fields, Some(ci), &mut field_of_arrow, &mut map_of_arrow, members, i, a, d, f, true, &class_of_arrow)?;
                        }
                    }
                }
                if *ent == a.target {
                    if let Some(Clause::On { fields, .. }) = on_clause(d, OnTarget::B) {
                        for f in fields {
                            push_contribution(&mut classes, &mut store_fields, Some(ci), &mut field_of_arrow, &mut map_of_arrow, members, i, a, d, f, false, &class_of_arrow)?;
                        }
                    }
                }
            }
        }
    }
    // Pre-pass: assign every store-level field's final name, mirroring
    // push_contribution's collision rule (first keeps the formula name,
    // colliders get the instance prefix). Two passes contribute at store
    // level, in this order: `on A` at an unfused-root self, then explicit
    // `on store` clauses. The per-(instance, arrow) maps ride into
    // ComposedRoles so authored bodies' field renames follow the assigned
    // names ("bare when unambiguous", the plan's csr naming pin).
    let mut store_field_names: HashMap<(usize, String), HashMap<String, Ident2>> = HashMap::new();
    {
        let mut assigned: Vec<Ident2> = Vec::new();
        let mut assign = |assigned: &mut Vec<Ident2>, i: usize, a: &crate::grammar::ArrowDecl, f: &crate::backing::ContribField| {
            let want = formula_name(f, f.formula, &a.name, &crate::grammar::link_stem(&a.name));
            let emitted = if assigned.iter().any(|e| *e == want) {
                format_ident!("{}_{}", snake(&members[i].0.to_string()), want)
            } else {
                Ident2::new(&want.to_string(), f.logical.span())
            };
            assigned.push(emitted.clone());
            store_field_names
                .entry((i, a.name.to_string()))
                .or_default()
                .insert(f.logical.to_string(), emitted);
        };
        for (i, infos) in inst_infos.iter().enumerate() {
            for info in infos {
                let Some(d) = info.backing else { continue };
                let a = info.decl;
                let sc = class_index_of(i, ent_index(i, &a.source)?);
                if classes[sc].owns_records {
                    continue;
                }
                for (_label, fields) in on_clauses(d, OnTarget::A) {
                    for f in fields {
                        assign(&mut assigned, i, a, f);
                    }
                }
            }
        }
        for (i, infos) in inst_infos.iter().enumerate() {
            for info in infos {
                let Some(d) = info.backing else { continue };
                let a = info.decl;
                for (_label, fields) in on_clauses(d, OnTarget::Store) {
                    for f in fields {
                        assign(&mut assigned, i, a, f);
                    }
                }
            }
        }
    }

    // Store-level pass: `on A` at an unfused-root self (the relaxed
    // intrusive head; a keyed map; anything else the declarations state).
    for (i, infos) in inst_infos.iter().enumerate() {
        for info in infos {
            let Some(d) = info.backing else { continue };
            let a = info.decl;
            let sc = class_index_of(i, ent_index(i, &a.source)?);
            if classes[sc].owns_records {
                continue;
            }
            for (_label, fields) in on_clauses(d, OnTarget::A) {
                for f in fields {
                    push_contribution(&mut classes, &mut store_fields, None, &mut field_of_arrow, &mut map_of_arrow, members, i, a, d, f, true, &class_of_arrow)?;
                }
            }
        }
    }

    // Explicit `on store` contributions (the phased csr fields; any
    // unlabeled store-level contribution). Named by the pre-pass; the
    // label is the phase attribution. These bypass push_contribution's
    // naming (the pre-pass decided it) but reuse its init rule.
    for (i, infos) in inst_infos.iter().enumerate() {
        for info in infos {
            let Some(d) = info.backing else { continue };
            let a = info.decl;
            for (label, fields) in on_clauses(d, OnTarget::Store) {
                for f in fields {
                    let field = store_field_names[&(i, a.name.to_string())][&f.logical.to_string()].clone();
                    let class_of = class_of_arrow(i, a)?;
                    let roles = ComposedRoles::new(&classes, i, &store_field_names, d, a, class_of, false, a.target.clone());
                    let ty = roles::rewrite_ty(&f.ty.to_token_stream(), &roles);
                    let option_id = f
                        .ty
                        .to_token_stream()
                        .to_string()
                        .replace(' ', "")
                        .starts_with("Option<");
                    let init = init_of(d, false, f, &a.name)?;
                    store_fields.push(FieldContribution {
                        field,
                        ty,
                        init,
                        inst: i,
                        arrow: a.name.clone(),
                        side_a: true,
                        option_id,
                        phase: label,
                    });
                }
            }
        }
    }

    let id_ty_of =
        |i: usize, e: usize| -> Ident2 { classes[class_index_of(i, e)].id_name.clone() };

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
        let mut fields = TokenStream2::new();
        for fc in &classes[ci].fields {
            let f = &fc.field;
            let ty = &fc.ty;
            let inst_name = &members[fc.inst].0;
            let arrow = &fc.arrow;
            fields.extend(quote! {
                #[doc = concat!("field contribution of arrow `", stringify!(#inst_name), ".", stringify!(#arrow), "`.")]
                #f: #ty,
            });
        }
        // Embedded arenas contributed by instances placed in this class.
        for d in classes.iter() {
            if let ClassLoc::Embedded(h) = d.loc {
                if h == ci {
                    let a = &d.arena_fld;
                    let dat = &d.data_name;
                    fields.extend(quote!(#a: ::std::vec::Vec<#dat>,));
                }
            }
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
    // The store binds a phased backing (csr) when any bound declaration
    // carries `phase:` — the type-state emission keys on this.
    let phased = inst_infos
        .iter()
        .flatten()
        .any(|info| info.backing.map(|d| d.is_phased()).unwrap_or(false));

    // The phase trait: one associated type per `on store build` field; the
    // Build impl types them naturally, Frozen types them `()`. `pub`
    // because the store's parameter is bounded by it (edition 2024 denies
    // private bounds); doc-hidden — not user API.
    let phase_trait = format_ident!("__{}Phases", store);
    let store_contrib = store_fields.iter().map(|fc| {
        let n = &fc.field;
        let ty = &fc.ty;
        let inst_name = &members[fc.inst].0;
        let arrow = &fc.arrow;
        // Phase attribution: `on store build` fields are typed through the
        // phase trait — `()` on Frozen — so the accumulators are released
        // at the type level; `on store frozen` and unlabeled fields exist
        // in all phases with their natural type.
        match fc.phase {
            Some(crate::backing::PhaseLabel::Build) => {
                let assoc = format_ident!("__build_{}", fc.field);
                quote! {
                    #[doc = concat!("store-level contribution of arrow `", stringify!(#inst_name), ".", stringify!(#arrow), "`.")]
                    #n: <P as #phase_trait>::#assoc,
                }
            }
            _ => quote! {
                #[doc = concat!("store-level contribution of arrow `", stringify!(#inst_name), ".", stringify!(#arrow), "`.")]
                #n: #ty,
            },
        }
    });
    let store_pattern_name = config
        .pattern_path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default();
    let phase_assocs: Vec<(Ident2, TokenStream2)> = store_fields
        .iter()
        .filter(|fc| fc.phase == Some(crate::backing::PhaseLabel::Build))
        .map(|fc| (format_ident!("__build_{}", fc.field), fc.ty.clone()))
        .collect();
    let assoc_names: Vec<&Ident2> = phase_assocs.iter().map(|(n, _)| n).collect();
    let assoc_tys: Vec<&TokenStream2> = phase_assocs.iter().map(|(_, t)| t).collect();
    let phase_trait_def = quote! {
        #[doc(hidden)]
        pub trait #phase_trait {
            #(type #assoc_names;)*
        }
        impl #phase_trait for Build {
            #(type #assoc_names = #assoc_tys;)*
        }
        impl #phase_trait for Frozen {
            #(type #assoc_names = ();)*
        }
    };
    if phased {
        // Type-state store (spike 6 / the mixed-phase probe): ONE store
        // type parameterized by a phase marker. Non-phased composed stores
        // keep the M2 shape exactly.
        let default_init = {
            let field_inits = classes
                .iter()
                .filter(|cl| cl.owns_records && cl.loc == ClassLoc::Root)
                .map(|cl| {
                    let a = &cl.arena_fld;
                    quote!(#a: ::std::default::Default::default(),)
                })
                .chain(store_fields.iter().map(|fc| {
                    let n = &fc.field;
                    quote!(#n: ::std::default::Default::default(),)
                }))
                .chain(::core::iter::once(quote!(_phase: ::core::marker::PhantomData,)))
                .collect::<Vec<_>>();
            quote! {
                Self {
                    #(#field_inits)*
                }
            }
        };
        let debug_names: Vec<Ident2> = classes
            .iter()
            .filter(|cl| cl.owns_records && cl.loc == ClassLoc::Root)
            .map(|cl| cl.arena_fld.clone())
            .chain(store_fields.iter().map(|fc| fc.field.clone()))
            .collect();
        let debug_fields = &debug_names;
        let debug_fmt = quote! {
            f.debug_struct(stringify!(#store))
                #(.field(stringify!(#debug_names), &self.#debug_fields))*
                .finish()
        };
        out.extend(quote! {
            /// Build phase: the store is owned mutably; `add_*`/`insert_*`,
            /// the backing hooks, and `freeze` exist only here.
            pub struct Build(());
            /// Frozen phase: compaction has run at the single `freeze()`;
            /// reads, handles, and `validate` exist only here.
            pub struct Frozen(());
            #phase_trait_def

            #[doc = concat!("Store generated by `implement!` for the composed pattern `", #store_pattern_name, "`.")]
            pub struct #store<P: #phase_trait = Frozen> {
                #(#root_arenas)*
                #(#store_contrib)*
                _phase: ::core::marker::PhantomData<P>,
            }

            impl ::core::default::Default for #store<Build> {
                fn default() -> Self {
                    #default_init
                }
            }
            impl ::core::default::Default for #store<Frozen> {
                fn default() -> Self {
                    #default_init
                }
            }
            impl ::core::fmt::Debug for #store<Build> {
                fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                    #debug_fmt
                }
            }
            impl ::core::fmt::Debug for #store<Frozen> {
                fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                    #debug_fmt
                }
            }

            impl #store<Build> {
                #[doc = "Empty store (build phase)."]
                pub fn new() -> Self {
                    Self::default()
                }
            }
        });
    } else {
        out.extend(quote! {
            #[doc = concat!("Store generated by `implement!` for the composed pattern `", #store_pattern_name, "`.")]
            #[derive(Default, Debug)]
            pub struct #store {
                #(#root_arenas)*
                #(#store_contrib)*
            }

            impl #store {
                #[doc = "Empty store."]
                pub fn new() -> Self {
                    Self::default()
                }
            }
        });
    }

    // ---- constructors, hooks, write ops, counts, root accessors ----
    // Write-side accumulates into `store_impl_build` (constructors, hooks,
    // ops; on a phased store these land in `impl Store<Build>`); read-side
    // into `store_impl_frozen` (counts, the read view, root accessors,
    // validate; `impl Store<Frozen>`). Unphased stores emit one impl block
    // holding both — impl-item order is semantically irrelevant, and the
    // suites are the criterion.
    let mut store_impl_build = TokenStream2::new();
    let mut store_impl_frozen = TokenStream2::new();
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
        // Merged ctor args: the class's ctor-fed field contributions, in
        // contribution order (composition-instance order, then member
        // schema order) — the combined rule was applied at collection.
        let arg_defs = classes[ci].fields.iter().filter_map(|fc| match &fc.init {
            FieldInit::CtorArg(a) => {
                let ty = &fc.ty;
                Some(quote!(#a: #ty,))
            }
            _ => None,
        });
        // Record literal: field args move in; hook-maintained fields start
        // empty (the hooks relink on insert); embedded arenas start empty.
        let mut record_fields = TokenStream2::new();
        for fc in &classes[ci].fields {
            let f = &fc.field;
            match &fc.init {
                FieldInit::CtorArg(a) => record_fields.extend(quote!(#f: #a,)),
                FieldInit::None_ => {
                    record_fields.extend(quote!(#f: ::core::option::Option::None,))
                }
                FieldInit::New => {
                    record_fields.extend(quote!(#f: ::std::default::Default::default(),))
                }
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
        // `on_add` hooks firing at construction of this class: a backing's
        // per-add growth of store-level per-self state (csr's bucket push).
        // Runs before the insert hooks below, so an insert hook that reads
        // the grown state for the new record sees it.
        let mut onadd_calls = TokenStream2::new();
        for &(i, e) in &cl.members {
            for info in &inst_infos[i] {
                let Some(bd) = info.backing else { continue };
                if !bd.has(|c| matches!(c, Clause::OnAdd { .. })) {
                    continue;
                }
                let a = info.decl;
                if class_index_of(i, ent_index(i, &a.source)?) != ci {
                    continue;
                }
                let _ = e;
                let hook = format_ident!("__onadd_{}_{}", snake(&members[i].0.to_string()), a.name);
                onadd_calls.extend(quote!(self.#hook(id);));
            }
        }
        // `by`-derived hooks firing at construction of this class: every
        // member arrow with an authored on_insert whose determinant `one`
        // arrow originates at a member entity.
        let mut hook_calls = TokenStream2::new();
        for &(i, e) in &cl.members {
            for info in &inst_infos[i] {
                let d = info.decl;
                let Some(det_name) = &d.by else { continue };
                if !info
                    .backing
                    .map(|b| b.has(|c| matches!(c, Clause::OnInsert { .. })))
                    .unwrap_or(false)
                {
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
        store_impl_build.extend(quote! {
            #[doc = #ctor_doc]
            pub fn #add_name(&mut self, #host_arg_def #(#arg_defs)*) -> #idt {
                #id_bind
                #push
                #onadd_calls
                #hook_calls
                id
            }
        });

        // Count fn: global at root level, per-host when embedded.
        let count_name = format_ident!("{}_count", snake(&cl.name.to_string()));
        op_names.push(count_name.to_string());
        match cl.loc {
            ClassLoc::Root => store_impl_frozen.extend(quote! {
                #[doc = concat!("Number of `", stringify!(#idt), " records.")]
                pub fn #count_name(&self) -> usize {
                    self.#arena.len()
                }
            }),
            ClassLoc::Embedded(host) => {
                let hid = &classes[host].id_name;
                let ha = &classes[host].arena_fld;
                store_impl_frozen.extend(quote! {
                    #[doc = concat!("Number of `", stringify!(#idt), "` records embedded in `host`'s record.")]
                    pub fn #count_name(&self, host: #hid) -> usize {
                        self.#ha[host.0 as usize].#arena.len()
                    }
                });
            }
        }
    }

    // Insert hooks + ops, field-write hooks + set ops, live-key predicates.
    // Hook names are always instance-qualified (internal); public op names
    // take the instance prefix only on cross-instance arrow-name collision.
    // Hook parameters keep the authored spellings (`a`, `b`, `k`) with
    // mixed-site spans; an unfused-root self omits `a` (it owns no id).
    for (i, infos) in inst_infos.iter().enumerate() {
        let inst_snake = snake(&members[i].0.to_string());
        let inst_name = &members[i].0;
        for info in infos {
            let Some(d) = info.backing else { continue };
            let a = info.decl;
            let arrow_users = arrow_name_users[&a.name.to_string()];
            let sc = class_index_of(i, ent_index(i, &a.source)?);
            let tc = class_index_of(i, ent_index(i, &a.target)?);
            let root_self = !classes[sc].owns_records;
            let sid = classes[sc].id_name.clone();
            let tid = classes[tc].id_name.clone();
            let arrow_doc = &a.name;

            // on_insert hook (+ primitive insert op).
            if let Some(body) = body_of(d, |c| matches!(c, Clause::OnInsert { .. })) {
                let backing_name = &d.name;
                let keyed = card_of_decl(d) == Cardinality::Keyed;
                let kid = if keyed {
                    Some(id_ty_of(i, ent_index(i, a.key.as_ref().unwrap())?))
                } else {
                    None
                };
                let hook = format_ident!("__insert_{}_{}", inst_snake, a.name);
                let class_of = class_of_arrow(i, a)?;
                let mut roles =
                    ComposedRoles::new(&classes, i, &store_field_names, d, a, class_of, root_self, classes[tc].name.clone())
                        .at(quote!(self));
                if keyed {
                    roles = roles.with_map(
                        map_of_arrow[&(i, a.name.to_string())].clone(),
                    );
                }
                if root_self {
                    roles = roles.with_a(quote!(()));
                }
                let body_rw = roles::rewrite(body, &roles);
                let label = format!("arrow `{inst_name}.{}`", a.name);
                let scaffolds = scaffold_lets(body, &label, None, false);
                let (sig, op_sig, op_args) = match (keyed, kid.as_ref(), root_self) {
                    (true, Some(kidt), true) => (
                        {
                            let k = Ident2::new("k", Span::mixed_site());
                            let b = Ident2::new("b", Span::mixed_site());
                            quote!(#k: #kidt, #b: #tid)
                        },
                        quote!(key: #kidt, target: #tid),
                        quote!(key, target),
                    ),
                    (true, Some(kidt), false) => (
                        {
                            let aa = Ident2::new("a", Span::mixed_site());
                            let k = Ident2::new("k", Span::mixed_site());
                            let b = Ident2::new("b", Span::mixed_site());
                            quote!(#aa: #sid, #k: #kidt, #b: #tid)
                        },
                        quote!(self_id: #sid, key: #kidt, target: #tid),
                        quote!(self_id, key, target),
                    ),
                    (false, _, true) => (
                        {
                            let b = Ident2::new("b", Span::mixed_site());
                            quote!(#b: #tid)
                        },
                        quote!(member: #tid),
                        quote!(member),
                    ),
                    (false, _, false) => (
                        {
                            let aa = Ident2::new("a", Span::mixed_site());
                            let b = Ident2::new("b", Span::mixed_site());
                            quote!(#aa: #sid, #b: #tid)
                        },
                        quote!(self_id: #sid, member: #tid),
                        quote!(self_id, member),
                    ),
                    (true, None, _) => unreachable!("keyed arrows bind a key sort"),
                };
                store_impl_build.extend(quote! {
                    #[doc(hidden)]
                    #[doc = concat!("insert hook of arrow `", stringify!(#arrow_doc), "` (instance ", stringify!(#inst_name), "; backing `", stringify!(#backing_name), "`)")]
                    fn #hook(&mut self, #sig) {
                        #scaffolds
                        #body_rw
                    }
                });
                // Primitive membership (no `by`): explicit insert op.
                if a.by.is_none() {
                    let op = if arrow_users > 1 {
                        format_ident!("insert_{}_{}", inst_snake, a.name)
                    } else {
                        format_ident!("insert_{}", a.name)
                    };
                    op_names.push(op.to_string());
                    store_impl_build.extend(quote! {
                        #[doc = concat!("Insert through the `", stringify!(#arrow_doc), "` insert hook (instance ", stringify!(#inst_name), "; backing `", stringify!(#backing_name), "`).")]
                        pub fn #op(&mut self, #op_sig) {
                            self.#hook(#op_args);
                        }
                    });
                }
            }

            // on_unlink hook: the total delete verb (M4's `set_`
            // re-pointing lowers through it), always instance-qualified.
            // Signature mirrors on_insert minus the key; the body's tail
            // is the bool.
            if let Some(body) = body_of(d, |c| matches!(c, Clause::OnUnlink { .. })) {
                let backing_name = &d.name;
                let hook = format_ident!("__unlink_{}_{}", inst_snake, a.name);
                let class_of = class_of_arrow(i, a)?;
                let mut roles =
                    ComposedRoles::new(&classes, i, &store_field_names, d, a, class_of, root_self, classes[tc].name.clone())
                        .at(quote!(self));
                if root_self {
                    roles = roles.with_a(quote!(()));
                }
                let body_rw = roles::rewrite(body, &roles);
                let label = format!("arrow `{inst_name}.{}`", a.name);
                let scaffolds = scaffold_lets(body, &label, None, false);
                let sig = if root_self {
                    let b = Ident2::new("b", Span::mixed_site());
                    quote!(#b: #tid)
                } else {
                    let aa = Ident2::new("a", Span::mixed_site());
                    let b = Ident2::new("b", Span::mixed_site());
                    quote!(#aa: #sid, #b: #tid)
                };
                store_impl_build.extend(quote! {
                    #[doc(hidden)]
                    #[doc = concat!("unlink hook of arrow `", stringify!(#arrow_doc), "` (instance ", stringify!(#inst_name), "; backing `", stringify!(#backing_name), "`): total — removes `b` from `a`'s list and returns whether it was there")]
                    fn #hook(&mut self, #sig) -> bool {
                        #scaffolds
                        #body_rw
                    }
                });
            }

            // Re-point hook + `set_` op: the single write path after
            // construction for a `ctor_arg`-fed field, grown into M4's
            // orchestration — unlink the by-derived memberships the field
            // determines, write, re-link under the new endpoint (the old
            // host is read from the record's own field: the by-derivation
            // invariant). Per clause authored; keyed derivatives skipped
            // (relink lands with the keyed carry), phased derivatives
            // skipped entirely (membership re-derives at freeze — M4's
            // decision 6). Hook names are instance-qualified.
            if d.has(|c| matches!(c, Clause::CtorArg { .. })) {
                let field = field_of_arrow
                    .get(&(i, a.name.to_string()))
                    .expect("field contributions collected")
                    .clone();
                let hook = format_ident!("__set_{}_{}", inst_snake, a.name);
                let sref = rec_ref(&classes, quote!(self), sc, quote!(self_id));
                let mut unlink_calls = TokenStream2::new();
                let mut insert_calls = TokenStream2::new();
                for i2 in infos.iter().filter(|i2| i2.decl.by.as_ref() == Some(&a.name)) {
                    let Some(d2) = i2.backing else { continue };
                    if d2.is_phased() || card_of_decl(d2) != Cardinality::Many {
                        continue;
                    }
                    let der_root =
                        !classes[class_index_of(i, ent_index(i, &i2.decl.source)?)].owns_records;
                    if d2.has(|c| matches!(c, Clause::OnUnlink { .. })) {
                        let un = format_ident!("__unlink_{}_{}", inst_snake, i2.decl.name);
                        unlink_calls.extend(if der_root {
                            quote!(self.#un(self_id);)
                        } else {
                            quote!(self.#un(old_host, self_id);)
                        });
                    }
                    if d2.has(|c| matches!(c, Clause::OnInsert { .. })) {
                        let ins = format_ident!("__insert_{}_{}", inst_snake, i2.decl.name);
                        insert_calls.extend(if der_root {
                            quote!(self.#ins(self_id);)
                        } else {
                            quote!(self.#ins(value, self_id);)
                        });
                    }
                }
                let old_host_read = if unlink_calls.is_empty() {
                    quote!()
                } else {
                    quote!(let old_host = #sref.#field;)
                };
                store_impl_build.extend(quote! {
                    #[doc(hidden)]
                    #[doc = concat!("re-point hook of arrow `", stringify!(#arrow_doc), "` (instance ", stringify!(#inst_name), "): unlink the by-derived memberships the field determines, write the field, re-link under the new endpoint")]
                    fn #hook(&mut self, self_id: #sid, value: #tid) {
                        #old_host_read
                        #unlink_calls
                        #sref.#field = value;
                        #insert_calls
                    }
                });
                let op = if arrow_users > 1 {
                    format_ident!("set_{}_{}", inst_snake, a.name)
                } else {
                    format_ident!("set_{}", a.name)
                };
                op_names.push(op.to_string());
                store_impl_build.extend(quote! {
                    #[doc = concat!("Set `self_id`'s `", stringify!(#arrow_doc), "` to `value` (instance ", stringify!(#inst_name), "; field-backed `one` arrow).")]
                    pub fn #op(&mut self, self_id: #sid, value: #tid) {
                        self.#hook(self_id, value);
                    }
                });
            }

            // Live-key predicate (keyed): the pure form of the check-then-
            // write insert — M4's staging calls it without performing the
            // write. Derived from the authored lookup body.
            if card_of_decl(d) == Cardinality::Keyed {
                let kid = id_ty_of(i, ent_index(i, a.key.as_ref().unwrap())?);
                let lookup = body_of(d, |c| matches!(c, Clause::Lookup { .. }))
                    .ok_or_else(|| syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!("backing `{}` authors no lookup(a, k) body", d.name),
                    ))?;
                let class_of = class_of_arrow(i, a)?;
                let mut roles =
                    ComposedRoles::new(&classes, i, &store_field_names, d, a, class_of, root_self, classes[tc].name.clone())
                        .at(quote!(self))
                        .with_map(map_of_arrow[&(i, a.name.to_string())].clone())
                        .with_k(quote!(key));
                if root_self {
                    roles = roles.with_a(quote!(()));
                } else {
                    roles = roles.with_a(quote!(self_id));
                }
                let lb = roles::rewrite(lookup, &roles);
                let pred = format_ident!("__{}_{}_key_live", inst_snake, a.name);
                if root_self {
                    store_impl_build.extend(quote! {
                        #[doc(hidden)]
                        #[doc = concat!("live-key predicate of arrow `", stringify!(#arrow_doc), "` (instance ", stringify!(#inst_name), "; store-level map)")]
                        fn #pred(&self, key: #kid) -> bool {
                            (#lb).is_some()
                        }
                    });
                } else {
                    store_impl_build.extend(quote! {
                        #[doc(hidden)]
                        #[doc = concat!("live-key predicate of arrow `", stringify!(#arrow_doc), "` (instance ", stringify!(#inst_name), ") on one self object")]
                        fn #pred(&self, self_id: #sid, key: #kid) -> bool {
                            (#lb).is_some()
                        }
                    });
                }
            }

            // `on_add` hook: per-add growth of store-level per-self state
            // (csr's bucket push). Fired from `add_<A>` orchestration (the
            // ctor loop above); applicability guaranteed an
            // `add_<A>`-orchestrated self (an unfused root was rejected for
            // phased bindings).
            if let Some(body) = body_of(d, |c| matches!(c, Clause::OnAdd { .. })) {
                let d_name = &d.name;
                let label = format!("arrow `{inst_name}.{}`", a.name);
                let hook = format_ident!("__onadd_{}_{}", inst_snake, a.name);
                let a_mixed = Ident2::new("a", Span::mixed_site());
                let mut roles = ComposedRoles::new(&classes, i, &store_field_names, d, a, class_of_arrow(i, a)?, root_self, classes[tc].name.clone())
                    .at(quote!(self));
                if root_self {
                    roles = roles.with_a(quote!(()));
                }
                let body_rw = roles::rewrite(body, &roles);
                let scaffolds = scaffold_lets(body, &label, None, false);
                store_impl_build.extend(quote! {
                    #[doc(hidden)]
                    #[doc = concat!("on-add hook of arrow `", stringify!(#arrow_doc), "` (instance ", stringify!(#inst_name), "; backing `", stringify!(#d_name), "`): per-add growth of store-level per-self state.")]
                    fn #hook(&mut self, #a_mixed: #sid) {
                        #scaffolds
                        #body_rw
                    }
                });
            }

            // `compact` hook (phased backings only): the compaction body
            // runs on the build-phase store at the single `freeze()`, in
            // binding order.
            if let Some(body) = body_of(d, |c| matches!(c, Clause::Compact { .. })) {
                let d_name = &d.name;
                let label = format!("arrow `{inst_name}.{}`", a.name);
                let hook = format_ident!("__compact_{}_{}", inst_snake, a.name);
                let roles = ComposedRoles::new(&classes, i, &store_field_names, d, a, class_of_arrow(i, a)?, root_self, classes[tc].name.clone())
                    .at(quote!(self));
                let body_rw = roles::rewrite(body, &roles);
                let scaffolds = scaffold_lets(body, &label, None, false);
                store_impl_build.extend(quote! {
                    #[doc(hidden)]
                    #[doc = concat!("compact contribution of arrow `", stringify!(#arrow_doc), "` (instance ", stringify!(#inst_name), "; backing `", stringify!(#d_name), "`): runs at the single `freeze()`.")]
                    fn #hook(&mut self) {
                        #scaffolds
                        #body_rw
                    }
                });
            }
        }
    }

    // The builder's read view: id-based getters on the store, taking and
    // returning ids and copies — never handles (no handle can coexist with
    // the `&mut` builder). One field-read getter per bound `one` arrow,
    // one keyed lookup per keyed arrow, both instance-prefixed on
    // cross-instance arrow-name collision; plus the cross-boundary form
    // below. The names are contract: user-written constructor logic
    // (induction, dedup) reads through them.
    for (i, infos) in inst_infos.iter().enumerate() {
        let inst_snake = snake(&members[i].0.to_string());
        let inst_name = &members[i].0;
        for info in infos {
            let Some(d) = info.backing else { continue };
            let a = info.decl;
            let sc = class_index_of(i, ent_index(i, &a.source)?);
            let tc = class_index_of(i, ent_index(i, &a.target)?);
            let root_self = !classes[sc].owns_records;
            let arrow_doc = &a.name;
            match card_of_decl(d) {
                Cardinality::One => {
                    let Some(body) = body_of(d, |c| matches!(c, Clause::Read { .. })) else { continue };
                    let sid = classes[sc].id_name.clone();
                    let tid = id_ty_of(i, ent_index(i, &a.target)?);
                    let getter = if arrow_name_users[&a.name.to_string()] > 1 {
                        format_ident!("{}_{}", inst_snake, a.name)
                    } else {
                        a.name.clone()
                    };
                    op_names.push(getter.to_string());
                    let class_of = class_of_arrow(i, a)?;
                    let roles = ComposedRoles::new(&classes, i, &store_field_names, d, a, class_of, false, classes[tc].name.clone())
                        .at(quote!(self))
                        .with_a(quote!(self_id));
                    let body_rw = roles::rewrite(body, &roles);
                    let view = quote! {
                        #[doc = concat!("Builder read view: the `", stringify!(#arrow_doc), "` field of `self_id` (instance ", stringify!(#inst_name), ").")]
                        pub fn #getter(&self, self_id: #sid) -> #tid {
                            #body_rw
                        }
                    };
                    store_impl_frozen.extend(view.clone());
                    if phased && !d.is_phased() {
                        store_impl_build.extend(view);
                    }
                }
                Cardinality::Keyed => {
                    let Some(body) = body_of(d, |c| matches!(c, Clause::Lookup { .. })) else { continue };
                    let kid = id_ty_of(i, ent_index(i, a.key.as_ref().unwrap())?);
                    let vt = id_ty_of(i, ent_index(i, &a.target)?);
                    let getter = if arrow_name_users[&a.name.to_string()] > 1 {
                        format_ident!("{}_{}_by_key", inst_snake, a.name)
                    } else {
                        format_ident!("{}_by_key", a.name)
                    };
                    op_names.push(getter.to_string());
                    let class_of = class_of_arrow(i, a)?;
                    let mut roles = ComposedRoles::new(&classes, i, &store_field_names, d, a, class_of, root_self, classes[tc].name.clone())
                        .at(quote!(self))
                        .with_map(map_of_arrow[&(i, a.name.to_string())].clone())
                        .with_k(quote!(key));
                    if root_self {
                        roles = roles.with_a(quote!(()));
                    } else {
                        roles = roles.with_a(quote!(self_id));
                    }
                    let body_rw = roles::rewrite(body, &roles);
                    if root_self {
                        let view = quote! {
                            #[doc = concat!("Builder read view: `", stringify!(#arrow_doc), "` keyed lookup (instance ", stringify!(#inst_name), "; root-sourced; no self id).")]
                            pub fn #getter(&self, key: #kid) -> ::core::option::Option<#vt> {
                                #body_rw
                            }
                        };
                        store_impl_frozen.extend(view.clone());
                        if phased && !d.is_phased() {
                            store_impl_build.extend(view);
                        }
                    } else {
                        let sid = classes[sc].id_name.clone();
                        let view = quote! {
                            #[doc = concat!("Builder read view: `", stringify!(#arrow_doc), "` keyed lookup on one self object (instance ", stringify!(#inst_name), ").")]
                            pub fn #getter(&self, self_id: #sid, key: #kid) -> ::core::option::Option<#vt> {
                                #body_rw
                            }
                        };
                        store_impl_frozen.extend(view.clone());
                        if phased && !d.is_phased() {
                            store_impl_build.extend(view);
                        }
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
                if !field_of_arrow.contains_key(&(yi, yb.name.to_string())) {
                    continue;
                }
                let getter = format_ident!("{}_{}", xa.name, yb.name);
                op_names.push(getter.to_string());
                let sid = classes[class_index_of(yi, ei)].id_name.clone();
                let tid = id_ty_of(yi, ent_index(yi, &yb.target)?);
                let sc = class_index_of(yi, ei);
                let field = field_of_arrow[&(yi, yb.name.to_string())].clone();
                let sref = rec_ref(&classes, quote!(self), sc, quote!(self_id));
                let through = &xa.name;
                let read = &yb.name;
                let xinst = &members[xi].0;
                let yinst = &members[yi].0;
                store_impl_frozen.extend(quote! {
                    #[doc = concat!("Builder read view: the `", stringify!(#read), "` field (instance ", stringify!(#yinst), "), where the id arrived through the `", stringify!(#through), "` reference closure (instance ", stringify!(#xinst), ").")]
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
        store_impl_frozen.extend(quote! {
            #[doc = concat!("Root handle: the \"object\" view of this store for the unfused root `", stringify!(#h), "`.")]
            pub fn #acc(&self) -> #h<'_> {
                #h { store: self }
            }
        });
    }

    // ---- validate(): one doc-hidden __validate_<inst>_<arrow> method per
    // bound backing, summoned in binding order (then defaulted arrows in
    // schema order). A uniform range pre-pass over contributed `Option<id>`
    // fields runs before the authored body and returns early on damage, so
    // authored walks may assume links in range and use `rec(x)` freely.
    // Value-in-arena test for an id `v` of class cj (path-aware: a path id
    // is in-bounds only within the arena its host component names).
    let out_of_range = |cj: usize, v: TokenStream2| -> TokenStream2 {
        match classes[cj].loc {
            ClassLoc::Root => {
                let ta = &classes[cj].arena_fld;
                quote!((#v).0 as usize >= self.#ta.len())
            }
            ClassLoc::Embedded(hc) => {
                let ha = &classes[hc].arena_fld;
                let ta = &classes[cj].arena_fld;
                quote!(
                    (#v).0.0 as usize >= self.#ha.len()
                        || (#v).1 as usize >= self.#ha[(#v).0.0 as usize].#ta.len()
                )
            }
        }
    };
    // Enumerate every record of class ci with `self_id` bound to its id
    // and `srec` to its record data (host-nested for embedded classes).
    let enum_recs = |ci: usize, inner: TokenStream2| -> TokenStream2 {
        match classes[ci].loc {
            ClassLoc::Root => {
                let a = &classes[ci].arena_fld;
                let idt = &classes[ci].id_name;
                quote! {
                    for (ri_, srec_) in self.#a.iter().enumerate() {
                        let self_id = #idt(ri_ as u32);
                        let srec = srec_;
                        #inner
                    }
                }
            }
            ClassLoc::Embedded(hc) => {
                let ha = &classes[hc].arena_fld;
                let a = &classes[ci].arena_fld;
                let hid = &classes[hc].id_name;
                let idt = &classes[ci].id_name;
                quote! {
                    for (hi_, hrec_) in self.#ha.iter().enumerate() {
                        for (ri_, srec_) in hrec_.#a.iter().enumerate() {
                            let self_id = #idt(#hid(hi_ as u32), ri_ as u32);
                            let srec = srec_;
                            #inner
                        }
                    }
                }
            }
        }
    };
    // The class an `Option<...>` contribution's element type names (by id
    // type name); `None` when the element is not a class id (skipped — the
    // pre-pass range-walks ids only).
    let elem_class_of = |fc: &FieldContribution| -> Option<usize> {
        let ty = fc.ty.to_string().replace(' ', "");
        let inner = ty
            .strip_prefix("Option<")
            .and_then(|s| s.strip_suffix('>'))?;
        classes.iter().position(|c| c.id_name.to_string() == inner)
    };
    let mut validate_order: Vec<String> = config.bindings.iter().map(|b| b.key_str()).collect();
    for (i, infos) in inst_infos.iter().enumerate() {
        for info in infos {
            if info.backing.is_some() && info.decl.sum.is_none() {
                let key = format!("{}.{}", members[i].0, info.decl.name);
                if !validate_order.contains(&key) {
                    validate_order.push(key);
                }
            }
        }
    }
    let mut validate_methods = TokenStream2::new();
    let mut validate_calls = TokenStream2::new();
    for key in &validate_order {
        let Some((i, info)) = inst_infos
            .iter()
            .enumerate()
            .find_map(|(i, infos)| {
                infos
                    .iter()
                    .find(|x| format!("{}.{}", members[i].0, x.decl.name) == *key)
                    .map(|x| (i, x))
            })
        else {
            continue;
        };
        let Some(d) = info.backing else { continue };
        let a = info.decl;
        let sc = class_index_of(i, ent_index(i, &a.source)?);
        let tc = class_index_of(i, ent_index(i, &a.target)?);
        let root_self = !classes[sc].owns_records;
        let label = format!("arrow `{}`", key);
        let wanted_variant = match card_of_decl(d) {
            Cardinality::Many => {
                if a.by.is_some() {
                    ValidateVariant::By
                } else {
                    ValidateVariant::Primitive
                }
            }
            _ => ValidateVariant::Plain,
        };
        // A `Plain` body (membership-agnostic — csr's offsets check) serves
        // any membership kind; the wanted variant wins when authored.
        let Some(vbody) = d
            .clause(|c| matches!(c, Clause::Validate { variant, .. } if *variant == wanted_variant))
            .or_else(|| d.clause(|c| matches!(c, Clause::Validate { variant, .. } if *variant == ValidateVariant::Plain)))
            .map(|c| match c {
                Clause::Validate { body, .. } => body,
                _ => unreachable!(),
            })
        else {
            continue; // arena: nothing — existence cannot dangle
        };
        let det_label = a.by.as_ref().map(|b| b.to_string()).unwrap_or_default();
        let det_field = a.by.as_ref().map(|det_name| {
            field_of_arrow
                .get(&(i, det_name.to_string()))
                .cloned()
                .unwrap_or_else(|| a.name.clone())
        });
        let class_of = class_of_arrow(i, a)?;
        let mut roles = ComposedRoles::new(&classes, i, &store_field_names, d, a, class_of, root_self, classes[tc].name.clone())
            .at(quote!(self))
            .with_det(det_field.unwrap_or_else(|| a.name.clone()));
        if card_of_decl(d) == Cardinality::Keyed {
            roles = roles.with_map(map_of_arrow[&(i, a.name.to_string())].clone());
        }
        if root_self {
            roles = roles.with_a(quote!(()));
        }
        let body_rw = roles::rewrite(vbody, &roles);
        let scaffolds = scaffold_lets(vbody, &label, Some(&det_label), true);

        // The uniform range pre-pass over this binding's contributed
        // `Option<id>` fields (the (b)-class half — derivable from the
        // declaration, so authored bodies never hand-roll it).
        let mut pre_pass = TokenStream2::new();
        let errs = Ident2::new("errs", Span::mixed_site());
        let arrow_label_id = Ident2::new("arrow_label", Span::mixed_site());
        let all_contrib = classes
            .iter()
            .enumerate()
            .filter(|(_, c)| c.owns_records)
            .flat_map(|(cj, c)| c.fields.iter().map(move |f| (Some(cj), f)))
            .chain(store_fields.iter().map(|f| (None, f)));
        for (owner, fc) in all_contrib {
            if fc.inst != i || fc.arrow != a.name || !fc.option_id {
                continue;
            }
            let Some(elem) = elem_class_of(fc) else { continue };
            let f = &fc.field;
            let what = if fc.side_a { "head" } else { "link" };
            let bad = out_of_range(elem, quote!(n));
            match owner {
                Some(cj) => {
                    let fmt = format!(
                        "{{}}: `{{}}` {what} of {{:?}} points at {{:?}}, which is not a live record"
                    );
                    pre_pass.extend(enum_recs(cj, quote! {
                        if let ::core::option::Option::Some(n) = srec.#f {
                            if #bad {
                                #errs.push(::std::format!(
                                    #fmt, #arrow_label_id, stringify!(#f), self_id, n,
                                ));
                            }
                        }
                    }));
                }
                None => {
                    let fmt = format!(
                        "{{}}: `{{}}` {what} of the store points at {{:?}}, which is not a live record"
                    );
                    pre_pass.extend(quote! {
                        if let ::core::option::Option::Some(n) = self.#f {
                            if #bad {
                                #errs.push(::std::format!(
                                    #fmt, #arrow_label_id, stringify!(#f), n,
                                ));
                            }
                        }
                    });
                }
            }
        }
        let method = format_ident!(
            "__validate_{}_{}",
            snake(&members[i].0.to_string()),
            a.name
        );
        validate_methods.extend(quote! {
            #[doc(hidden)]
            #[doc = concat!("validate contribution of ", #label, ": the authored body with the uniform range pre-pass.")]
            fn #method(&self) -> ::core::result::Result<(), ::std::string::String> {
                #scaffolds
                #pre_pass
                if !#errs.is_empty() {
                    return ::core::result::Result::Err(#errs.join("; "));
                }
                #body_rw
                if #errs.is_empty() {
                    ::core::result::Result::Ok(())
                } else {
                    ::core::result::Result::Err(#errs.join("; "))
                }
            }
        });
        validate_calls.extend(quote! {
            if let ::core::result::Result::Err(e) = self.#method() {
                errs.push(e);
            }
        });
    }
    let validate_method = quote! {
        #[doc = "Structural validation: the canonical runtime inventory, one authored contribution per bound backing (key existence, value-in-arena, value-host agreement for keyed maps; endpoint ranges and the intrusive-ownership contract for lists — the partition walk for primitive membership, determinant consistency for `by`-derived membership). M4's flush calls this unconditionally."]
        pub fn validate(&self) -> ::core::result::Result<(), ::std::string::String> {
            let mut errs: ::std::vec::Vec<::std::string::String> = ::std::vec::Vec::new();
            #validate_calls
            if errs.is_empty() {
                ::core::result::Result::Ok(())
            } else {
                ::core::result::Result::Err(errs.join("; "))
            }
        }
    };

    // freeze()/finish() (phased): compact every phased backing in binding
    // order at the single boundary, then rebuild the store with the phase
    // marker swapped — build-attributed fields reset (their Frozen type is
    // `()`, and the compact body already drained them), everything else
    // moved over.
    let compact_hooks: Vec<Ident2> = validate_order
        .iter()
        .filter_map(|key| {
            inst_infos.iter().enumerate().find_map(|(i, infos)| {
                let info = infos
                    .iter()
                    .find(|x| format!("{}.{}", members[i].0, x.decl.name) == *key)?;
                let d = info.backing?;
                d.is_phased()
                    .then(|| format_ident!("__compact_{}_{}", snake(&members[i].0.to_string()), info.decl.name))
            })
        })
        .collect();
    let freeze_fields = classes
        .iter()
        .filter(|cl| cl.owns_records && cl.loc == ClassLoc::Root)
        .map(|cl| {
            let a = &cl.arena_fld;
            quote!(#a: self.#a,)
        })
        .chain(store_fields.iter().map(|fc| {
            let n = &fc.field;
            match fc.phase {
                Some(crate::backing::PhaseLabel::Build) => quote!(#n: ::std::default::Default::default(),),
                _ => quote!(#n: self.#n,),
            }
        }))
        .collect::<Vec<_>>();
    let freeze_finish = quote! {
        #[doc = "The single builder boundary: compact the phased backings in binding order, consume the build-phase store, and return the frozen store. Insert-after-freeze is unrepresentable — freeze consumes."]
        pub fn freeze(mut self) -> #store<Frozen> {
            #(self.#compact_hooks();)*
            #store {
                #(#freeze_fields)*
                _phase: ::core::marker::PhantomData,
            }
        }

        #[doc = "Freeze, then validate — M4's flush ordering: compaction (flush step 6) precedes validation (step 7)."]
        pub fn finish(self) -> ::core::result::Result<#store<Frozen>, ::std::string::String> {
            let frozen = self.freeze();
            frozen.validate()?;
            ::core::result::Result::Ok(frozen)
        }
    };

    if phased {
        out.extend(quote! {
            impl #store<Build> {
                #store_impl_build
                #freeze_finish
            }
            impl #store<Frozen> {
                #store_impl_frozen
                #validate_methods
                #validate_method
            }
        });
    } else {
        out.extend(quote! {
            impl #store {
                #store_impl_build
                #store_impl_frozen
                #validate_methods
                #validate_method
            }
        });
    }

    // The store type as handles and iterators hold it: bare when unphased
    // (M2 shape); `#store<Frozen>` when phased — handles exist only for the
    // frozen phase.
    let store_ref: TokenStream2 = if phased { quote!(#store<Frozen>) } else { quote!(#store) };

    // ---- handles ----
    for ci in 0..classes.len() {
        let cl = &classes[ci];
        let h = &cl.name;
        let (body, id_fn, eq_body, hash_body, dbg_body) = if !cl.owns_records {
            (
                quote!(store: &'g #store_ref),
                quote!(),
                quote!(::std::ptr::eq(self.store, other.store)),
                quote!((self.store as *const #store_ref).hash(state);),
                {
                    quote!(f.debug_tuple(stringify!(#h)).finish())
                },
            )
        } else {
            match cl.loc {
                ClassLoc::Root => {
                    let idt = &cl.id_name;
                    (
                        quote!(store: &'g #store_ref, id: #idt),
                        quote! {
                            #[doc = "Index of this entity in its arena."]
                            pub fn id(self) -> #idt {
                                self.id
                            }
                        },
                        quote!(self.id == other.id && ::std::ptr::eq(self.store, other.store)),
                        quote! {
                            (self.store as *const #store_ref).hash(state);
                            self.id.hash(state);
                        },
                        quote!(write!(f, "{}({:?})", stringify!(#h), self.id)),
                    )
                }
                ClassLoc::Embedded(host) => {
                    let hid = &classes[host].id_name;
                    let idt = &cl.id_name;
                    (
                        quote!(store: &'g #store_ref, host: #hid, id: u32),
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
                            (self.store as *const #store_ref).hash(state);
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

    // The full path id of the method's `self` handle (an embedded handle
    // stores host + raw leaf; the roles take whole ids).
    let self_id_expr = |sc: usize| -> TokenStream2 {
        if !classes[sc].owns_records {
            return quote!(());
        }
        match classes[sc].loc {
            ClassLoc::Root => quote!(self.id),
            ClassLoc::Embedded(_) => {
                let idt = &classes[sc].id_name;
                quote!(#idt(self.host, self.id))
            }
        }
    };

    // A bound many-arrow sibling's iterate body, rewritten off a handle
    // binding `self` — the shared shape of the trait method and a sum's
    // sibling expressions (inlined so sums never depend on trait imports).
    let sibling_iterate_expr = |i: usize, sib: &ArrowInfo| -> syn::Result<TokenStream2> {
        let d = sib.backing.expect("sum siblings are bound many arrows");
        let a = sib.decl;
        let body = body_of(d, |c| matches!(c, Clause::Iterate { .. }))
            .ok_or_else(|| syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("backing `{}` authors no iterate(a) body", d.name),
            ))?;
        let sc = class_index_of(i, ent_index(i, &a.source)?);
        let tc = class_index_of(i, ent_index(i, &a.target)?);
        let it = iter_name(i, &a.name);
        let class_of = class_of_arrow(i, a)?;
        let mut roles = ComposedRoles::new(&classes, i, &store_field_names, d, a, class_of, false, classes[tc].name.clone())
            .at(quote!(self.store))
            .with_walk_ty(it);
        let a_sub = self_id_expr(sc);
        if !classes[sc].owns_records {
            roles = roles.with_a(quote!(()));
        } else {
            roles = roles.with_a(a_sub.clone());
        }
        if matches!(classes[tc].loc, ClassLoc::Embedded(_)) {
            roles = roles.with_walk_host(a_sub);
        }
        Ok(roles::rewrite(body, &roles))
    };

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

            // Derived (`:= sum`) arrows chain their siblings' iterators.
            if let Some((ua, ub)) = &d.sum {
                let ia = infos.iter().find(|x| &x.decl.name == ua).ok_or_else(|| {
                    syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!("sum names unknown sibling `{ua}`"),
                    )
                })?;
                let ib = infos.iter().find(|x| &x.decl.name == ub).ok_or_else(|| {
                    syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!("sum names unknown sibling `{ub}`"),
                    )
                })?;
                let ita = iter_name(i, &ia.decl.name);
                let itb = iter_name(i, &ib.decl.name);
                let ea = sibling_iterate_expr(i, ia)?;
                let eb = sibling_iterate_expr(i, ib)?;
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

            let Some(decl) = info.backing else { continue };
            let root_self = !classes[sc].owns_records;
            match card_of_decl(decl) {
                Cardinality::Many => {
                    let backing_name = &decl.name;
                    let walk_ty = it.clone();
                    let class_of = class_of_arrow(i, d)?;
                    // The Walk struct: authored fields (renamed types,
                    // mixed-site spans) + the reserved store ref (+ the
                    // reserved host at embedded targets — r4-advance).
                    let walk_fields = decl
                        .clause(|c| matches!(c, Clause::Iter { .. }))
                        .map(|c| match c {
                            Clause::Iter { fields, .. } => fields
                                .iter()
                                .map(|(n, t)| {
                                    let ty_roles = ComposedRoles::new(
                                        &classes,
                                        i,
                                        &store_field_names,
                                        decl,
                                        d,
                                        class_of,
                                        false,
                                        classes[tc].name.clone(),
                                    );
                                    let ty = roles::rewrite_ty(&t.to_token_stream(), &ty_roles);
                                    let field = Ident2::new(&n.to_string(), Span::mixed_site());
                                    quote!(#field: #ty,)
                                })
                                .collect::<Vec<_>>(),
                            _ => unreachable!(),
                        })
                        .unwrap_or_default();
                    let host_field = match classes[tc].loc {
                        ClassLoc::Embedded(hc) => {
                            let hid = &classes[hc].id_name;
                            Some(quote!(host: #hid,))
                        }
                        ClassLoc::Root => None,
                    };
                    let adv = body_of(decl, |c| matches!(c, Clause::Advance { .. }))
                        .ok_or_else(|| syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!("backing `{}` authors no advance(w) body", decl.name),
                        ))?;
                    let adv_roles = ComposedRoles::new(&classes, i, &store_field_names, decl, d, class_of, root_self, classes[tc].name.clone())
                        .at(quote!(self.store))
                        .with_w(quote!(self))
                        .with_walk_ty(walk_ty.clone());
                    let adv_roles = match classes[tc].loc {
                        ClassLoc::Embedded(_) => adv_roles.with_walk_host(quote!(self.host)),
                        ClassLoc::Root => adv_roles,
                    };
                    let adv_rw = roles::rewrite(adv, &adv_roles);
                    let iter_doc = match decl
                        .clause(|c| matches!(c, Clause::IterationOrder { .. }))
                        .map(|c| match c {
                            Clause::IterationOrder { value, .. } => value.to_string(),
                            _ => unreachable!(),
                        })
                        .as_deref()
                    {
                        Some("forward_insertion") => "forward insertion order",
                        _ => "reverse insertion order",
                    };
                    iter_defs.extend(quote! {
                        #[doc = concat!("Iterator of `", stringify!(#arrow_doc_name), "' (instance ", stringify!(#inst_name), "; backing `", stringify!(#backing_name), "`): authored Walk, ", #iter_doc, ".")]
                        pub struct #it<'g> {
                            store: &'g #store_ref,
                            #host_field
                            #(#walk_fields)*
                        }
                        impl<'g> ::core::iter::Iterator for #it<'g> {
                            type Item = #tgt_h<'g>;
                            fn next(&mut self) -> ::core::option::Option<#tgt_h<'g>> {
                                #adv_rw
                            }
                        }
                    });
                    let iterate = body_of(decl, |c| matches!(c, Clause::Iterate { .. }))
                        .ok_or_else(|| syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!("backing `{}` authors no iterate(a) body", decl.name),
                        ))?;
                    let mut roles = ComposedRoles::new(&classes, i, &store_field_names, decl, d, class_of, root_self, classes[tc].name.clone())
                        .at(quote!(self.store))
                        .with_walk_ty(walk_ty);
                    let a_sub = self_id_expr(sc);
                    if root_self {
                        roles = roles.with_a(quote!(()));
                    } else {
                        roles = roles.with_a(a_sub.clone());
                    }
                    if matches!(classes[tc].loc, ClassLoc::Embedded(_)) {
                        roles = roles.with_walk_host(a_sub);
                    }
                    let ie = roles::rewrite(iterate, &roles);
                    let gat = meta.gat.as_ref().unwrap();
                    trait_impls.extend(quote! {
                        impl<'g> #trait_path<#marker_path> for #self_h<'g> {
                            type #assoc = #tgt_h<'g>;
                            type #gat<'a> = #it<'g> where Self: 'a;
                            fn #m(&self) -> #it<'g> {
                                #ie
                            }
                        }
                    });
                }
                Cardinality::One => {
                    let body = body_of(decl, |c| matches!(c, Clause::Read { .. }))
                        .ok_or_else(|| syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!("backing `{}` authors no read(a) body", decl.name),
                        ))?;
                    let class_of = class_of_arrow(i, d)?;
                    let mut roles = ComposedRoles::new(&classes, i, &store_field_names, decl, d, class_of, root_self, classes[tc].name.clone())
                        .at(quote!(self.store));
                    if root_self {
                        roles = roles.with_a(quote!(()));
                    } else {
                        roles = roles.with_a(self_id_expr(sc));
                    }
                    let body_rw = roles::rewrite(body, &roles);
                    trait_impls.extend(quote! {
                        impl<'g> #trait_path<#marker_path> for #self_h<'g> {
                            type #assoc = #tgt_h<'g>;
                            fn #m(&self) -> #tgt_h<'g> {
                                #tgt_h {
                                    store: self.store,
                                    id: #body_rw,
                                }
                            }
                        }
                    });
                }
                // Keyed partial: one authored lookup; the associated-type
                // bindings name the impl lifetime ('g — the E0637-pinned
                // emission), and a `<->`-closed key is the partner's plain
                // id (no wrapper newtype).
                Cardinality::Keyed => {
                    let kid = id_ty_of(i, ent_index(i, d.key.as_ref().unwrap())?);
                    let key_assoc = meta
                        .key
                        .as_ref()
                        .expect("keyed roots publish the key assoc name");
                    let lookup = body_of(decl, |c| matches!(c, Clause::Lookup { .. }))
                        .ok_or_else(|| syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!("backing `{}` authors no lookup(a, k) body", decl.name),
                        ))?;
                    let class_of = class_of_arrow(i, d)?;
                    let mut roles = ComposedRoles::new(&classes, i, &store_field_names, decl, d, class_of, root_self, classes[tc].name.clone())
                        .at(quote!(self.store))
                        .with_map(map_of_arrow[&(i, d.name.to_string())].clone())
                        .with_k(quote!(key));
                    if root_self {
                        roles = roles.with_a(quote!(()));
                    } else {
                        roles = roles.with_a(self_id_expr(sc));
                    }
                    let body_rw = roles::rewrite(lookup, &roles);
                    let mk_handle = match classes[tc].loc {
                        ClassLoc::Root => quote!(#tgt_h { store: self.store, id }),
                        ClassLoc::Embedded(_) => quote!(#tgt_h { store: self.store, host: id.0, id: id.1 }),
                    };
                    trait_impls.extend(quote! {
                        impl<'g> #trait_path<#marker_path> for #self_h<'g> {
                            type #assoc = #tgt_h<'g>;
                            type #key_assoc = #kid;
                            fn #m(&self, key: #kid) -> ::core::option::Option<#tgt_h<'g>> {
                                #body_rw.map(|id| #mk_handle)
                            }
                        }
                    });
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

    // ---- M4 step 8/9: the transaction (write side) ----
    // The composed codegen builds the same normalized TxModel the flat
    // codegen does, so the whole flush pipeline exists in one place. Gate:
    // non-phased, every fusion class at root level or embedded at depth 1
    // (composition already rejects deeper), and every bound arrow's carry
    // classifiable by the flat taxonomy — with the nested-store
    // restrictions step 9 pins: an embedded class's arrows are existence
    // or field-one only (embedded intrusive chains and per-host keyed maps
    // carry per-host state and land with the keyed-carry step), a ctor-arg
    // target must be a root-level entity (a path-id remap has no meaning
    // this step), and an embedded class with on_add hooks gates off (the
    // skeleton builds embedded records inside the host's field init and
    // cannot fire per-record store hooks there). The composed Tx is
    // inherent-surface in this step: the write-TRAIT impls wait for the
    // composed write-trait surface (per-entity traits would need
    // member-pattern paths the composed schema does not carry; the
    // arrow-named impls land beside them, pinned by tests) — `HandleToRef`
    // impls emit regardless (the runtime trait needs only the per-instance
    // markers finish_compose already mints).
    let mut tx_ok = !phased
        && classes
            .iter()
            .all(|c| matches!(c.loc, ClassLoc::Root | ClassLoc::Embedded(_)));
    // An arrow's unique local basis: the bare arrow name, or the
    // instance-qualified spelling exactly where the public write ops take
    // the prefix (cross-instance arrow-name collision) — so the Tx verbs
    // (`insert_<basis>`, `set_<basis>`) match the store's op names.
    let basis_of = |i: usize, arrow: &Ident2| -> Ident2 {
        if arrow_name_users[&arrow.to_string()] > 1 {
            format_ident!("{}_{}", snake(&members[i].0.to_string()), arrow)
        } else {
            arrow.clone()
        }
    };
    // Ctor-arg target resolution: a field's rewritten type names the
    // target class's id type at root level.
    let class_by_id_name: HashMap<String, usize> = classes
        .iter()
        .enumerate()
        .filter(|(_, c)| c.owns_records)
        .map(|(ci, c)| (c.id_name.to_string(), ci))
        .collect();
    let ty_str = |f: &crate::backing::ContribField| f.ty.to_token_stream().to_string().replace(' ', "");
    let mut tx_intrusive: Vec<crate::tx::TxIntrusive> = Vec::new();
    let mut tx_keyed: Vec<crate::tx::TxKeyed> = Vec::new();
    let mut tx_inserts: Vec<crate::tx::TxInsert> = Vec::new();
    let mut tx_sets: Vec<crate::tx::TxSet> = Vec::new();

    // Carry classification, per bound arrow (the flat twin's checks over
    // the placement-independent declaration; the emitted field names come
    // from the placement's collision-resolved contributions).
    'arrows: for (i, infos) in inst_infos.iter().enumerate() {
        for info in infos {
            let Some(d) = info.backing else { continue }; // `:= sum` reads carry nothing
            let a = info.decl;
            let sc = class_index_of(i, ent_index(i, &a.source)?);
            let tc = class_index_of(i, ent_index(i, &a.target)?);
            let root_self = !classes[sc].owns_records;
            let inst_snake = snake(&members[i].0.to_string());
            match card_of_decl(d) {
                Cardinality::One => {
                    let a_fields: Vec<&crate::backing::ContribField> = on_clauses(d, OnTarget::A)
                        .iter()
                        .flat_map(|(_, fs)| fs.iter())
                        .collect();
                    // A ctor-arg TARGET must be root-level: a path-id remap
                    // (host translation + per-host index) is the
                    // keyed-carry step's work. An embedded SELF is fine —
                    // the field rides the embedded record.
                    let ok = a_fields.len() == 1
                        && a_fields[0].formula == crate::backing::NameFormula::Arrow
                        && ty_str(a_fields[0]) == "BId"
                        && d.has(|c| matches!(c, Clause::CtorArg { .. }))
                        && classes[tc].loc == ClassLoc::Root;
                    if !ok {
                        tx_ok = false;
                        break 'arrows;
                    }
                }
                Cardinality::Many => {
                    if d.has(|c| matches!(c, Clause::MembershipExistence { .. })) {
                        if crate::codegen::contributed_fields(d).next().is_some() {
                            tx_ok = false;
                            break 'arrows;
                        }
                        continue;
                    }
                    // Intrusive chains at an embedded placement carry
                    // per-host state — the keyed-carry step's.
                    if classes[sc].loc != ClassLoc::Root || classes[tc].loc != ClassLoc::Root {
                        tx_ok = false;
                        break 'arrows;
                    }
                    let a_fields: Vec<&crate::backing::ContribField> = on_clauses(d, OnTarget::A)
                        .iter()
                        .flat_map(|(_, fs)| fs.iter())
                        .collect();
                    let b_fields: Vec<&crate::backing::ContribField> = on_clauses(d, OnTarget::B)
                        .iter()
                        .flat_map(|(_, fs)| fs.iter())
                        .collect();
                    let ok = a_fields.len() == 1
                        && b_fields.len() == 1
                        && on_clauses(d, OnTarget::Store).is_empty()
                        && a_fields[0].formula == crate::backing::NameFormula::Stem
                        && b_fields[0].formula == crate::backing::NameFormula::Stem
                        && ty_str(a_fields[0]) == "Option<BId>"
                        && ty_str(b_fields[0]) == "Option<BId>"
                        && d.has(|c| matches!(c, Clause::OnInsert { .. }));
                    if !ok {
                        tx_ok = false;
                        break 'arrows;
                    }
                    let head_logical = a_fields[0].logical.to_string();
                    let head_field = if root_self {
                        store_field_names[&(i, a.name.to_string())][&head_logical].clone()
                    } else {
                        classes[sc]
                            .fields
                            .iter()
                            .find(|fc| fc.inst == i && fc.arrow == a.name && fc.side_a)
                            .expect("the A-side head contribution was collected")
                            .field
                            .clone()
                    };
                    let member_link = classes[tc]
                        .fields
                        .iter()
                        .find(|fc| fc.inst == i && fc.arrow == a.name && !fc.side_a)
                        .expect("the B-side link contribution was collected")
                        .field
                        .clone();
                    tx_intrusive.push(crate::tx::TxIntrusive {
                        arrow: basis_of(i, &a.name),
                        host: (!root_self).then(|| classes[sc].name.clone()),
                        head_field,
                        member: classes[tc].name.clone(),
                        member_link,
                    });
                }
                Cardinality::Keyed => {
                    // The keyed carry (steps 8/11). Store-level: the self
                    // owns no records (the map rides the store) and the
                    // key and target are root-level. Per-record (the
                    // keyed-carry step): the map rides the self class's
                    // records; the key stays root-level (a path-id key
                    // has no ref form and no acceptance need), and the
                    // target may be root-level or embedded IN THE SELF
                    // CLASS (value-host agreement puts a map's targets in
                    // its owner's own embedded arena — the keyed-carry
                    // probe's Hierarchy shape: `sub.subnode` keyed by a
                    // root-level MainNode fused to `inner.Node`).
                    let Some(k) = &a.key else {
                        tx_ok = false;
                        break 'arrows;
                    };
                    let kc = class_index_of(i, ent_index(i, k)?);
                    let map_ok = map_of_arrow.contains_key(&(i, a.name.to_string()))
                        && d.has(|c| matches!(c, Clause::OnInsert { .. }))
                        && body_of(d, |c| matches!(c, Clause::Lookup { .. })).is_some();
                    let shape_ok = map_ok
                        && classes[kc].loc == ClassLoc::Root
                        && (root_self && classes[tc].loc == ClassLoc::Root
                            || !root_self
                                && match classes[tc].loc {
                                    ClassLoc::Root => true,
                                    // Embedded in the self class only.
                                    ClassLoc::Embedded(h) => h == sc,
                                });
                    if !shape_ok {
                        tx_ok = false;
                        break 'arrows;
                    }
                    let member_host = match classes[tc].loc {
                        ClassLoc::Root => None,
                        ClassLoc::Embedded(h) => Some(classes[h].name.clone()),
                    };
                    tx_keyed.push(crate::tx::TxKeyed {
                        arrow: basis_of(i, &a.name),
                        host: (!root_self).then(|| classes[sc].name.clone()),
                        key_entity: classes[kc].name.clone(),
                        member: classes[tc].name.clone(),
                        member_host,
                        map_field: map_of_arrow[&(i, a.name.to_string())].clone(),
                        hook: format_ident!("__insert_{}_{}", inst_snake, a.name),
                        key_live: format_ident!("__{}_{}_key_live", inst_snake, a.name),
                        verb: None,
                    });
                }
            }
        }
    }

    // Entities: one per record-owning class, in class order. Ctor args
    // are the class's CtorArg field contributions in contribution order —
    // exactly the add op's parameter order, so replay is positional.
    let mut tx_entities: Vec<crate::tx::TxEntity> = Vec::new();
    if tx_ok {
        for ci in 0..classes.len() {
            if !classes[ci].owns_records {
                continue;
            }
            let cl = &classes[ci];
            let mut ctor_args: Vec<crate::tx::TxCtorArg> = Vec::new();
            let mut fields: Vec<crate::tx::TxField> = Vec::new();
            for fc in &cl.fields {
                match &fc.init {
                    FieldInit::CtorArg(arg) => {
                        let Some(&tci) = class_by_id_name.get(&fc.ty.to_string().replace(' ', ""))
                        else {
                            tx_ok = false;
                            break;
                        };
                        // A ctor-arg target must be a root-level entity:
                        // remapping a path id (host translation + per-host
                        // index) is the keyed-carry step's work.
                        if classes[tci].loc != ClassLoc::Root {
                            tx_ok = false;
                            break;
                        }
                        let target = classes[tci].name.clone();
                        ctor_args.push(crate::tx::TxCtorArg {
                            arg: arg.clone(),
                            basis: basis_of(fc.inst, &fc.arrow),
                            target_entity: target.clone(),
                            target_ref_alias: format_ident!("{}Ref", target),
                            target_into_trait: format_ident!("Into{}Arg", target),
                            target_into_fn: format_ident!("into_{}_ref", snake(&target.to_string())),
                        });
                        fields.push(crate::tx::TxField {
                            emitted: fc.field.clone(),
                            init: crate::tx::TxFieldInit::CtorArg {
                                target: target.clone(),
                                arrow: basis_of(fc.inst, &fc.arrow),
                            },
                        });
                    }
                    FieldInit::None_ => fields.push(crate::tx::TxField {
                        emitted: fc.field.clone(),
                        init: crate::tx::TxFieldInit::None_,
                    }),
                    FieldInit::New => fields.push(crate::tx::TxField {
                        emitted: fc.field.clone(),
                        init: crate::tx::TxFieldInit::New,
                    }),
                }
            }
            if !tx_ok {
                break;
            }
            // A host record's embedded arenas ride as NestedArena fields
            // (same order as the store's own record literal: contributions
            // first, then embedded arenas).
            for d in classes.iter() {
                if let ClassLoc::Embedded(h) = d.loc {
                    if h == ci && d.owns_records {
                        fields.push(crate::tx::TxField {
                            emitted: d.arena_fld.clone(),
                            init: crate::tx::TxFieldInit::NestedArena(d.name.clone()),
                        });
                    }
                }
            }
            // `on_add` hooks firing at construction of this class — the
            // same member-arrows loop the add op itself uses, so replay
            // fires exactly what direct construction fires. An embedded
            // class's records are built inside the host's field init,
            // where per-record store hooks cannot fire — gate off instead.
            let mut onadd_hooks: Vec<Ident2> = Vec::new();
            for &(mi, _me) in &cl.members {
                for info in &inst_infos[mi] {
                    let Some(bd) = info.backing else { continue };
                    if !bd.has(|c| matches!(c, Clause::OnAdd { .. })) {
                        continue;
                    }
                    if class_index_of(mi, ent_index(mi, &info.decl.source)?) != ci {
                        continue;
                    }
                    onadd_hooks
                        .push(format_ident!("__onadd_{}_{}", snake(&members[mi].0.to_string()), info.decl.name));
                }
            }
            if !onadd_hooks.is_empty() && classes[ci].loc != ClassLoc::Root {
                tx_ok = false;
                break;
            }
            // The per-entity marker is the canonical member's per-instance
            // entity marker (finish_compose mints one per closed element).
            let (m0i, m0e) = cl.members[0];
            let m0_snake = format_ident!("{}", snake(&members[m0i].0.to_string()));
            let m0_ent = &members[m0i].2.entities[m0e].name;
            tx_entities.push(crate::tx::TxEntity {
                name: cl.name.clone(),
                arena: cl.arena_fld.clone(),
                host: match cl.loc {
                    ClassLoc::Root => None,
                    ClassLoc::Embedded(h) => Some(classes[h].name.clone()),
                },
                ctor_args,
                onadd_hooks,
                fields,
                marker_path: quote!(#marker_base_ts::#marker_mod::markers::#m0_snake::#m0_ent),
                add_trait: None,
                remove_trait: None,
            });
        }
    }

    if tx_ok {
        // Store-level fields (a store-level ctor-arg is unrepresentable —
        // applicability rejected it at an unfused root).
        let mut tx_store_fields: Vec<crate::tx::TxField> = Vec::new();
        for fc in &store_fields {
            match fc.init {
                FieldInit::CtorArg(_) => {
                    tx_ok = false;
                    break;
                }
                FieldInit::None_ => tx_store_fields.push(crate::tx::TxField {
                    emitted: fc.field.clone(),
                    init: crate::tx::TxFieldInit::None_,
                }),
                FieldInit::New => tx_store_fields.push(crate::tx::TxField {
                    emitted: fc.field.clone(),
                    init: crate::tx::TxFieldInit::New,
                }),
            }
        }
        if tx_ok {
            // Primitive-insert verbs (non-`by` bound many arrows with an
            // insert hook) and set verbs (ctor-arg arrows) — hooks are
            // instance-qualified, verbs are basis-named (matching the
            // store's public op names), no trait impls (this step's
            // inherent-surface rule).
            for (i, infos) in inst_infos.iter().enumerate() {
                let inst_snake = snake(&members[i].0.to_string());
                for info in infos {
                    let Some(d) = info.backing else { continue };
                    let a = info.decl;
                    let sc = class_index_of(i, ent_index(i, &a.source)?);
                    let tc = class_index_of(i, ent_index(i, &a.target)?);
                    let root_self = !classes[sc].owns_records;
                    if card_of_decl(d) == Cardinality::Many
                        && a.by.is_none()
                        && !d.has(|c| matches!(c, Clause::MembershipExistence { .. }))
                        && d.has(|c| matches!(c, Clause::OnInsert { .. }))
                    {
                        tx_inserts.push(crate::tx::TxInsert {
                            arrow: basis_of(i, &a.name),
                            host: (!root_self).then(|| classes[sc].name.clone()),
                            member: classes[tc].name.clone(),
                            hook: format_ident!("__insert_{}_{}", inst_snake, a.name),
                            verb: None,
                        });
                    }
                    if d.has(|c| matches!(c, Clause::CtorArg { .. })) {
                        // A set verb's self is a record ref; an embedded
                        // record's ref is a path id, whose host-translation
                        // staging is the keyed-carry step's. The field
                        // itself still carries and dangler-checks (the
                        // entity's ctor args cover it) — only the verb is
                        // skipped.
                        if classes[sc].loc != ClassLoc::Root {
                            continue;
                        }
                        tx_sets.push(crate::tx::TxSet {
                            arrow: basis_of(i, &a.name),
                            self_entity: classes[sc].name.clone(),
                            target_entity: classes[tc].name.clone(),
                            hook: format_ident!("__set_{}_{}", inst_snake, a.name),
                            verb: None,
                        });
                    }
                }
            }
            let model = crate::tx::TxModel {
                store: store.clone(),
                rt: crate::runtime_path(),
                entities: tx_entities,
                store_fields: tx_store_fields,
                intrusive: tx_intrusive,
                inserts: tx_inserts,
                sets: tx_sets,
                keyed: tx_keyed,
            };
            out.extend(crate::tx::emit(&model));
        }
    }

    Ok(out)
}
