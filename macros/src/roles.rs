//! The role-substitution engine (plans/milestone-3, `backing!` grammar):
//! authored bodies are ordinary Rust over a closed role vocabulary, and
//! codegen *binds* the roles per placement by rewriting the body's tokens.
//!
//! The rewrite is textual (spike 5's pinned mechanism), outermost-role-first,
//! with three contracts the plan states and this module enforces:
//!
//! - **Entity recovery.** A role's entity-variable (`rec(x)`: *which*
//!   arena?) is recovered syntactically: the literal parameter spellings
//!   `a`/`b`/`k` name their declaration variables; a loop variable bound by
//!   `for v in records(E)` carries E; tuple destructuring over `maps(A)`
//!   binds the map variable (and `.iter()` over it binds key/value to K/B);
//!   anything else is the *ambient* default `B` (the handoff's ambient-var
//!   rule — the arbitrary expressions that reach a role in the stdlib
//!   bodies are B-valued: walk ids, field values, map values).
//! - **Scaffold idents** (`errs`, `arrow_label`, `det_label`) are re-emitted
//!   with `Span::mixed_site()`, matching the `let` bindings codegen emits
//!   around the spliced body (spike 5, caveat 1: mixed_site is load-bearing
//!   across both macro layers).
//! - **Container paths are absolutized** (`HashMap`/`HashSet` →
//!   `::std::collections::..`): item paths resolve at the backing's *use*
//!   site, where the import may not exist (spike 5, caveat 2).
//!
//! Contributed-field references rename only in *field position* — after a
//! `.` — so authored locals with the same spelling as a logical field name
//! (`let mut offsets = ..` beside the store's `offsets` field) are never
//! touched.

use proc_macro2::{Delimiter, Ident, Span, TokenStream as TokenStream2, TokenTree};
use quote::{quote, ToTokens};

/// The declaration's entity variables.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ent {
    A,
    B,
    K,
}

/// What a loop pattern variable is bound to by the source it draws from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ScopeKind {
    /// An id of this entity variable.
    Id(Ent),
    /// A contributed map (K -> B); `.iter()` over it binds (k, v).
    Map,
}

/// How one emission site binds the roles. Flat codegen implements this with
/// store-level arenas and plain ids; composed codegen (path ids, hosts,
/// partner maps) implements it with its placement-computed shapes — one
/// authored body, per-placement bindings.
pub(crate) trait RoleBinding {
    /// The token for the `store` role at this site (`self` in store
    /// methods, `self.store` on handles, `self.store` inside `next()`).
    fn store(&self) -> TokenStream2;
    /// Substitution for a parameter spelling (`a`, `b`, `k`, `w`). `None`
    /// keeps the authored spelling (hook signatures keep `a`/`b`/`k`;
    /// `w` rewrites to `self` inside `next()`).
    fn param(&self, _name: &str) -> Option<TokenStream2> {
        None
    }
    /// The id-type tokens for an entity variable (`AId` -> `NodeId`).
    fn id_ty(&self, e: Ent) -> TokenStream2;
    /// `rec(x)` — the record of id `x` through its placement-correct path.
    fn rec(&self, arg: TokenStream2, e: Ent) -> TokenStream2;
    /// `records(E)` — placement-polymorphic enumeration of E's ids.
    fn records(&self, e: Ent) -> TokenStream2;
    /// `maps(A)` — every contributed map as `(owner, map)` pairs.
    fn maps(&self) -> TokenStream2;
    /// `host_of(v)` — the host component of an id (vacuous at flat ids).
    fn host_of(&self, arg: TokenStream2, e: Ent) -> TokenStream2;
    /// `idx(id)` — id newtype -> `usize`.
    fn idx(&self, arg: TokenStream2, e: Ent) -> TokenStream2;
    /// `b_arena_of(x)` / `k_arena_of(x)` — the arena hosted by x's record
    /// (the root-level arena of that entity at flat placements).
    fn arena_of(&self, e: Ent, arg: TokenStream2) -> TokenStream2;
    /// `map_from(a)` — the map hosted by a's record (store-level fold at an
    /// unfused-root self).
    fn map_from(&self, arg: TokenStream2) -> TokenStream2;
    /// `determining(x)` — the by-arrow's emitted field on x's record.
    fn determining(&self, arg: TokenStream2) -> TokenStream2;
    /// `handle(x)` — mint the target handle from the Walk's reserved store
    /// ref; `raw` marks a raw-index argument (minted through the id ctor).
    fn handle(&self, arg: TokenStream2, raw: bool) -> TokenStream2;
    /// The emitted name of the iterator (`iter Walk` renames per binding).
    fn walk_ty(&self) -> Option<Ident> {
        None
    }
    /// The Walk's reserved `host` field at embedded-target placements: the
    /// initializer a `Walk { .. }` literal carries, and the anchor a raw
    /// `handle(x)` mints through (r4-advance: the index mint needs the
    /// host; the link walk takes it from the path id and leaves the field
    /// unused). `None` when the Walk carries no host.
    fn walk_host(&self) -> Option<TokenStream2> {
        None
    }
    /// Logical -> emitted field rename for this binding's contributions.
    fn rename_field(&self, logical: &str) -> Option<Ident> {
        let _ = logical;
        None
    }
    /// `true` when a Walk field is declared over a bare `u32` (a raw index:
    /// `handle(x)` mints `#Id(x)` from it rather than path-splitting).
    fn walk_field_is_raw(&self, field: &str) -> bool {
        let _ = field;
        false
    }
    /// `true` when the name is one of this binding's authored Walk fields.
    fn is_walk_field(&self, field: &str) -> bool {
        let _ = field;
        false
    }
}

const ROLE_HEADS: &[&str] = &[
    "rec",
    "records",
    "maps",
    "host_of",
    "idx",
    "b_arena_of",
    "k_arena_of",
    "map_from",
    "determining",
    "handle",
];

/// Rewrite one authored body under a binding.
pub(crate) fn rewrite(ts: &TokenStream2, rb: &dyn RoleBinding) -> TokenStream2 {
    let mut scope: Vec<(String, ScopeKind)> = Vec::new();
    rw(ts, rb, &mut scope)
}

/// Rewrite a *type* (contributed fields, Walk fields): entity-variable id
/// types and container paths substitute; no expression roles appear.
pub(crate) fn rewrite_ty(ts: &TokenStream2, rb: &dyn RoleBinding) -> TokenStream2 {
    rewrite(ts, rb)
}

fn mixed_site_ident(name: &str) -> Ident {
    Ident::new(name, Span::mixed_site())
}

fn rw(ts: &TokenStream2, rb: &dyn RoleBinding, scope: &mut Vec<(String, ScopeKind)>) -> TokenStream2 {
    let tts: Vec<TokenTree> = ts.clone().into_iter().collect();
    let mut out = TokenStream2::new();
    let mut i = 0;
    while i < tts.len() {
        let tt = tts[i].clone();
        match &tt {
            TokenTree::Ident(id) => {
                let s = id.to_string();

                // `for <pat> in <src> { body }` — rewrite the source and
                // bind the pattern's variables to entities for the body.
                if s == "for" {
                    let (consumed, rewritten) = rw_for(&tts[i..], rb, scope);
                    if consumed > 0 {
                        out.extend(rewritten);
                        i += consumed;
                        continue;
                    }
                }

                // Outermost role head followed by its parenthesized argument.
                if ROLE_HEADS.contains(&s.as_str())
                    && matches!(tts.get(i + 1), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis)
                {
                    let arg_group = match &tts[i + 1] {
                        TokenTree::Group(g) => g.stream(),
                        _ => unreachable!("checked above"),
                    };
                    let arg_rw = rw(&arg_group, rb, scope);
                    let rendered = match s.as_str() {
                        "rec" => rb.rec(arg_rw, ent_of(&arg_group, scope)),
                        "records" => rb.records(parse_ent(&arg_group)),
                        "maps" => rb.maps(),
                        "host_of" => rb.host_of(arg_rw, ent_of(&arg_group, scope)),
                        "idx" => rb.idx(arg_rw, ent_of(&arg_group, scope)),
                        "b_arena_of" => rb.arena_of(Ent::B, arg_rw),
                        "k_arena_of" => rb.arena_of(Ent::K, arg_rw),
                        "map_from" => rb.map_from(arg_rw),
                        "determining" => rb.determining(arg_rw),
                        "handle" => rb.handle(arg_rw, is_raw_walk_field(&arg_group, rb)),
                        _ => unreachable!("ROLE_HEADS is exhaustive"),
                    };
                    out.extend(rendered);
                    i += 2;
                    continue;
                }

                // The Walk literal: `Walk { .. }` gains the reserved store
                // ref and renames to the binding's iterator type.
                if s == "Walk"
                    && matches!(tts.get(i + 1), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace)
                {
                    let fields = match &tts[i + 1] {
                        TokenTree::Group(g) => rw_walk_literal(&g.stream(), rb, scope),
                        _ => unreachable!("checked above"),
                    };
                    match (rb.walk_ty(), rb.walk_host()) {
                        (Some(w), Some(host)) => {
                            let store = rb.store();
                            out.extend(quote!(#w { store: #store, host: #host, #fields }));
                        }
                        (Some(w), None) => {
                            let store = rb.store();
                            out.extend(quote!(#w { store: #store, #fields }));
                        }
                        (None, _) => out.extend(quote!(Walk { #fields })),
                    }
                    i += 2;
                    continue;
                }

                // Plain identifiers: roles, entity-variable types, params,
                // containers, scaffolds.
                match s.as_str() {
                    "store" => out.extend(rb.store()),
                    "AId" => out.extend(rb.id_ty(Ent::A)),
                    "BId" => out.extend(rb.id_ty(Ent::B)),
                    "KId" => out.extend(rb.id_ty(Ent::K)),
                    "HashMap" => out.extend(quote!(::std::collections::HashMap)),
                    "HashSet" => out.extend(quote!(::std::collections::HashSet)),
                    "errs" | "arrow_label" | "det_label" => {
                        out.extend(mixed_site_ident(&s).to_token_stream())
                    }
                    "Walk" => match rb.walk_ty() {
                        Some(w) => out.extend(w.to_token_stream()),
                        None => out.extend(tt.to_token_stream()),
                    },
                    "a" | "b" | "k" | "w" => match rb.param(&s) {
                        Some(sub) => out.extend(sub),
                        // Kept authored spelling: a hook parameter (bound by
                        // the emitted signature with mixed-site spans) or a
                        // loop-bound local (bound by the pattern, courier
                        // spans — keep the body's own span so they pair).
                        None if scope.iter().any(|(n, _)| *n == s) => {
                            out.extend(tt.to_token_stream())
                        }
                        None => out.extend(mixed_site_ident(&s).to_token_stream()),
                    },
                    _ => out.extend(tt.to_token_stream()),
                }
                i += 1;
            }
            TokenTree::Punct(p) if p.as_char() == '.' => {
                // Field position: `.<logical>` renames to the emitted field
                // name; `.<walk-field>` re-emits with the mixed-site span
                // the emitted Walk struct carries; method calls and tuple
                // index accesses pass through.
                if let Some(TokenTree::Ident(f)) = tts.get(i + 1) {
                    if let Some(renamed) = rb.rename_field(&f.to_string()) {
                        out.extend(p.to_token_stream());
                        out.extend(renamed.to_token_stream());
                        i += 2;
                        continue;
                    }
                    let is_call = matches!(tts.get(i + 2), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis);
                    if !is_call && rb.is_walk_field(&f.to_string()) {
                        let mixed = mixed_site_ident(&f.to_string());
                        out.extend(p.to_token_stream());
                        out.extend(mixed.to_token_stream());
                        i += 2;
                        continue;
                    }
                }
                out.extend(tt.to_token_stream());
                i += 1;
            }
            TokenTree::Group(g) => {
                let inner = rw(&g.stream(), rb, scope);
                let mut g2 = proc_macro2::Group::new(g.delimiter(), inner);
                g2.set_span(g.span());
                out.extend(g2.to_token_stream());
                i += 1;
            }
            _ => {
                out.extend(tt.to_token_stream());
                i += 1;
            }
        }
    }
    out
}

/// The inside of a `Walk { .. }` literal: split on top-level commas, re-key
/// each `field: value` (or shorthand `field`) with the mixed-site span the
/// emitted Walk struct's fields carry, and rewrite the values as bodies.
fn rw_walk_literal(
    ts: &TokenStream2,
    rb: &dyn RoleBinding,
    scope: &mut Vec<(String, ScopeKind)>,
) -> TokenStream2 {
    let tts: Vec<TokenTree> = ts.clone().into_iter().collect();
    let mut entries: Vec<Vec<TokenTree>> = vec![Vec::new()];
    for tt in tts {
        if let TokenTree::Punct(p) = &tt {
            if p.as_char() == ',' {
                entries.push(Vec::new());
                continue;
            }
        }
        if matches!(tt, TokenTree::Group(_)) {
            // A nested group may contain its own commas; keep it intact as
            // one token tree within the current entry (values are rewritten
            // by the generic pass below).
            entries.last_mut().unwrap().push(tt);
            continue;
        }
        entries.last_mut().unwrap().push(tt);
    }
    let mut out = TokenStream2::new();
    for entry in entries {
        if entry.is_empty() {
            continue;
        }
        // `key : value` (single top-level colon) or shorthand `key`.
        let colon = entry
            .iter()
            .position(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ':'));
        match (colon, entry.first()) {
            (Some(1), Some(TokenTree::Ident(key))) => {
                let mixed = mixed_site_ident(&key.to_string());
                let value =
                    rw(&entry[2..].iter().cloned().collect::<TokenStream2>(), rb, scope);
                out.extend(quote!(#mixed : #value,));
            }
            (None, Some(TokenTree::Ident(key))) if entry.len() == 1 => {
                let mixed = mixed_site_ident(&key.to_string());
                out.extend(quote!(#mixed,));
            }
            _ => {
                let raw = entry.into_iter().collect::<TokenStream2>();
                out.extend(rw(&raw, rb, scope));
                out.extend(quote!(,));
            }
        }
    }
    out
}

/// Rewrite `for <pat> in <src> { body }`, binding pattern variables to
/// entities from the source's role shape. Returns (tokens consumed, output);
/// consumed == 0 means "not a for-loop we recognize" (emit verbatim).
fn rw_for(
    tts: &[TokenTree],
    rb: &dyn RoleBinding,
    scope: &mut Vec<(String, ScopeKind)>,
) -> (usize, TokenStream2) {
    // [for] <pat...> [in] <src...> [{ body }]
    let mut i = 1; // past `for`
    let mut pat: Vec<TokenTree> = Vec::new();
    while i < tts.len() && !matches!(&tts[i], TokenTree::Ident(x) if x.to_string() == "in") {
        pat.push(tts[i].clone());
        i += 1;
    }
    if i >= tts.len() {
        return (0, TokenStream2::new());
    }
    i += 1; // past `in`
    let mut src: Vec<TokenTree> = Vec::new();
    while i < tts.len() && !matches!(&tts[i], TokenTree::Group(g) if g.delimiter() == Delimiter::Brace) {
        src.push(tts[i].clone());
        i += 1;
    }
    let Some(TokenTree::Group(body_g)) = tts.get(i) else {
        return (0, TokenStream2::new());
    };
    let body = body_g.stream();
    let consumed = i + 1;

    // What does the source enumerate?
    enum Src {
        Records(Ent),
        Maps,
        MapIter,
        Other,
    }
    let src_kind = if src.len() == 2
        && matches!(&src[0], TokenTree::Ident(x) if x.to_string() == "records")
        && matches!(&src[1], TokenTree::Group(g) if g.delimiter() == Delimiter::Parenthesis)
    {
        let e = match &src[1] {
            TokenTree::Group(g) => parse_ent(&g.stream()),
            _ => unreachable!(),
        };
        Src::Records(e)
    } else if src.len() == 2
        && matches!(&src[0], TokenTree::Ident(x) if x.to_string() == "maps")
        && matches!(&src[1], TokenTree::Group(g) if g.delimiter() == Delimiter::Parenthesis)
    {
        Src::Maps
    } else if matches!(&src.get(2), Some(TokenTree::Ident(m)) if m.to_string() == "iter")
        && matches!(&src.get(1), Some(TokenTree::Punct(p)) if p.as_char() == '.')
        // `iter()` scans as [m, ., iter, ()] — the call's empty parens ride
        // along as a fourth token.
        && (src.len() == 3
            || matches!(&src.get(3), Some(TokenTree::Group(g)) if g.stream().is_empty()))
        && matches!(&src.first(), Some(TokenTree::Ident(v))
            if scope.iter().any(|(n, k)| *n == v.to_string() && *k == ScopeKind::Map))
    {
        Src::MapIter
    } else {
        Src::Other
    };

    // Bind pattern variables, then rewrite under the extended scope. Tuple
    // patterns keep their elements inside a paren group — flatten one level
    // so `(k, v)` binds like `v` does (a missed binding would re-emit the
    // body's role-named elements mixed-site instead of keeping the pattern's
    // courier spans, and they would stop pairing).
    let pushed = scope.len();
    let pat_idents: Vec<String> = pat
        .iter()
        .flat_map(|t| match t {
            TokenTree::Ident(id) => vec![id.to_string()],
            TokenTree::Group(g) if g.delimiter() == Delimiter::Parenthesis => g
                .stream()
                .into_iter()
                .filter_map(|t| match t {
                    TokenTree::Ident(id) => Some(id.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    match (&src_kind, pat_idents.as_slice()) {
        (Src::Records(e), [v]) => scope.push((v.clone(), ScopeKind::Id(*e))),
        (Src::Maps, [owner, m]) => {
            // owner: the anchoring id (vacuous host at flat placements —
            // never role-examined); m: the map (K -> B).
            scope.push((owner.clone(), ScopeKind::Id(Ent::A)));
            scope.push((m.clone(), ScopeKind::Map));
        }
        (Src::MapIter, [k, v]) if pat_is_tuple(&pat) => {
            scope.push((k.clone(), ScopeKind::Id(Ent::K)));
            scope.push((v.clone(), ScopeKind::Id(Ent::B)));
        }
        _ => {}
    }
    let src_rw = rw(&src.into_iter().collect::<TokenStream2>(), rb, scope);
    let body_rw = rw(&body, rb, scope);
    scope.truncate(pushed); // bindings live only in the loop body

    let out = quote!(for #(#pat)* in #src_rw { #body_rw });
    (consumed, out)
}

fn pat_is_tuple(pat: &[TokenTree]) -> bool {
    matches!(pat.first(), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis)
}

fn parse_ent(ts: &TokenStream2) -> Ent {
    match ts.to_string().trim() {
        "A" => Ent::A,
        "B" => Ent::B,
        "K" => Ent::K,
        other => panic!("entity-variable role argument is `{other}`, not A/B/K"),
    }
}

/// Entity of a role argument: literal parameter spelling, then scope, then
/// the ambient default B.
fn ent_of(arg: &TokenStream2, scope: &[(String, ScopeKind)]) -> Ent {
    let s = arg.to_string();
    match s.trim() {
        "a" => Ent::A,
        "b" => Ent::B,
        "k" => Ent::K,
        other => scope
            .iter()
            .rev()
            .find(|(n, k)| *n == other && matches!(k, ScopeKind::Id(_)))
            .map(|(_, k)| match k {
                ScopeKind::Id(e) => *e,
                ScopeKind::Map => Ent::B,
            })
            .unwrap_or(Ent::B),
    }
}

/// `handle(x)`'s raw-index test: the argument is a Walk field access whose
/// declared type is a bare `u32`.
fn is_raw_walk_field(arg: &TokenStream2, rb: &dyn RoleBinding) -> bool {
    let tts: Vec<TokenTree> = arg.clone().into_iter().collect();
    if tts.len() == 3 {
        if let (TokenTree::Ident(_), TokenTree::Punct(p), TokenTree::Ident(f)) = (&tts[0], &tts[1], &tts[2]) {
            return p.as_char() == '.' && rb.walk_field_is_raw(&f.to_string());
        }
    }
    false
}
