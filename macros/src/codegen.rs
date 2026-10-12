//! Flat (non-composed) store codegen — the codegen stage of
//! `__implement_gen!`, entered once every backing declaration and root
//! metadata tuple has been collected. All codegen lives here, in real
//! Rust, not tt-munching.
//!
//! M3: the four built-ins are no longer special. Every bound arrow's
//! storage is driven by its resolved `backing!` declaration — contributed
//! fields (renamed per the declaration's naming formulas), hook bodies,
//! the access protocols, and validate bodies are the declaration's
//! authored tokens, bound per placement by the role-substitution engine
//! ([`crate::roles`]). What stays uniform (the plan's (b) half): the
//! entity kit (`Id(pub u32)` newtype, record, `Copy` handle with
//! hand-written `Eq`/`Hash` by (store pointer, id)), placement of
//! contributions (self record, or the store when the self is the unfused
//! root), the doc-hidden hook-method naming, the `__validate_<arrow>`
//! convention with its uniform range pre-pass, and collision-free
//! generated names.

use std::collections::HashMap;

use proc_macro2::{Ident as Ident2, Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};

use crate::backing::{formula_name, BackingCard, BackingDecl, Clause, ContribField, NameFormula, OnTarget, PhaseLabel, ValidateVariant};
use crate::grammar::{
    arena_fld, data_ty, id_ty, iter_ty, link_stem, marker_ty, path_tokens, snake, Binding,
    Cardinality, RootMeta,
};
use crate::roles::{self, Ent, RoleBinding};
use crate::ChainConfig;

/// A resolved arrow: declaration plus its root's metadata and backing
/// declaration (`None` for derived `:= sum` arrows — never bound).
struct ArrowInfo<'a> {
    decl: &'a crate::grammar::ArrowDecl,
    meta: &'a RootMeta,
    backing: Option<&'a BackingDecl>,
}

pub(crate) fn card_of_decl(d: &BackingDecl) -> Cardinality {
    match d.card {
        BackingCard::One => Cardinality::One,
        BackingCard::Many => Cardinality::Many,
        BackingCard::Keyed => Cardinality::Keyed,
    }
}

/// Codegen entry: dispatch on the schema kind. Flat codegen is M1's;
/// composed codegen (fusion classes, placement, path ids) is M2's.
pub(crate) fn expand(config: &ChainConfig) -> syn::Result<TokenStream2> {
    match &config.schema {
        crate::grammar::GenSchema::Flat(_) => expand_flat(config),
        crate::grammar::GenSchema::Composed(_) => crate::codegen_composed::expand_composed(config),
    }
}

// ---------------------------------------------------------------------------
// The flat role binding
// ---------------------------------------------------------------------------

/// Flat placement of one binding's roles: store-level arenas, plain ids,
/// the self record (or the store itself at an unfused-root self).
struct FlatRoles {
    store: TokenStream2,
    a: Option<TokenStream2>,
    k: Option<TokenStream2>,
    w: Option<TokenStream2>,
    root_self: bool,
    id_ty: [TokenStream2; 3],
    arena: [Ident2; 3],
    map_field: Option<Ident2>,
    det_field: Option<Ident2>,
    tgt_handle: Ident2,
    walk_ty: Option<Ident2>,
    renames: Vec<(String, Ident2)>,
    walk_fields: Vec<(Ident2, TokenStream2)>,
}

impl FlatRoles {
    #[allow(clippy::too_many_arguments)]
    fn new(
        decl: &BackingDecl,
        arrow: &crate::grammar::ArrowDecl,
        root_self: bool,
        tgt_handle: Ident2,
        store_overrides: &HashMap<(String, String), Ident2>,
    ) -> Self {
        let id_a = id_ty(&arrow.source);
        let id_b = id_ty(&arrow.target);
        let id_k = arrow.key.as_ref().map(id_ty);
        let renames = contributed_fields(decl)
            .into_iter()
            .map(|f| {
                // Store-level contributions disambiguated per arrow when two
                // bindings emit the same store field (the pinned "bare when
                // unambiguous" rule); record fields keep the formula.
                let emitted = store_overrides
                    .get(&(arrow.name.to_string(), f.logical.to_string()))
                    .cloned()
                    .unwrap_or_else(|| formula_name(&f, f.formula, &arrow.name, &link_stem(&arrow.name)));
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
        FlatRoles {
            store: quote!(self),
            a: None,
            k: None,
            w: None,
            root_self,
            id_ty: [
                quote!(#id_a),
                quote!(#id_b),
                id_k.map(|i| quote!(#i)).unwrap_or(quote!(())),
            ],
            arena: [
                arena_fld(&arrow.source),
                arena_fld(&arrow.target),
                arrow.key.as_ref().map(arena_fld).unwrap_or_else(|| arena_fld(&arrow.target)),
            ],
            map_field: contributed_fields(decl)
                .find(|f| f.formula == NameFormula::Suffix)
                .map(|f| formula_name(&f, f.formula, &arrow.name, &link_stem(&arrow.name))),
            det_field: None,
            tgt_handle,
            walk_ty: None,
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

    fn with_det(mut self, det: Ident2) -> Self {
        self.det_field = Some(det);
        self
    }
}

impl RoleBinding for FlatRoles {
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
        self.id_ty[match e {
            Ent::A => 0,
            Ent::B => 1,
            Ent::K => 2,
        }]
        .clone()
    }
    fn rec(&self, arg: TokenStream2, e: Ent) -> TokenStream2 {
        if self.root_self && e == Ent::A {
            // The unfused root's record is the store itself.
            return self.store();
        }
        let arena = &self.arena[match e {
            Ent::A => 0,
            Ent::B => 1,
            Ent::K => 2,
        }];
        let s = self.store();
        quote!(#s.#arena[(#arg).0 as usize])
    }
    fn records(&self, e: Ent) -> TokenStream2 {
        if self.root_self && e == Ent::A {
            return quote!(::core::iter::once(()));
        }
        let arena = &self.arena[match e {
            Ent::A => 0,
            Ent::B => 1,
            Ent::K => 2,
        }];
        let idt = self.id_ty(e);
        let s = self.store();
        quote!((0..#s.#arena.len() as u32).map(#idt))
    }
    fn maps(&self) -> TokenStream2 {
        let f = self.map_field.as_ref().expect("hash_index contributes its map field");
        let s = self.store();
        if self.root_self {
            // One store-level pair; the owner is the vacuous host.
            quote!(::core::iter::once(((), &#s.#f)))
        } else {
            let aa = &self.arena[0];
            quote!(#s.#aa.iter().map(|d| ((), &d.#f)))
        }
    }
    fn host_of(&self, _arg: TokenStream2, _e: Ent) -> TokenStream2 {
        quote!(())
    }
    fn idx(&self, arg: TokenStream2, _e: Ent) -> TokenStream2 {
        quote!((#arg).0 as usize)
    }
    fn arena_of(&self, e: Ent, _arg: TokenStream2) -> TokenStream2 {
        let arena = &self.arena[match e {
            Ent::A => 0,
            Ent::B => 1,
            Ent::K => 2,
        }];
        let s = self.store();
        quote!(#s.#arena)
    }
    fn map_from(&self, arg: TokenStream2) -> TokenStream2 {
        let f = self.map_field.as_ref().expect("hash_index contributes its map field");
        if self.root_self {
            let s = self.store();
            quote!(#s.#f)
        } else {
            let r = self.rec(arg, Ent::A);
            quote!(#r.#f)
        }
    }
    fn determining(&self, arg: TokenStream2) -> TokenStream2 {
        let f = self.det_field.as_ref().expect("validate by binds det_label and the determinant field");
        let r = self.rec(arg, Ent::B);
        quote!(#r.#f)
    }
    fn handle(&self, arg: TokenStream2, raw: bool) -> TokenStream2 {
        let h = &self.tgt_handle;
        let s = self.store();
        if raw {
            let idt = self.id_ty(Ent::B);
            quote!(#h { store: #s, id: #idt(#arg) })
        } else {
            quote!(#h { store: #s, id: #arg })
        }
    }
    fn walk_ty(&self) -> Option<Ident2> {
        self.walk_ty.clone()
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

// ---------------------------------------------------------------------------
// Declaration helpers
// ---------------------------------------------------------------------------

/// All contributed fields of a declaration, in clause order.
pub(crate) fn contributed_fields(decl: &BackingDecl) -> impl Iterator<Item = &ContribField> {
    decl.clauses.iter().filter_map(|c| match c {
        Clause::On { fields, .. } => Some(fields.iter().map(|f| f)),
        _ => None,
    })
    .flatten()
}

/// Every `on <target>` clause of a declaration as (label, fields) pairs —
/// `on store` may repeat per label, so collection sees them all.
pub(crate) fn on_clauses<'a>(decl: &'a BackingDecl, target: OnTarget) -> Vec<(Option<PhaseLabel>, &'a [ContribField])> {
    decl.clauses
        .iter()
        .filter_map(|c| match c {
            Clause::On { target: t, label, fields, .. } if *t == target => {
                Some((*label, fields.as_slice()))
            }
            _ => None,
        })
        .collect()
}

pub(crate) fn on_clause<'a>(decl: &'a BackingDecl, target: OnTarget) -> Option<&'a Clause> {
    decl.clause(|c| matches!(c, Clause::On { target: t, .. } if *t == target))
}

pub(crate) fn body_of<'a>(decl: &'a BackingDecl, f: fn(&Clause) -> bool) -> Option<&'a TokenStream2> {
    decl.clause(f).map(|c| match c {
        Clause::Read { body, .. }
        | Clause::Lookup { body, .. }
        | Clause::Iterate { body, .. }
        | Clause::Advance { body, .. }
        | Clause::OnInsert { body, .. }
        | Clause::OnAdd { body, .. }
        | Clause::Compact { body, .. }
        | Clause::Validate { body, .. } => body,
        _ => unreachable!("selector matches body-carrying clauses only"),
    })
}

/// How one contributed field initializes in the record literal: the ctor
/// arg when its declaration carries `ctor_arg` (and the field rides the
/// self record), else the uniform init rule.
pub(crate) enum FieldInit {
    CtorArg(Ident2),
    None_,
    New,
}

pub(crate) fn init_of(decl: &BackingDecl, on_self_record: bool, field: &ContribField, arrow: &Ident2) -> syn::Result<FieldInit> {
    if decl.has(|c| matches!(c, Clause::CtorArg { .. })) && on_self_record {
        return Ok(FieldInit::CtorArg(arrow.clone()));
    }
    let t = field.ty.to_token_stream().to_string().replace(' ', "");
    if t.starts_with("Option<") {
        Ok(FieldInit::None_)
    } else if t.starts_with("Vec<") || t.starts_with("HashMap<") {
        Ok(FieldInit::New)
    } else {
        Err(syn::Error::new(
            Span::call_site(),
            format!(
                "field `{}` of backing `{}` is hook-maintained with type `{}`, but the uniform init rule covers only `Option<_>`, `Vec<_>`, and `HashMap<_, _>`",
                field.logical, decl.name, t
            ),
        ))
    }
}

pub(crate) fn scaffold_lets(
    body: &TokenStream2,
    label: &str,
    det_label: Option<&str>,
    errs_always: bool,
) -> TokenStream2 {
    let s = body.to_string();
    let mut out = TokenStream2::new();
    if errs_always || s.contains("errs") {
        let e = Ident2::new("errs", Span::mixed_site());
        out.extend(quote!(let mut #e: ::std::vec::Vec<::std::string::String> = ::std::vec::Vec::new();));
    }
    if s.contains("arrow_label") {
        let a = Ident2::new("arrow_label", Span::mixed_site());
        out.extend(quote!(let #a: &str = #label;));
    }
    if s.contains("det_label") {
        let d = Ident2::new("det_label", Span::mixed_site());
        let det = det_label.unwrap_or("");
        out.extend(quote!(let #d: &str = #det;));
    }
    out
}

/// One collected record/store field with everything emission needs.
struct CollectedField {
    emitted: Ident2,
    ty: TokenStream2,
    init: FieldInit,
    /// `Some(entity)` for a record field, `None` for a store field.
    owner: Option<Ident2>,
    arrow: Ident2,
    backing_name: Ident2,
    /// For the range pre-pass: the field's id type is `Option<id>`.
    option_id: bool,
    /// Phase attribution for store-level contributions: `on store build`
    /// fields are typed per phase (released at the type level on Frozen);
    /// `on store frozen` and unlabeled fields exist in all phases.
    phase: Option<PhaseLabel>,
}

// ---------------------------------------------------------------------------
// Flat codegen
// ---------------------------------------------------------------------------

pub(crate) fn expand_flat(config: &ChainConfig) -> syn::Result<TokenStream2> {
    let schema = match &config.schema {
        crate::grammar::GenSchema::Flat(p) => p,
        crate::grammar::GenSchema::Composed(_) => unreachable!("dispatched by expand"),
    };
    let body = &schema.body;
    let store = &config.store;

    // metas[i] belongs to the i-th unique root (first-occurrence order over
    // the schema's arrows) — the order the resolution chain queued them in,
    // keyed by *resolver-absolute* path spelling.
    let unique_roots = crate::resolved_roots(schema)?;
    if unique_roots.len() != config.metas.len() {
        return Err(syn::Error::new(
            Span::call_site(),
            format!(
                "internal: resolved {} root metadata tuples for {} roots",
                config.metas.len(),
                unique_roots.len()
            ),
        ));
    }
    let mut meta_of: HashMap<String, &RootMeta> = HashMap::new();
    for (p, m) in unique_roots.iter().zip(&config.metas) {
        meta_of.insert(path_tokens(p).to_string(), m);
    }

    let root_entity = body
        .entities
        .iter()
        .find(|e| e.root)
        .expect("pattern! validated exactly one root-marked entity");
    let elements: Vec<&crate::grammar::EntityDecl> =
        body.entities.iter().filter(|e| !e.root).collect();
    let is_root = |name: &proc_macro2::Ident| root_entity.name == *name;

    // ---- resolve arrows: backing declarations against root cardinalities ----
    let decl_of = |key: &str| -> Option<&BackingDecl> {
        config
            .backing_decls
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, p)| &p.decl)
    };
    let binding_of = |name: &str| -> Option<&Binding> {
        config.bindings.iter().find(|b| b.arrow == name)
    };
    let mut infos: Vec<ArrowInfo> = Vec::new();
    for decl in &body.arrows {
        let meta = meta_of
            .get(&crate::resolve_schema_path(decl.inst.as_ref().unwrap(), &schema.pkg)?.to_string())
            .expect("every arrow's root was queued");
        // The instantiation's key clause must agree with the root's keyed
        // cardinality: the root owns the cardinality, the instantiation only
        // binds the key sort.
        match (meta.card, &decl.key) {
            (Cardinality::Keyed, None) => {
                return Err(syn::Error::new(
                    Span::call_site(),
                    format!(
                        "arrow `{}` instantiates the keyed root `{}` but does not bind a key sort (`key(LocalEntity)`)",
                        decl.name,
                        path_tokens(decl.inst.as_ref().unwrap())
                    ),
                ));
            }
            (card, Some(k)) if card != Cardinality::Keyed => {
                return Err(syn::Error::new(
                    Span::call_site(),
                    format!(
                        "arrow `{}` declares `key({k})` but its root `{}` is not a keyed root",
                        decl.name,
                        path_tokens(decl.inst.as_ref().unwrap())
                    ),
                ));
            }
            _ => {}
        }
        let backing = if decl.sum.is_some() {
            None
        } else if let Some(key) = binding_of(&decl.name.to_string()).map(|_| decl.name.to_string()) {
            let d = decl_of(&key).expect("stage1 queued every explicit binding's courier");
            if card_of_decl(d) != meta.card {
                return Err(syn::Error::new(
                    Span::call_site(),
                    format!(
                        "backing `{}` has wrong cardinality for arrow `{}` (a `{}` arrow)",
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
                Span::call_site(),
                format!(
                    "missing backing binding for `many` arrow `{}` (`arena` vs `intrusive_list` is an explicit choice)",
                    decl.name
                ),
            ));
        } else if meta.card == Cardinality::Keyed {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "missing backing binding for keyed arrow `{}` (`hash_index` is the only keyed-legal backing)",
                    decl.name
                ),
            ));
        } else {
            // `one` arrows default to `field`; the default-decl hop has
            // collected its declaration under the arrow's key.
            Some(decl_of(&decl.name.to_string()).expect("defaulted one arrow's field declaration collected"))
        };
        infos.push(ArrowInfo { decl, meta, backing });
    }

    // ---- per-declaration applicability (flat placements) ----
    // `membership: existence` (declared predicate): B's entity arrow is
    // the unique arrow targeting B bound to an existence backing — a
    // duplicate binding is rejected naming both arrows (parity with the
    // composed applicability loop).
    let mut existence_by_target: HashMap<String, Vec<String>> = HashMap::new();
    for info in &infos {
        let Some(d) = info.backing else { continue };
        if !d.has(|c| matches!(c, Clause::MembershipExistence { .. })) {
            continue;
        }
        existence_by_target
            .entry(info.decl.target.to_string())
            .or_default()
            .push(info.decl.name.to_string());
    }
    for (target, arrows) in &existence_by_target {
        if arrows.len() > 1 {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "class `{target}` has two entity-arrow candidates — arrows `{}` and `{}` both bind a `membership: existence` backing; the entity arrow of a target is unique, so one of them must bind a relation backing (`intrusive_list`)",
                    arrows[0],
                    arrows[1],
                ),
            ));
        }
    }
    for info in &infos {
        let Some(d) = info.backing else { continue };
        let named = info.decl.name.to_string();
        // Phased backings (csr): the entity-arrow / unfused-root rejection. A
        // phased backing maintains membership by insert hooks, so it cannot
        // bind its target's entity arrow — mechanically, `hosts_arena(self,
        // B)` must not hold. Flat stores host every arena at the store, so
        // the predicate's only flat arm is the unfused-root self (the root
        // owns no `a` for `on_add`/`on_insert` either way).
        if d.is_phased() && is_root(&info.decl.source) {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "backing `{d_name}` (arrow `{named}`) is phased, but `{named}` is sourced at the pattern's root-marked entity `{src}` — the entity-arrow/unfused-root case: the root owns no record (it *is* the store), so there is no self id for `on_add`/`on_insert`, and an arrow whose B arena is root-level under an unfused root is its target's entity arrow, which a hook-maintained backing cannot declare (bind `arena` — existence is `add_<B>`'s own write path — or source the arrow at an element)",
                    d_name = d.name,
                    src = info.decl.source,
                ),
            ));
        }
        // Constructor-orchestrated clauses need an `add_<A>`-orchestrated
        // self: an unfused root owns no constructor (Store::new() takes no
        // arguments) and no self id.
        let orchestrated = d.has(|c| matches!(c, Clause::CtorArg { .. }))
            || d.has(|c| matches!(c, Clause::OnAdd { .. }));
        if orchestrated && is_root(&info.decl.source) {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "arrow `{named}` is sourced at the root-marked entity `{}`; the `{}` backing needs a field on the self record, and the root owns no record (it *is* the store)",
                    info.decl.source, d.name
                ),
            ));
        }
        // `membership: existence` (declared predicate): the entity arrow's
        // membership kind is primitive — a `by`-derived arrow contradicts
        // existence.
        if d.has(|c| matches!(c, Clause::MembershipExistence { .. })) {
            if let Some(det) = &info.decl.by {
                return Err(syn::Error::new(
                    Span::call_site(),
                    format!(
                        "backing `{d_name}` on arrow `{named}` declares `membership: existence`, but `{named}` derives membership `by {det}` — existence is primitive; bind a relation backing (`intrusive_list`) or drop the `by`",
                        d_name = d.name,
                    ),
                ));
            }
        }
        // `hosts_arena` (declared predicate): B's arena is hosted by A's
        // record. A flat store hosts every arena at the store, so the
        // predicate holds only at the pattern's root (the unfused-root
        // self case); at an element self it is rejected with the placement
        // facts. (`direct_records` holds everywhere flat — no embedded
        // placements exist — so it evaluates to true and needs no check.)
        if d.has(|c| matches!(c, Clause::Requires { items, .. } if items.iter().any(|x| *x == "hosts_arena")))
            && !is_root(&info.decl.source)
        {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "arrow `{named}` declares `requires: hosts_arena` (backing `{d_name}`), but B's arena is a root-level arena of the store, not hosted by the self record (self case: plain self at the element `{src}`; target `{tgt}` at root level; flat stores host every arena at the store, so `arena` binds only root-sourced arrows)",
                    d_name = d.name,
                    src = info.decl.source,
                    tgt = info.decl.target,
                ),
            ));
        }
        // Validate-variant selection: the arrow's membership facts pick the
        // body; a declaration that does not author it is rejected here. A
        // `Plain` body (membership-agnostic — csr's offsets check,
        // hash_index's map walk) satisfies any membership kind.
        if card_of_decl(d) == Cardinality::Many {
            let wanted = if info.decl.by.is_some() {
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
                    Span::call_site(),
                    format!(
                        "backing `{}` authors no {missing} body, but arrow `{named}` needs it (the declaration binds only the other membership kind)",
                        d.name
                    ),
                ));
            }
        }
    }

    // `by <det>`: the determining arrow must be total (its root says `one`).
    for info in &infos {
        let Some(det_name) = &info.decl.by else { continue };
        let det = infos.iter().find(|i| &i.decl.name == det_name).ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                format!("`by {}` does not name a resolvable arrow", det_name),
            )
        })?;
        if det.meta.card != Cardinality::One {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "`by {}` names arrow `{}`, which is not total (`one`) — membership would be undefined",
                    det_name, det.decl.name
                ),
            ));
        }
    }

    // ---- collect contributed fields (records + store) ----
    // Pre-scan: store-level contribution name usage. The pinned naming rule
    // ("the declared stem, bare when unambiguous"): when two bindings emit
    // the same store field, all colliding fields rename per arrow (the
    // suffix formula, `<arrow>_<name>`). An `on A` contribution at an
    // unfused-root self folds to the store, so it joins the scan.
    let mut store_name_users: HashMap<String, Vec<(Ident2, Ident2)>> = HashMap::new();
    for info in &infos {
        let Some(d) = info.backing else { continue };
        let a = info.decl;
        for target in [OnTarget::A, OnTarget::Store] {
            if target == OnTarget::A && !is_root(&a.source) {
                continue;
            }
            for (_label, fields) in on_clauses(d, target) {
                for f in fields {
                    let emitted = formula_name(f, f.formula, &a.name, &link_stem(&a.name));
                    store_name_users
                        .entry(emitted.to_string())
                        .or_default()
                        .push((a.name.clone(), f.logical.clone()));
                }
            }
        }
    }
    let mut store_overrides: HashMap<(String, String), Ident2> = HashMap::new();
    for users in store_name_users.values() {
        if users.len() > 1 {
            for (arrow, logical) in users {
                let disambiguated = Ident2::new(
                    &format!("{}_{}", arrow, logical),
                    logical.span(),
                );
                store_overrides.insert((arrow.to_string(), logical.to_string()), disambiguated);
            }
        }
    }
    let mut rec_fields: HashMap<String, Vec<CollectedField>> =
        elements.iter().map(|e| (e.name.to_string(), Vec::new())).collect();
    let mut store_fields: Vec<CollectedField> = Vec::new();
    for info in &infos {
        let Some(d) = info.backing else { continue };
        let a = info.decl;
        for (target, owner_name, on_self_record) in [
            (OnTarget::A, Some(&a.source), true),
            (OnTarget::B, Some(&a.target), false),
            (OnTarget::Store, None, false),
        ] {
            for (label, fields) in on_clauses(d, target) {
                for f in fields {
                    let mut emitted = formula_name(f, f.formula, &a.name, &link_stem(&a.name));
                    if let Some(r) = store_overrides.get(&(a.name.to_string(), f.logical.to_string())) {
                        emitted = r.clone();
                    }
                    let ty = roles::rewrite_ty(&f.ty.to_token_stream(), &FlatRoles::new(d, a, false, a.target.clone(), &store_overrides));
                    let init = init_of(d, on_self_record && d.has(|c| matches!(c, Clause::CtorArg { .. })), f, &a.name)?;
                    // A field contributed at the root-marked self folds to the
                    // store (the unfused root's record is the store).
                    let owner = match owner_name {
                        Some(name) if !is_root(name) => Some(name.clone()),
                        _ => None,
                    };
                    // Phase attribution rides only on explicit `on store`
                    // contributions; record fields and unlabeled store fields
                    // exist in all phases.
                    let phase = if target == OnTarget::Store { label } else { None };
                    let cf = CollectedField {
                        option_id: f.ty.to_token_stream().to_string().replace(' ', "").starts_with("Option<"),
                        emitted,
                        ty,
                        init,
                        owner,
                        arrow: a.name.clone(),
                        backing_name: d.name.clone(),
                        phase,
                    };
                    match &cf.owner {
                        Some(o) => rec_fields.get_mut(&o.to_string()).unwrap().push(cf),
                        None => store_fields.push(cf),
                    }
                }
            }
        }
    }

    // The store binds a phased backing (csr) when any bound declaration
    // carries `phase:` — the type-state emission below keys on this.
    let phased = infos
        .iter()
        .any(|i| i.backing.map(|d| d.is_phased()).unwrap_or(false));

    let mut out = TokenStream2::new();

    // ---- entity kit: id newtype + record, non-root entities only ----
    for e in &elements {
        let ent_doc_name = &e.name;
        let idt = id_ty(&e.name);
        let dat = data_ty(&e.name);
        let fields = rec_fields[&e.name.to_string()]
            .iter()
            .map(|f| {
                let name = &f.emitted;
                let doc = format!(
                    "field backing of arrow `{}` (backing `{}`)",
                    f.arrow, f.backing_name
                );
                let ty = &f.ty;
                quote!(
                    #[doc = #doc]
                    #name: #ty,
                )
            })
            .collect::<Vec<_>>();
        out.extend(quote! {
            #[doc = concat!("Index of a `", stringify!(#ent_doc_name), "` in its arena.")]
            #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
            pub struct #idt(pub u32);
            #[doc = concat!("Record of a `", stringify!(#ent_doc_name), "`.")]
            #[derive(Debug)]
            struct #dat { #(#fields)* }
        });
    }

    // ---- constructors + hooks + write ops ----
    let mut ctor_impls = TokenStream2::new();
    let mut hook_impls = TokenStream2::new();
    for e in &elements {
        let ent_doc_name = &e.name;
        // One argument per total `one` arrow originating at this entity
        // whose backing contributes `ctor_arg`.
        let args: Vec<&ArrowInfo> = infos
            .iter()
            .filter(|i| {
                i.decl.source == e.name
                    && i.backing
                        .map(|d| d.has(|c| matches!(c, Clause::CtorArg { .. })))
                        .unwrap_or(false)
            })
            .collect();
        let arg_defs = args.iter().map(|i| {
            let n = &i.decl.name;
            let tid = id_ty(&i.decl.target);
            quote!(#n: #tid,)
        });
        // Record literal: every contributed field of this record inits per
        // the uniform rule (ctor args move in; links start empty — the
        // hooks relink on insert).
        let mut record_fields = TokenStream2::new();
        for f in &rec_fields[&e.name.to_string()] {
            let n = &f.emitted;
            match &f.init {
                FieldInit::CtorArg(a) => record_fields.extend(quote!(#n: #a,)),
                FieldInit::None_ => {
                    record_fields.extend(quote!(#n: ::core::option::Option::None,));
                }
                FieldInit::New => record_fields.extend(quote!(#n: ::std::default::Default::default(),)),
            }
        }
        // `on_add` hooks fired by this constructor: a backing's per-add
        // growth of store-level per-self state (csr's bucket push) runs
        // before the insert hooks below, so an insert hook that reads the
        // grown state for the new record sees it.
        let mut onadd_calls = TokenStream2::new();
        for info in &infos {
            let Some(d) = info.backing else { continue };
            if info.decl.source == e.name && d.has(|c| matches!(c, Clause::OnAdd { .. })) {
                let hook = format_ident!("__onadd_{}", info.decl.name);
                onadd_calls.extend(quote!(self.#hook(id);));
            }
        }
        // `by`-derived arrows whose determining `one` arrow originates here:
        // the new record becomes that arrow's first member, inserted by the
        // arrow's named hook reading the determining argument.
        let mut hook_calls = TokenStream2::new();
        for info in &infos {
            let Some(det_name) = &info.decl.by else { continue };
            let Some(det) = args.iter().find(|i| &i.decl.name == det_name) else {
                continue;
            };
            if info
                .backing
                .map(|d| d.has(|c| matches!(c, Clause::OnInsert { .. })))
                .unwrap_or(false)
            {
                let hook = format_ident!("__insert_{}", info.decl.name);
                let det_arg = &det.decl.name;
                hook_calls.extend(quote!(self.#hook(#det_arg, id);));
            }
        }
        let add_name = format_ident!("add_{}", snake(&e.name.to_string()));
        let idt = id_ty(&e.name);
        let dat = data_ty(&e.name);
        let arena = arena_fld(&e.name);
        ctor_impls.extend(quote! {
            #[doc = concat!("Constructor for `", stringify!(#ent_doc_name), "`: one argument per total `one` arrow originating here, then each bound backing's insert hook.")]
            pub fn #add_name(&mut self, #(#arg_defs)*) -> #idt {
                let id = #idt(self.#arena.len() as u32);
                self.#arena.push(#dat { #record_fields });
                #onadd_calls
                #hook_calls
                id
            }
        });
    }

    // Hook methods + public write ops, per binding. Emitted hook/param
    // names keep the authored spelling (`a`, `b`, `k`) with mixed-site
    // spans; `w` rewrites to `self` inside `next()`.
    for info in &infos {
        let Some(d) = info.backing else { continue };
        let a = info.decl;
        let arrow_doc_name = &a.name;
        let label = format!("arrow `{arrow_doc_name}`");
        let root_self = is_root(&a.source);

        // on_insert hook (+ primitive insert op).
        if let Some(body) = body_of(d, |c| matches!(c, Clause::OnInsert { .. })) {
            let d_name = &d.name;
            let hook = format_ident!("__insert_{}", a.name);
            let ida = id_ty(&a.source);
            let idb = id_ty(&a.target);
            let idk = a.key.as_ref().map(id_ty);
            let mut roles = FlatRoles::new(d, a, root_self, a.target.clone(), &store_overrides);
            if root_self {
                roles = roles.with_a(quote!(()));
            }
            let body_rw = roles::rewrite(body, &roles);
            let scaffolds = scaffold_lets(body, &label, None, false);
            let (sig, op_sig, op_args) = match (root_self, idk.as_ref()) {
                (true, Some(kid)) => (
                    {
                        let k = Ident2::new("k", Span::mixed_site());
                        let b = Ident2::new("b", Span::mixed_site());
                        quote!(#k: #kid, #b: #idb)
                    },
                    quote!(key: #kid, target: #idb),
                    quote!(key, target),
                ),
                (true, None) => (
                    {
                        let b = Ident2::new("b", Span::mixed_site());
                        quote!(#b: #idb)
                    },
                    quote!(member: #idb),
                    quote!(member),
                ),
                (false, Some(kid)) => (
                    {
                        let a = Ident2::new("a", Span::mixed_site());
                        let k = Ident2::new("k", Span::mixed_site());
                        let b = Ident2::new("b", Span::mixed_site());
                        quote!(#a: #ida, #k: #kid, #b: #idb)
                    },
                    quote!(self_id: #ida, key: #kid, target: #idb),
                    quote!(self_id, key, target),
                ),
                (false, None) => (
                    {
                        let a = Ident2::new("a", Span::mixed_site());
                        let b = Ident2::new("b", Span::mixed_site());
                        quote!(#a: #ida, #b: #idb)
                    },
                    quote!(self_id: #ida, member: #idb),
                    quote!(self_id, member),
                ),
            };
            hook_impls.extend(quote! {
                #[doc(hidden)]
                #[doc = concat!("insert hook of arrow `", stringify!(#arrow_doc_name), "` (backing `", stringify!(#d_name), "`)")]
                fn #hook(&mut self, #sig) {
                    #scaffolds
                    #body_rw
                }
            });
            // Primitive membership (no `by`) and keyed arrows expose the
            // insert op; `by`-derived membership can never contradict the
            // determining field, so those arrows generate no insert op.
            if a.by.is_none() {
                let insert_name = format_ident!("insert_{}", a.name);
                ctor_impls.extend(quote! {
                    #[doc = concat!("Insert through the `", stringify!(#arrow_doc_name), "` insert hook (backing `", stringify!(#d_name), "`).")]
                    pub fn #insert_name(&mut self, #op_sig) {
                        self.#hook(#op_args);
                    }
                });
            }
        }

        // Field-write hook + `set_` op: the single write path after
        // construction for a `ctor_arg`-fed field (M2's named hook; M4's
        // flush replays through it).
        if d.has(|c| matches!(c, Clause::CtorArg { .. })) {
            let field = contributed_fields(d)
                .find(|f| f.formula == NameFormula::Arrow)
                .map(|f| formula_name(f, f.formula, &a.name, &link_stem(&a.name)))
                .unwrap_or_else(|| a.name.clone());
            let hook = format_ident!("__set_{}", a.name);
            let set_name = format_ident!("set_{}", a.name);
            let sid = id_ty(&a.source);
            let tid = id_ty(&a.target);
            let s_arena = arena_fld(&a.source);
            let f = &field;
            hook_impls.extend(quote! {
                #[doc(hidden)]
                #[doc = concat!("field-write hook of arrow `", stringify!(#arrow_doc_name), "`: set `self_id`'s field to `value`")]
                fn #hook(&mut self, self_id: #sid, value: #tid) {
                    self.#s_arena[self_id.0 as usize].#f = value;
                }
            });
            ctor_impls.extend(quote! {
                #[doc = concat!("Set `self_id`'s `", stringify!(#arrow_doc_name), "` to `value` (field-backed `one` arrow).")]
                pub fn #set_name(&mut self, self_id: #sid, value: #tid) {
                    self.#hook(self_id, value);
                }
            });
        }

        // `on_add` hook: per-add growth of store-level per-self state
        // (csr's bucket push). Fired from `add_<A>` orchestration (the ctor
        // loop above); applicability guaranteed an `add_<A>`-orchestrated
        // self (an unfused root was rejected).
        if let Some(body) = body_of(d, |c| matches!(c, Clause::OnAdd { .. })) {
            let d_name = &d.name;
            let hook = format_ident!("__onadd_{}", a.name);
            let ida = id_ty(&a.source);
            let a_mixed = Ident2::new("a", Span::mixed_site());
            // The self is never an unfused root here (applicability rejected
            // it — on_add is constructor-orchestrated), so `a` binds the
            // real id; the body's mentions re-emit mixed-site and pair.
            let roles = FlatRoles::new(d, a, false, a.target.clone(), &store_overrides);
            let body_rw = roles::rewrite(body, &roles);
            let scaffolds = scaffold_lets(body, &label, None, false);
            hook_impls.extend(quote! {
                #[doc(hidden)]
                #[doc = concat!("on-add hook of arrow `", stringify!(#arrow_doc_name), "` (backing `", stringify!(#d_name), "`): per-add growth of store-level per-self state.")]
                fn #hook(&mut self, #a_mixed: #ida) {
                    #scaffolds
                    #body_rw
                }
            });
        }

        // `compact` hook (phased backings only): the compaction body runs on
        // the build-phase store at the single `freeze()`, in binding order.
        if let Some(body) = body_of(d, |c| matches!(c, Clause::Compact { .. })) {
            let d_name = &d.name;
            let hook = format_ident!("__compact_{}", a.name);
            let roles = FlatRoles::new(d, a, root_self, a.target.clone(), &store_overrides);
            let body_rw = roles::rewrite(body, &roles);
            let scaffolds = scaffold_lets(body, &label, None, false);
            hook_impls.extend(quote! {
                #[doc(hidden)]
                #[doc = concat!("compact contribution of arrow `", stringify!(#arrow_doc_name), "` (backing `", stringify!(#d_name), "`): runs at the single `freeze()`.")]
                fn #hook(&mut self) {
                    #scaffolds
                    #body_rw
                }
            });
        }
    }

    // ---- validate methods (the round-4 convention) ----
    // One doc-hidden `__validate_<arrow>` per bound backing, summoned from
    // `validate()` in binding order (then defaulted arrows in schema
    // order). A uniform range pre-pass over contributed `Option<id>` fields
    // runs before the authored body and returns early on damage, so
    // authored walks may assume links in range and use `rec(x)` freely.
    let mut validate_order: Vec<String> = config.bindings.iter().map(|b| b.key_str()).collect();
    for info in &infos {
        if info.backing.is_some() && info.decl.sum.is_none() {
            let key = info.decl.name.to_string();
            if !validate_order.contains(&key) {
                validate_order.push(key);
            }
        }
    }
    let mut validate_methods = TokenStream2::new();
    let mut validate_calls = TokenStream2::new();
    for key in &validate_order {
        let Some(info) = infos.iter().find(|i| i.decl.name.to_string() == *key) else {
            continue;
        };
        let Some(d) = info.backing else { continue };
        let a = info.decl;
        let label = format!("arrow `{}`", a.name);
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
        let det_field = a
            .by
            .as_ref()
            .map(|det_name| {
                let det = body.arrows.iter().find(|x| &x.name == det_name).expect("pattern! validated the determinant");
                formula_name(
                    contributed_fields(
                        infos
                            .iter()
                            .find(|i| &i.decl.name == det_name)
                            .and_then(|i| i.backing)
                            .expect("the determinant is a bound `one` arrow"),
                    )
                    .find(|f| f.formula == NameFormula::Arrow)
                    .expect("the determinant is field-backed"),
                    NameFormula::Arrow,
                    &det.name,
                    &link_stem(&det.name),
                )
            });
        let mut roles = FlatRoles::new(d, a, is_root(&a.source), a.target.clone(), &store_overrides).with_det(
            det_field.unwrap_or_else(|| a.name.clone()),
        );
        if is_root(&a.source) {
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
        for cf in rec_fields.values().flatten().chain(store_fields.iter()) {
            if cf.arrow != a.name || !cf.option_id {
                continue;
            }
            let f = &cf.emitted;
            let b_arena = arena_fld(&a.target);
            let head = match &cf.owner {
                Some(o) => *o == a.source,
                None => is_root(&a.source),
            };
            let what = if head { "head" } else { "link" };
            match &cf.owner {
                Some(owner) => {
                    let o_arena = arena_fld(owner);
                    let oid = id_ty(owner);
                    let fmt = format!(
                        "{{}}: `{{}}` {what} of {{:?}} points at {{:?}}, which is not a live record"
                    );
                    pre_pass.extend(quote! {
                        for (ri, r) in self.#o_arena.iter().enumerate() {
                            if let ::core::option::Option::Some(n) = r.#f {
                                if n.0 as usize >= self.#b_arena.len() {
                                    #errs.push(::std::format!(
                                        #fmt,
                                        #arrow_label_id, stringify!(#f), #oid(ri as u32), n,
                                    ));
                                }
                            }
                        }
                    });
                }
                None => {
                    let fmt = format!(
                        "{{}}: `{{}}` {what} of the store points at {{:?}}, which is not a live record"
                    );
                    pre_pass.extend(quote! {
                        if let ::core::option::Option::Some(n) = self.#f {
                            if n.0 as usize >= self.#b_arena.len() {
                                #errs.push(::std::format!(
                                    #fmt,
                                    #arrow_label_id, stringify!(#f), n,
                                ));
                            }
                        }
                    });
                }
            }
        }
        let method = format_ident!("__validate_{}", a.name);
        let a_name = &a.name;
        validate_methods.extend(quote! {
            #[doc(hidden)]
            #[doc = concat!("validate contribution of arrow `", stringify!(#a_name), "`: the authored body with the uniform range pre-pass.")]
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

    // ---- store struct ----
    let arena_fields = elements.iter().map(|e| {
        let f = arena_fld(&e.name);
        let dat = data_ty(&e.name);
        quote!(#f: ::std::vec::Vec<#dat>,)
    });
    let phase_trait = format_ident!("__{}Phases", store);
    let store_contrib = store_fields.iter().map(|f| {
        let n = &f.emitted;
        let ty = &f.ty;
        let doc = format!(
            "store-level contribution of arrow `{}` (backing `{}`)",
            f.arrow, f.backing_name
        );
        // Phase attribution: `on store build` fields are typed through the
        // phase trait — `()` on Frozen — so a forgetful freeze has nothing
        // stale to carry and the accumulators are unwritable on the frozen
        // type; `on store frozen` and unlabeled fields exist in all phases
        // with their natural type.
        match f.phase {
            Some(PhaseLabel::Build) => {
                let assoc = format_ident!("__build_{}", f.emitted);
                quote!(#[doc = #doc] #n: <P as #phase_trait>::#assoc,)
            }
            _ => quote!(#[doc = #doc] #n: #ty,),
        }
    });
    let count_fns = elements.iter().map(|e| {
        let ent_doc_name = &e.name;
        let f = format_ident!("{}_count", snake(&e.name.to_string()));
        let arena = arena_fld(&e.name);
        quote! {
            #[doc = concat!("Number of `", stringify!(#ent_doc_name), "` records.")]
            pub fn #f(&self) -> usize {
                self.#arena.len()
            }
        }
    });
    // The phase trait: one associated type per `on store build` field. The
    // Build impl types them naturally; Frozen types them `()` — the
    // type-level release of the accumulators. Private to the module (the
    // struct's fields are private, so no private-in-public exposure).
    let phase_assocs: Vec<(Ident2, TokenStream2)> = store_fields
        .iter()
        .filter(|f| f.phase == Some(PhaseLabel::Build))
        .map(|f| (format_ident!("__build_{}", f.emitted), f.ty.clone()))
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
    // freeze()/finish() (phased): compact every phased backing in binding
    // order at the single boundary, then rebuild the store with the phase
    // marker swapped — build-attributed fields reset (their Frozen type is
    // `()`, and the compact body already drained them), everything else
    // moved over.
    let compact_hooks: Vec<Ident2> = validate_order
        .iter()
        .filter_map(|key| {
            let info = infos.iter().find(|i| i.decl.name.to_string() == *key)?;
            let d = info.backing?;
            d.is_phased().then(|| format_ident!("__compact_{}", info.decl.name))
        })
        .collect();
    let freeze_fields = elements.iter().map(|e| {
        let n = arena_fld(&e.name);
        quote!(#n: self.#n,)
    }).chain(store_fields.iter().map(|f| {
        let n = &f.emitted;
        match f.phase {
            Some(PhaseLabel::Build) => quote!(#n: ::std::default::Default::default(),),
            _ => quote!(#n: self.#n,),
        }
    }));
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
    // Debug field lists (phased manual impls; `_phase` is skipped, as derive
    // would print it but nothing pins it).
    let debug_names: Vec<Ident2> = elements
        .iter()
        .map(|e| arena_fld(&e.name))
        .chain(store_fields.iter().map(|f| f.emitted.clone()))
        .collect();
    let debug_fields = &debug_names;
    // The builder's read view: id-based getters derived from the authored
    // access protocols — one field-read getter per bound `one` arrow (named
    // after the arrow), one keyed lookup per keyed arrow. Pattern-level
    // constructor logic (induction, dedup) is user code over this view; the
    // getter names are contract. On a phased store the full view is Frozen;
    // the Build phase carries the non-phased bindings' getters only (a
    // phased backing contributes no build-phase reads — its store-level
    // state is pre-compaction by construction).
    let mut read_view = TokenStream2::new();
    let mut read_view_build = TokenStream2::new();
    for info in &infos {
        let Some(d) = info.backing else { continue };
        let a = info.decl;
        let arrow_doc = &a.name;
        let root_self = is_root(&a.source);
        let is_phased_binding = d.is_phased();
        let mut view = TokenStream2::new();
        match card_of_decl(d) {
            Cardinality::One => {
                let Some(body) = body_of(d, |c| matches!(c, Clause::Read { .. })) else { continue };
                let getter = &a.name;
                let sid = id_ty(&a.source);
                let tid = id_ty(&a.target);
                let roles = FlatRoles::new(d, a, root_self, a.target.clone(), &store_overrides)
                    .at(quote!(self))
                    .with_a(quote!(self_id));
                let body_rw = roles::rewrite(body, &roles);
                view.extend(quote! {
                    #[doc = concat!("Builder read view: the `", stringify!(#arrow_doc), "` field of `self_id`.")]
                    pub fn #getter(&self, self_id: #sid) -> #tid {
                        #body_rw
                    }
                });
            }
            Cardinality::Keyed => {
                let Some(body) = body_of(d, |c| matches!(c, Clause::Lookup { .. })) else { continue };
                let getter = format_ident!("{}_by_key", a.name);
                let kid = id_ty(a.key.as_ref().expect("keyed arrows bind a key sort"));
                let tid = id_ty(&a.target);
                let roles = FlatRoles::new(d, a, root_self, a.target.clone(), &store_overrides)
                    .at(quote!(self))
                    .with_a(quote!(self_id))
                    .with_k(quote!(key));
                let body_rw = roles::rewrite(body, &roles);
                if root_self {
                    view.extend(quote! {
                        #[doc = concat!("Builder read view: `", stringify!(#arrow_doc), "` keyed lookup (root-sourced; no self id).")]
                        pub fn #getter(&self, key: #kid) -> ::core::option::Option<#tid> {
                            #body_rw
                        }
                    });
                } else {
                    let sid = id_ty(&a.source);
                    view.extend(quote! {
                        #[doc = concat!("Builder read view: `", stringify!(#arrow_doc), "` keyed lookup on one self object.")]
                        pub fn #getter(&self, self_id: #sid, key: #kid) -> ::core::option::Option<#tid> {
                            #body_rw
                        }
                    });
                }
            }
            _ => {}
        }
        read_view.extend(view.clone());
        if !is_phased_binding {
            read_view_build.extend(view);
        }
    }
    let root_snake = Ident2::new(&snake(&root_entity.name.to_string()), root_entity.name.span());
    let root_handle = &root_entity.name;
    let store_pattern_name = config
        .pattern_path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default();
    // The store type as handles and iterators hold it: bare when unphased
    // (M1 shape); `#store<Frozen>` when phased — handles exist only for the
    // frozen phase.
    let store_ref: TokenStream2 = if phased { quote!(#store<Frozen>) } else { quote!(#store) };
    let root_accessor = quote! {
        #[doc = concat!("The root handle: the \"object\" view of this store (root entity `", stringify!(#root_handle), "`).")]
        pub fn #root_snake(&self) -> #root_handle<'_> {
            #root_handle { store: self }
        }
    };
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
    if phased {
        // Type-state store (spike 6 / the mixed-phase probe): ONE store type
        // parameterized by a phase marker, emitted only when a phased
        // backing is bound. `on store build` fields are typed through the
        // phase trait — `()` on Frozen — so a forgetful freeze has nothing
        // stale to carry and the accumulators are unwritable on the frozen
        // type. Non-phased stores keep the M1 shape (their own builder), so
        // the M1/M2 surfaces are unchanged.
        let arena_field_inits = elements.iter().map(|e| {
            let f = arena_fld(&e.name);
            quote!(#f: ::std::default::Default::default(),)
        });
        let store_field_inits = store_fields.iter().map(|f| {
            let n = &f.emitted;
            quote!(#n: ::std::default::Default::default(),)
        });
        let default_init = quote! {
            Self {
                #(#arena_field_inits)*
                #(#store_field_inits)*
                _phase: ::core::marker::PhantomData,
            }
        };
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

            #[doc = concat!("Store generated by `implement!` for pattern `", #store_pattern_name, "`.")]
            pub struct #store<P: #phase_trait = Frozen> {
                #(#arena_fields)*
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

                #ctor_impls

                #hook_impls

                #freeze_finish

                #read_view_build
            }

            impl #store<Frozen> {
                #(#count_fns)*

                #read_view

                #validate_methods

                #root_accessor

                #validate_method
            }
        });
    } else {
        out.extend(quote! {
            #[doc = concat!("Store generated by `implement!` for pattern `", #store_pattern_name, "`.")]
            #[derive(Default, Debug)]
            pub struct #store {
                #(#arena_fields)*
                #(#store_contrib)*
            }

            impl #store {
                #[doc = "Empty store."]
                pub fn new() -> Self {
                    Self::default()
                }

                #ctor_impls

                #hook_impls

                #validate_methods

                #(#count_fns)*

                #read_view

                #root_accessor

                #validate_method
            }
        });
    }

    // ---- handles ----
    for e in &body.entities {
        let h = &e.name;
        let (body, id_fn, eq_body, hash_body, dbg_body) = if e.root {
            (
                quote!(store: &'g #store_ref),
                quote!(),
                quote!(::std::ptr::eq(self.store, other.store)),
                quote!((self.store as *const #store_ref).hash(state);),
                {
                    // Mirror the reference `arena.rs`: the root handle's
                    // Debug lists the per-entity counts.
                    let count_fields = elements.iter().map(|el| {
                        let f = format_ident!("{}_count", snake(&el.name.to_string()));
                        let s = snake(&el.name.to_string());
                        quote!(.field(#s, &self.store.#f()))
                    });
                    quote! {
                        f.debug_struct(stringify!(#h))#(#count_fields)*.finish()
                    }
                },
            )
        } else {
            let idt = id_ty(&e.name);
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

            // Handles are equal only if they point into the same store at
            // the same index. Deriving would bound `Store: Eq`/`Hash`; the
            // id alone would equate handles across stores.
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
    // Markers live at <prefix-of-pattern-path>::<snake(pattern name)>::markers
    // (the module `pattern!` emitted at its invocation site).
    let mut marker_base = config.pattern_path.clone();
    marker_base.segments.pop(); // drop the pattern name; keep the prefix
    let marker_base_ts = path_tokens(&marker_base);
    let marker_mod = Ident2::new(
        &snake(&config.pattern_path.segments.last().unwrap().ident.to_string()),
        Span::call_site(),
    );

    // The iterate-body emission for one bound many arrow, read off a handle
    // binding `self` — the shared shape of the trait method and a sum's
    // sibling expressions.
    let iterate_expr = |info: &ArrowInfo, d: &BackingDecl| -> syn::Result<TokenStream2> {
        let a = info.decl;
        let body = body_of(d, |c| matches!(c, Clause::Iterate { .. }))
            .ok_or_else(|| syn::Error::new(Span::call_site(), format!("backing `{}` authors no iterate(a) body", d.name)))?;
        let it = iter_ty(&a.name);
        let roles = FlatRoles::new(d, a, is_root(&a.source), a.target.clone(), &store_overrides)
            .at(quote!(self.store))
            .with_a(quote!(self.id))
            .with_walk_ty(it);
        Ok(roles::rewrite(body, &roles))
    };

    let mut iter_defs = TokenStream2::new();
    let mut trait_impls = TokenStream2::new();

    for info in &infos {
        let d_expr = info.decl;
        let meta = info.meta;
        let arrow_doc_name = &d_expr.name;
        let marker = marker_ty(&d_expr.name);
        let marker_path = quote!(#marker_base_ts::#marker_mod::markers::#marker);
        let trait_path = crate::resolve_schema_path(d_expr.inst.as_ref().unwrap(), &schema.pkg)?;
        let self_h = &d_expr.source;
        let tgt_h = &d_expr.target;
        let m = &meta.method;
        let assoc = &meta.assoc;
        let it = iter_ty(&d_expr.name);

        if let Some((ua, ub)) = &d_expr.sum {
            let da = body.arrows.iter().find(|x| x.name == *ua).ok_or_else(|| {
                syn::Error::new(Span::call_site(), format!("sum names unknown sibling `{ua}`"))
            })?;
            let db = body.arrows.iter().find(|x| x.name == *ub).ok_or_else(|| {
                syn::Error::new(Span::call_site(), format!("sum names unknown sibling `{ub}`"))
            })?;
            let info_of = |n: &Ident2| -> syn::Result<ArrowInfo> {
                let i = infos.iter().find(|i| i.decl.name == *n).ok_or_else(|| {
                    syn::Error::new(Span::call_site(), format!("sum names unknown sibling `{n}`"))
                })?;
                Ok(ArrowInfo { decl: i.decl, meta: i.meta, backing: i.backing })
            };
            let ia = info_of(ua)?;
            let ib = info_of(ub)?;
            let da_decl = ia.backing.expect("sum siblings are bound many arrows");
            let db_decl = ib.backing.expect("sum siblings are bound many arrows");
            let ita = iter_ty(&da.name);
            let itb = iter_ty(&db.name);
            let ea = iterate_expr(&ia, da_decl)?;
            let eb = iterate_expr(&ib, db_decl)?;
            let gat = meta.gat.as_ref().unwrap();
            iter_defs.extend(quote! {
                #[doc = concat!("Iterator of derived arrow `", stringify!(#arrow_doc_name), "`: its siblings chained, self-loops twice.")]
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

        let Some(d) = info.backing else { continue };
        match card_of_decl(d) {
            Cardinality::Many => {
                let d_name = &d.name;
                let gat = meta.gat.as_ref().unwrap();
                let walk_ty = it.clone();
                let walk_fields = d
                    .clause(|c| matches!(c, Clause::Iter { .. }))
                    .map(|c| match c {
                        Clause::Iter { fields, .. } => fields
                            .iter()
                            .map(|(n, t)| {
                                let ty = roles::rewrite_ty(
                                    &t.to_token_stream(),
                                    &FlatRoles::new(d, d_expr, false, d_expr.target.clone(), &store_overrides),
                                );
                                let field = Ident2::new(&n.to_string(), Span::mixed_site());
                                quote!(#field: #ty,)
                            })
                            .collect::<Vec<_>>(),
                        _ => unreachable!(),
                    })
                    .unwrap_or_default();
                let adv = body_of(d, |c| matches!(c, Clause::Advance { .. }))
                    .ok_or_else(|| syn::Error::new(Span::call_site(), format!("backing `{}` authors no advance(w) body", d.name)))?;
                let adv_roles = FlatRoles::new(d, d_expr, is_root(self_h), tgt_h.clone(), &store_overrides)
                    .at(quote!(self.store))
                    .with_w(quote!(self))
                    .with_walk_ty(walk_ty.clone());
                let adv_rw = roles::rewrite(adv, &adv_roles);
                let iter_doc = match d
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
                    #[doc = concat!("Iterator of `", stringify!(#arrow_doc_name), "' (backing `", stringify!(#d_name), "`): authored Walk, ", #iter_doc, ".")]
                    pub struct #it<'g> {
                        store: &'g #store_ref,
                        #(#walk_fields)*
                    }
                    impl<'g> ::core::iter::Iterator for #it<'g> {
                        type Item = #tgt_h<'g>;
                        fn next(&mut self) -> ::core::option::Option<#tgt_h<'g>> {
                            #adv_rw
                        }
                    }
                });
                let ie = iterate_expr(info, d)?;
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
                let body = body_of(d, |c| matches!(c, Clause::Read { .. }))
                    .ok_or_else(|| syn::Error::new(Span::call_site(), format!("backing `{}` authors no read(a) body", d.name)))?;
                let roles = FlatRoles::new(d, d_expr, false, tgt_h.clone(), &store_overrides)
                    .at(quote!(self.store))
                    .with_a(quote!(self.id));
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
            // emission).
            Cardinality::Keyed => {
                let kid = id_ty(d_expr.key.as_ref().expect("keyed arrows bind a key sort"));
                let key_assoc = meta.key.as_ref().expect("keyed roots publish the key assoc name");
                let body = body_of(d, |c| matches!(c, Clause::Lookup { .. }))
                    .ok_or_else(|| syn::Error::new(Span::call_site(), format!("backing `{}` authors no lookup(a, k) body", d.name)))?;
                let root_self = is_root(self_h);
                let mut roles = FlatRoles::new(d, d_expr, root_self, tgt_h.clone(), &store_overrides)
                    .at(quote!(self.store))
                    .with_k(quote!(key));
                if !root_self {
                    roles = roles.with_a(quote!(self.id));
                }
                let body_rw = roles::rewrite(body, &roles);
                trait_impls.extend(quote! {
                    impl<'g> #trait_path<#marker_path> for #self_h<'g> {
                        type #assoc = #tgt_h<'g>;
                        type #key_assoc = #kid;
                        fn #m(&self, key: #kid) -> ::core::option::Option<#tgt_h<'g>> {
                            #body_rw.map(|id| #tgt_h { store: self.store, id })
                        }
                    }
                });
            }
        }
    }

    out.extend(iter_defs);
    out.extend(trait_impls);
    Ok(out)
}
