//! Proc macros for metagraph: `pattern!` (declare an abstract type graph),
//! `implement!` (bind arrows to backings, generate a store), and the
//! doc-hidden `__implement_gen!` courier target.
//!
//! Schema information travels between macros only through generated code:
//! every `pattern!` emits a *courier* macro_rules with exactly one
//! target-parameterized rule — the caller names the forwarding target, and
//! the schema (or, for root patterns, the trait-metadata tuple) rides inside
//! the target's delimiter group. All grammar lives in the proc macros'
//! parsers (shared module [`grammar`]), where errors get real spans.

mod codegen;
mod compose;
mod grammar;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{braced, bracketed, Ident, Path, Token};

use grammar::{
    parse_bindings, path_tokens, root_meta_tokens, snake, Binding, Cardinality, RootMeta,
};

// ---------------------------------------------------------------------------
// pattern!
// ---------------------------------------------------------------------------

struct PatternInput {
    name: Ident,
    body: grammar::SchemaBody,
}

impl Parse for PatternInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        let content;
        braced!(content in input);
        Ok(PatternInput { name, body: content.parse()? })
    }
}

/// Declare an abstract type graph: entities and single-arrow relations
/// ("capabilities") with cardinalities. No storage — backings bind at
/// `implement!` time.
///
/// One entity per pattern is marked `root`: its record is the store
/// itself, so it owns no id, record, or arena — only its handle and the
/// store's root accessor. No arrow may *target* it (there is no id to
/// store); arrows *sourced* at it are fine.
///
/// A single-arrow pattern with no instantiation source is a **root**
/// pattern: it declares a fresh trait and registers its metadata so any
/// crate can instantiate it. Any other pattern instantiates roots by
/// path and never restates cardinality or associated types — the local
/// arrow name is an alias; read-side identity comes from the root.
///
/// ```ignore
/// pattern! { Nodes { entities: root Graph, Node; arrows { nodes: Graph -> Node, many; } } }
/// pattern! {
///     Digraph {
///         entities: root Graph, Node, Edge;
///         arrows {
///             nodes:     stdlib::Nodes,    Graph -> Node;
///             out_edges: stdlib::OutEdges, Node -> Edge, by source;
///             edges:     stdlib::Edges,    Node -> Edge := sum(out_edges, in_edges);
///         }
///     }
/// }
/// ```
///
/// Note: declare patterns in a named module (as the stdlib does). A
/// pattern declared at a crate root collides with its courier's
/// `#[macro_export]` crate-root anchor (E0255).
#[proc_macro]
pub fn pattern(input: TokenStream) -> TokenStream {
    let pat = syn::parse_macro_input!(input as PatternInput);
    match expand_pattern(&pat) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_pattern(pat: &PatternInput) -> syn::Result<TokenStream2> {
    validate_pattern(pat)?;
    let is_root_pattern = pat.body.arrows.iter().all(|a| a.inst.is_none());
    if is_root_pattern {
        expand_root_pattern(pat)
    } else {
        expand_instantiation_pattern(pat)
    }
}

/// Structural checks that need only the declaring pattern (real spans).
/// The compose-time checks of M2 run at the compose chain's final hop; the
/// implement-time checks (backings, `by`-totality) run in `__implement_gen!`.
fn validate_pattern(pat: &PatternInput) -> syn::Result<()> {
    let body = &pat.body;

    // Exactly one root-marked entity; it is read by the store's root accessor
    // and (M2) the placement rule — never derived from arrow shape.
    let roots: Vec<&grammar::EntityDecl> =
        body.entities.iter().filter(|e| e.root).collect();
    match roots.len() {
        1 => {}
        0 => {
            return Err(syn::Error::new(
                pat.name.span(),
                format!(
                    "pattern `{}` must mark exactly one entity `root` (the entity whose record is the store itself)",
                    pat.name
                ),
            ))
        }
        n => {
            let names: Vec<String> = roots.iter().map(|e| e.name.to_string()).collect();
            return Err(syn::Error::new(
                roots[1].name.span(),
                format!(
                    "pattern `{}` marks {} entities `root` ({}); exactly one is allowed",
                    pat.name,
                    n,
                    names.join(", ")
                ),
            ));
        }
    }

    // No duplicate entity or arrow names.
    for (i, e) in body.entities.iter().enumerate() {
        if body.entities[..i].iter().any(|x| x.name == e.name) {
            return Err(syn::Error::new(
                e.name.span(),
                format!("duplicate entity `{}`", e.name),
            ));
        }
    }
    for (i, a) in body.arrows.iter().enumerate() {
        if body.arrows[..i].iter().any(|x| x.name == a.name) {
            return Err(syn::Error::new(
                a.name.span(),
                format!("duplicate arrow `{}`", a.name),
            ));
        }
    }

    for a in &body.arrows {
        // Arrows reference declared entities.
        for (what, x) in [("source", &a.source), ("target", &a.target)] {
            if !body.entities.iter().any(|e| e.name == *x) {
                return Err(syn::Error::new(
                    x.span(),
                    format!("arrow `{}` names undeclared {what} entity `{}`", a.name, x),
                ));
            }
        }

        // No arrow may TARGET the root-marked entity: after the no-arena rule
        // it owns no id type to store. Arrows sourced at it are fine.
        if let Some(r) = body.entities.iter().find(|e| e.root) {
            if a.target == r.name {
                return Err(syn::Error::new(
                    a.name.span(),
                    format!(
                        "arrow `{}` targets the root-marked entity `{}` (its record is the store itself; there is no id to store) — arrows may only be sourced at it",
                        a.name, r.name
                    ),
                ));
            }
        }

        // Open entities are arrow targets only in their declaring pattern.
        if let Some(e) = body.entities.iter().find(|e| e.open && e.name == a.source) {
            return Err(syn::Error::new(
                a.name.span(),
                format!(
                    "open entity `{}` may only be the target of arrows in its declaring pattern",
                    e.name
                ),
            ));
        }

        if a.by.is_some() && a.sum.is_some() {
            return Err(syn::Error::new(
                a.name.span(),
                format!("arrow `{}` is derived both `by` and `:= sum(..)`; pick one", a.name),
            ));
        }

        // `by` and `:= sum` are properties of instantiations, not roots.
        if a.inst.is_none() && (a.by.is_some() || a.sum.is_some()) {
            return Err(syn::Error::new(
                a.name.span(),
                format!(
                    "root arrow `{}` cannot carry `by`/`:= sum(..)` — derivations are properties of instantiations",
                    a.name
                ),
            ));
        }

        // `key(K)` does not combine with `by` or `:= sum(..)` (keyed arrows
        // bind explicitly and are never derived).
        if a.key.is_some() && (a.by.is_some() || a.sum.is_some()) {
            return Err(syn::Error::new(
                a.name.span(),
                format!(
                    "arrow `{}` is keyed (`key(..)`) and cannot also be derived (`by`/`:= sum(..)`)",
                    a.name
                ),
            ));
        }
        // `key(K)` IS the root's cardinality declaration — not combinable
        // with `one`/`many` on a root, and never restated by an
        // instantiation (the instantiation's `key(E)` only binds the sort).
        if a.key.is_some() && a.many.is_some() {
            return Err(syn::Error::new(
                a.name.span(),
                format!(
                    "arrow `{}` declares both a cardinality and `key(..)`; on a root, `key(K)` is the cardinality",
                    a.name
                ),
            ));
        }
        // The key entity must be declared in the pattern, and it must own an
        // id type — the root-marked entity owns none, so it cannot be keyed
        // on. (Open entities may be keys on the *instantiation* side, where
        // the clause binds a local sort that `<->` closes; a root pattern
        // declaring its own key on an open entity would leave the key
        // unresolved, so reject that too.)
        if let Some(k) = &a.key {
            let ke = body.entities.iter().find(|e| e.name == *k).ok_or_else(|| {
                syn::Error::new(
                    k.span(),
                    format!("`key({})` names an entity `{}` does not declare", k, pat.name),
                )
            })?;
            if a.inst.is_none() {
                // Root side: the key must be an ordinary element entity.
                if ke.root {
                    return Err(syn::Error::new(
                        k.span(),
                        format!(
                            "`key({})` names the root-marked entity; the root owns no id type to key on",
                            k
                        ),
                    ));
                }
                if ke.open {
                    return Err(syn::Error::new(
                        k.span(),
                        format!(
                            "root arrow `{}` keys on open entity `{}`; a root pattern's key must be an entity it owns",
                            a.name, k
                        ),
                    ));
                }
            } else if ke.root {
                return Err(syn::Error::new(
                    k.span(),
                    format!(
                        "`key({})` names the root-marked entity; the root owns no id type to key on",
                        k
                    ),
                ));
            }
        }

        // Cardinality is declared on the root, inherited by instantiations.
        if let (Some(inst), Some(_)) = (&a.inst, a.many) {
            let it = path_tokens(inst);
            return Err(syn::Error::new(
                a.name.span(),
                format!(
                    "instantiation `{}` restates cardinality — `{}` declares it on the root",
                    a.name, it
                ),
            ));
        }
        if a.inst.is_none() && a.many.is_none() && a.key.is_none() {
            return Err(syn::Error::new(
                a.name.span(),
                format!(
                    "root arrow `{}` must declare a cardinality (`one`, `many`, or `key(K)`)",
                    a.name
                ),
            ));
        }

        // `by <det>`: the determinant is a declared sibling arrow running
        // opposite (target-of-by -> source-of-by); totality is checked at
        // `implement!` time, where the root's cardinality is known.
        if let Some(det_name) = &a.by {
            let det = body.arrows.iter().find(|x| x.name == *det_name).ok_or_else(|| {
                syn::Error::new(
                    det_name.span(),
                    format!("`by` names arrow `{}`, which this pattern does not declare", det_name),
                )
            })?;
            if det.source != a.target || det.target != a.source {
                return Err(syn::Error::new(
                    det_name.span(),
                    format!(
                        "`by {}` names arrow `{}: {} -> {}`, which does not run opposite `{}: {} -> {}` (the determining field lives on the target record)",
                        det_name, det.name, det.source, det.target, a.name, a.source, a.target
                    ),
                ));
            }
        }

        // `:= sum(a, b)`: siblings with the same self and associated entity.
        if let Some((ua, ub)) = &a.sum {
            for u in [ua, ub] {
                let d = body.arrows.iter().find(|x| x.name == *u).ok_or_else(|| {
                    syn::Error::new(u.span(), format!("sum names undeclared sibling `{}`", u))
                })?;
                if d.source != a.source || d.target != a.target {
                    return Err(syn::Error::new(
                        u.span(),
                        format!(
                            "sum sibling `{}: {} -> {}` does not share `{}: {} -> {}`'s self and associated entities",
                            d.name, d.source, d.target, a.name, a.source, a.target
                        ),
                    ));
                }
            }
        }
    }

    // Shape: a root pattern declares exactly one arrow (and it has no
    // instantiation source); an instantiation pattern instantiates a root
    // for every arrow.
    if body.arrows.is_empty() {
        return Err(syn::Error::new(
            pat.name.span(),
            format!("pattern `{}` declares no arrows", pat.name),
        ));
    }
    let declared = body.arrows.iter().filter(|a| a.inst.is_none()).count();
    if declared > 0 && declared < body.arrows.len() {
        return Err(syn::Error::new(
            pat.name.span(),
            format!(
                "pattern `{}` mixes root-declared arrows with instantiations; a pattern is either a root single-arrow pattern or instantiates a root for every arrow",
                pat.name
            ),
        ));
    }
    if declared > 0 && body.arrows.len() != 1 {
        return Err(syn::Error::new(
            pat.name.span(),
            "a root pattern declares exactly one arrow",
        ));
    }
    Ok(())
}

/// A root pattern owns its trait and registers it: trait + registration
/// courier (metadata tuple) + doc-hidden `pub use` at the invocation site.
fn expand_root_pattern(pat: &PatternInput) -> syn::Result<TokenStream2> {
    let a = &pat.body.arrows[0];
    let trait_name = &pat.name;
    let m = &a.name;
    let assoc = &a.target;
    let meta = RootMeta::of_root_arrow(a)?;
    let doc = format!(
        "Root arrow `{}: {} -> {}` ({}).",
        a.name,
        a.source,
        a.target,
        match meta.card {
            Cardinality::Many => "many",
            Cardinality::One => "one",
            Cardinality::Keyed => "keyed partial",
        }
    );
    let mut out = match meta.card {
        Cardinality::Many => {
            let gat = meta.gat.as_ref().unwrap();
            quote! {
                #[doc = #doc]
                pub trait #trait_name<M> {
                    type #assoc;
                    type #gat<'a>: ::core::iter::Iterator<Item = Self::#assoc>
                    where
                        Self: 'a;
                    fn #m(&self) -> Self::#gat<'_>;
                }
            }
        }
        Cardinality::One => {
            quote! {
                #[doc = #doc]
                pub trait #trait_name<M> {
                    type #assoc;
                    fn #m(&self) -> Self::#assoc;
                }
            }
        }
        // Keyed partial (probe-verified, /tmp/m2-probe-edge-induced): the
        // root cannot name the instantiation's key sort, so the trait carries
        // it as an associated type and the method takes it as an argument.
        Cardinality::Keyed => {
            let key = meta.key.as_ref().unwrap();
            quote! {
                #[doc = #doc]
                pub trait #trait_name<M> {
                    type #assoc;
                    type #key;
                    fn #m(&self, key: Self::#key) -> ::core::option::Option<Self::#assoc>;
                }
            }
        }
    };

    // Registration courier: publishes the metadata tuple. Same
    // target-parameterized rule as every courier — the caller names the
    // forwarding target, the tuple rides inside its delimiter group.
    let courier = format_ident!("__metagraph_pattern_{}", pat.name);
    let meta_ts = root_meta_tokens(&meta);
    out.extend(quote! {
        #[doc(hidden)]
        #[macro_export]
        macro_rules! #courier {
            (($($fwd:tt)::+ ; $($rest:tt)*)) => {
                $($fwd)::+ ! { { #meta_ts } ; $($rest)* }
            };
        }
        #[doc(hidden)]
        #[allow(unused_imports)]
        pub use #courier;
    });
    Ok(out)
}

/// An instantiation pattern emits its markers module and schema courier
/// under `pub mod <snake(name)>` at the invocation site.
fn expand_instantiation_pattern(pat: &PatternInput) -> syn::Result<TokenStream2> {
    let mod_name = Ident::new(&snake(&pat.name.to_string()), pat.name.span());
    let courier = format_ident!("__metagraph_pattern_{}", pat.name);

    let marker_items = pat.body.arrows.iter().map(|a| {
        let marker = grammar::marker_ty(&a.name);
        let arrow_name = &a.name;
        let inst_ts = a.inst.as_ref().map(path_tokens).unwrap_or_default();
        quote! {
            #[doc = concat!("Marker for the arrow instantiation `", stringify!(#arrow_name), ": ", stringify!(#inst_ts), "`.")]
            #[derive(Debug, Clone, Copy)]
            pub struct #marker;
        }
    });

    let schema_tokens = {
        // Declaring-crate resolver: bake the declaring crate's package name
        // into the courier payload. `pattern!` runs in the declaring crate's
        // rustc process, so `CARGO_PKG_NAME` names the declaring crate — the
        // eventual consumer's expansion sees the baked string, not its own.
        let pkg = std::env::var("CARGO_PKG_NAME")
            .expect("CARGO_PKG_NAME not set during pattern! expansion");
        let packaged = grammar::PackagedSchema { pkg, body: pat.body.clone() };
        grammar::packaged_schema_tokens(&packaged)
    };

    Ok(quote! {
        pub mod #mod_name {
            /// Per-arrow-instantiation markers: module paths, never name
            /// concatenation. One unit struct per arrow instantiation (per
            /// use, not per root).
            pub mod markers {
                #(#marker_items)*
            }
        }
        // Target-parameterized courier: the schema rides inside the
        // forwarding target's delimiter group. M1's only target is
        // `__implement_gen!`; M2's composition chain and M1's own
        // root-metadata resolution reuse this shape with other targets.
        // The `pub use` lands here, at the invocation site, so the courier
        // is addressable as <invoking module>::__metagraph_pattern_<Name>.
        #[doc(hidden)]
        #[macro_export]
        macro_rules! #courier {
            (($($fwd:tt)::+ ; $($rest:tt)*)) => {
                $($fwd)::+ ! { { #schema_tokens } ; $($rest)* }
            };
        }
        #[doc(hidden)]
        #[allow(unused_imports)]
        pub use #courier;
    })
}

// ---------------------------------------------------------------------------
// compose_pattern! and the __compose_step! courier chain
// (plans/milestone-2, "the courier chain")
// ---------------------------------------------------------------------------

/// Replace a pattern path's last segment with its courier name
/// (`P::Name` -> `P::__metagraph_pattern_Name`).
fn courier_path(pattern_path: &Path) -> syn::Result<Path> {
    if pattern_path.segments.len() < 2 {
        let last = pattern_path.segments.last().unwrap();
        return Err(syn::Error::new(
            last.ident.span(),
            format!(
                "`{}` is a bare pattern name; import the pattern's module and pass its path (e.g. `m2_fixtures::shapes::{}`)",
                last.ident, last.ident
            ),
        ));
    }
    let mut p = pattern_path.clone();
    let last = p.segments.last_mut().expect("pattern path nonempty");
    last.ident = format_ident!("__metagraph_pattern_{}", last.ident);
    Ok(p)
}

struct ComposeInput {
    name: Ident,
    instances: Vec<(Ident, Path)>,
    /// Raw fusion declarations, parsed into `Fusion`s only at the final hop
    /// where all member schemas are in hand.
    fusions: TokenStream2,
}

impl Parse for ComposeInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        let body;
        braced!(body in input);
        let kw: Ident = body.parse()?;
        if kw != "instances" {
            return Err(syn::Error::new(kw.span(), "expected `instances`"));
        }
        body.parse::<Token![:]>()?;
        let mut instances = Vec::new();
        loop {
            let inst: Ident = body.parse()?;
            body.parse::<Token![:]>()?;
            let path: Path = body.parse()?;
            instances.push((inst, path));
            if body.peek(Token![,]) {
                body.parse::<Token![,]>()?;
            } else {
                break;
            }
        }
        body.parse::<Token![;]>()?;
        let fusions: TokenStream2 = body.parse()?;
        if instances.is_empty() {
            return Err(syn::Error::new(
                name.span(),
                "compose_pattern! needs at least one instance",
            ));
        }
        for (i, (inst, _)) in instances.iter().enumerate() {
            if instances[..i].iter().any(|(x, _)| x == inst) {
                return Err(syn::Error::new(
                    inst.span(),
                    format!("duplicate instance name `{inst}` in compose_pattern!"),
                ));
            }
        }
        Ok(ComposeInput { name, instances, fusions })
    }
}

/// Compose pattern instances by declaring entity fusions:
///
/// ```ignore
/// compose_pattern! {
///     EdgeInduced {
///         instances: main: metagraph_exper::stdlib::Digraph, sub: m2_fixtures::shapes::Subgraph;
///         sub.MainNode <-> main.Node;
///         sub.MainEdge <-> main.Edge;
///     }
/// }
/// ```
///
/// `<=>` is colocation fusion (one record, summed capabilities);
/// `<->` is reference fusion (closes an open entity, unifying id types
/// only). A proc macro cannot read its member patterns' tokens, so this
/// emits an invocation of the first member's schema courier with the
/// doc-hidden `__compose_step!` as the forwarding target; each hop appends
/// one member schema and forwards, and the final hop validates the
/// composition, mints per-instance markers, and emits the composed
/// pattern's own courier — after which it is indistinguishable from a
/// hand-written pattern.
#[proc_macro]
pub fn compose_pattern(input: TokenStream) -> TokenStream {
    let comp = syn::parse_macro_input!(input as ComposeInput);
    match expand_compose(&comp) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_compose(comp: &ComposeInput) -> syn::Result<TokenStream2> {
    let rt = runtime_target();
    let name = &comp.name;
    let modname = Ident::new(&snake(&name.to_string()), name.span());
    let (head_inst, head_path) = &comp.instances[0];
    let first_courier = courier_path(head_path)?;
    let rest_pairs: Vec<TokenStream2> = comp.instances[1..]
        .iter()
        .map(|(inst, path)| {
            let cp = courier_path(path)?;
            Ok(quote!((#inst, #cp)))
        })
        .collect::<syn::Result<_>>()?;
    let fusions = &comp.fusions;
    // Emit ONLY the first hop; the chain carries the queue and the schemas
    // collected so far. Brace-delimited: the invocation sits in item
    // position (spike-pinned). Fusions stay opaque token soup until the
    // final hop.
    Ok(quote! {
        #first_courier! {
            (#rt::__compose_step ;
                name = (#name)
                mod = (#modname)
                head = (#head_inst)
                rest = ( #(#rest_pairs)* )
                fusions = { #fusions }
                collected = ()
            )
        }
    })
}

/// `inst = (tokens)` fields out of a step payload tail.
fn step_fields(ts: TokenStream2) -> Vec<(String, TokenStream2)> {
    let mut out = Vec::new();
    let mut it = ts.into_iter();
    while let Some(proc_macro2::TokenTree::Ident(id)) = it.next() {
        match it.next() {
            Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == '=' => {}
            other => {
                return vec![("__error__".into(), quote!(expected eq after #id, got #other))];
            }
        }
        match it.next() {
            Some(proc_macro2::TokenTree::Group(g)) => out.push((id.to_string(), g.stream())),
            other => {
                return vec![("__error__".into(), quote!(expected group for #id, got #other))];
            }
        }
    }
    out
}

fn step_field<'a>(
    fields: &'a [(String, TokenStream2)],
    name: &str,
) -> syn::Result<&'a TokenStream2> {
    fields
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, ts)| ts)
        .ok_or_else(|| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("compose step payload missing `{name}`"),
            )
        })
}

/// `( ( inst , path )* )` — the remaining-courier queue.
struct RestPair {
    inst: Ident,
    path: Path,
}
impl Parse for RestPair {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let inst = input.parse()?;
        input.parse::<Token![,]>()?;
        let path = input.parse()?;
        Ok(RestPair { inst, path })
    }
}

fn parse_rest(ts: &TokenStream2) -> syn::Result<Vec<(Ident, Path)>> {
    let mut out = Vec::new();
    for tt in ts.clone() {
        let proc_macro2::TokenTree::Group(g) = tt else {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "rest entry must be a parenthesized (instance, courier) pair",
            ));
        };
        let p: RestPair = syn::parse2(g.stream())?;
        out.push((p.inst, p.path));
    }
    Ok(out)
}

/// One hop of the composition chain. The invoking member courier has
/// already wrapped its packaged schema around the payload; this appends it
/// to the collected set and either forwards to the next member's courier
/// or — queue empty — finishes the composition.
#[doc(hidden)]
#[proc_macro]
pub fn __compose_step(input: TokenStream) -> TokenStream {
    match compose_step(TokenStream2::from(input)) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn compose_step(ts: TokenStream2) -> syn::Result<TokenStream2> {
    // `{ packaged schema } ; <payload fields>` — the schema stays a raw
    // group; only the final hop parses it.
    let mut it = ts.into_iter();
    let schema_group = match it.next() {
        Some(proc_macro2::TokenTree::Group(g)) => g,
        other => {
            return Err(syn::Error::new(
                other.map(|t| t.span()).unwrap_or_else(proc_macro2::Span::call_site),
                "compose step: expected the member's packaged schema group \
                 (is the instance a root single-arrow pattern? roots are \
                 instantiated, not composed)",
            ))
        }
    };
    match it.next() {
        Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == ';' => {}
        other => {
            return Err(syn::Error::new(
                other.map(|t| t.span()).unwrap_or_else(proc_macro2::Span::call_site),
                "compose step: expected `;` after the schema group",
            ))
        }
    }
    let fields = step_fields(it.collect());
    if fields.iter().any(|(n, _)| n == "__error__") {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("compose step: malformed payload: {}", fields[0].1),
        ));
    }
    let name: Ident = syn::parse2(step_field(&fields, "name")?.clone())?;
    let modname: Ident = syn::parse2(step_field(&fields, "mod")?.clone())?;
    let head: Ident = syn::parse2(step_field(&fields, "head")?.clone())?;
    let mut rest = parse_rest(step_field(&fields, "rest")?)?;
    let fusions = step_field(&fields, "fusions")?.clone();
    let mut collected = parse_collected(step_field(&fields, "collected")?);
    collected.push((head, schema_group));

    let Some((next_inst, next_courier)) = rest.first().cloned() else {
        return finish_compose(&name, &modname, fusions, collected);
    };
    rest.remove(0);
    let rest_pairs: Vec<TokenStream2> = rest
        .iter()
        .map(|(inst, path)| quote!((#inst, #path)))
        .collect();
    let collected_pairs: Vec<TokenStream2> = collected
        .iter()
        .map(|(inst, group)| quote!((#inst, #group)))
        .collect();
    let rt = runtime_target();
    Ok(quote! {
        #next_courier! {
            (#rt::__compose_step ;
                name = (#name)
                mod = (#modname)
                head = (#next_inst)
                rest = ( #(#rest_pairs)* )
                fusions = { #fusions }
                collected = ( #(#collected_pairs)* )
            )
        }
    })
}

/// Collected entries in flight: `(inst, <packaged-schema paren group>)`.
fn parse_collected(ts: &TokenStream2) -> Vec<(Ident, proc_macro2::Group)> {
    let mut out = Vec::new();
    for tt in ts.clone() {
        if let proc_macro2::TokenTree::Group(g) = tt {
            let mut inner = g.stream().into_iter();
            if let (Some(proc_macro2::TokenTree::Ident(inst)), Some(proc_macro2::TokenTree::Punct(p)), Some(schema)) =
                (inner.next(), inner.next(), inner.next())
            {
                if p.as_char() == ',' {
                    out.push((inst, match schema {
                        proc_macro2::TokenTree::Group(gg) => gg,
                        _ => continue,
                    }));
                }
            }
        }
    }
    out
}

/// Final hop: all member schemas collected. Run the compose-time structural
/// checks, then mint per-instance markers and emit the composed pattern's
/// own courier — same markers-module layout, same courier rule, same
/// path-derivation rule as a hand-written pattern, which is what makes
/// `implement!` resolution configuration-free.
fn finish_compose(
    name: &Ident,
    modname: &Ident,
    fusions_ts: TokenStream2,
    collected: Vec<(Ident, proc_macro2::Group)>,
) -> syn::Result<TokenStream2> {
    let mut members: Vec<(Ident, String, grammar::SchemaBody)> = Vec::new();
    for (inst, group) in &collected {
        let packaged: grammar::PackagedSchema = syn::parse2(group.stream()).map_err(|e| {
            syn::Error::new(
                e.span(),
                format!("compose_pattern! `{name}`: member `{inst}` schema: {e}"),
            )
        })?;
        members.push((inst.clone(), packaged.pkg, packaged.body));
    }
    let fusions: Vec<grammar::Fusion> = {
        use syn::parse::Parser as _;
        let parse_all = |input: syn::parse::ParseStream| {
            let mut v = Vec::new();
            while !input.is_empty() {
                v.push(input.parse()?);
            }
            Ok(v)
        };
        parse_all
            .parse2(fusions_ts.clone())
            .map_err(|e| syn::Error::new(e.span(), format!("compose_pattern! `{name}`: {e}")))?
    };

    let composed = grammar::ComposedSchema {
        instances: members.iter().map(|(i, _, _)| i.clone()).collect(),
        members,
        fusions,
        pkg: std::env::var("CARGO_PKG_NAME").unwrap_or_default(),
    };
    compose::validate_composition(&composed)
        .map_err(|e| syn::Error::new(e.span(), format!("compose_pattern! `{name}`: {e}")))?;

    // ---- mint per-instance markers: fresh unit structs, never re-exports
    // (re-exported shared markers would produce conflicting impls on fused
    // handles; the diamond case is two markers, not a conflicting impl) ----
    let mut marker_mods = TokenStream2::new();
    for (inst, _, body) in &composed.members {
        let markers: Vec<Ident> = body
            .arrows
            .iter()
            .map(|a| grammar::marker_ty(&a.name))
            .collect();
        let arrow_docs: Vec<String> = body
            .arrows
            .iter()
            .map(|a| format!("Marker for the `{inst}.{}` arrow instantiation.", a.name))
            .collect();
        marker_mods.extend(quote! {
            #[doc = concat!("Per-arrow-instantiation markers of the `", stringify!(#inst), "` instance.")]
            pub mod #inst {
                #(
                    #[doc = #arrow_docs]
                    #[derive(Debug, Clone, Copy)]
                    pub struct #markers;
                )*
            }
        });
    }

    let schema_tokens = grammar::composed_schema_tokens(&composed);
    let courier = format_ident!("__metagraph_pattern_{}", name);

    Ok(quote! {
        // The composed pattern is indistinguishable from a hand-written one:
        // same markers-module layout, same courier rule, same
        // path-derivation rule. Marker paths derive from the composed
        // pattern's path by the M1 rule.
        pub mod #modname {
            /// Per-instance, per-arrow-instantiation markers.
            pub mod markers {
                #marker_mods
            }
            #[doc(hidden)]
            #[macro_export]
            macro_rules! #courier {
                (($($fwd:tt)::+ ; $($rest:tt)*)) => {
                    $($fwd)::+ ! { { #schema_tokens } ; $($rest)* }
                };
            }
            #[doc(hidden)]
            #[allow(unused_imports)]
            pub use #courier;
        }
    })
}



// ---------------------------------------------------------------------------
// implement!
// ---------------------------------------------------------------------------

struct ImplementInput {
    /// The pattern path exactly as written (e.g. `stdlib::Digraph`): its
    /// prefix is the module holding the `pattern!` invocation, its last
    /// segment the pattern name. It never names a real item; `implement!`
    /// derives the courier and markers paths from it.
    pattern_path: Path,
    store: Ident,
    bindings: Vec<Binding>,
}

impl Parse for ImplementInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let pattern_path: Path = input.parse()?;
        let last = pattern_path.segments.last().unwrap();
        if !matches!(last.arguments, syn::PathArguments::None) {
            return Err(syn::Error::new(
                last.ident.span(),
                "pattern path must end in a plain ident (no generics)",
            ));
        }
        input.parse::<Token![as]>()?;
        let store: Ident = input.parse()?;
        let bindings = parse_bindings(input)?;
        Ok(ImplementInput { pattern_path, store, bindings })
    }
}

/// Bind each arrow of a pattern to a *backing* and emit the concrete
/// store, ids, handles, iterators, and trait impls at the call site.
///
/// The pattern is addressed by the path of the module holding its
/// `pattern!` invocation plus the pattern name (`stdlib::Digraph` for
/// the stdlib's, `crate::shapes::Labeled` for one of yours). `many`
/// arrows bind explicitly (`arena` vs `intrusive_list` is a real choice
/// with no safe default); `one` arrows default to `field`; derived
/// (`:= sum`) arrows take no binding.
///
/// ```ignore
/// implement! {
///     metagraph_exper::stdlib::Digraph as GraphStore {
///         nodes:     arena,
///         out_edges: intrusive_list,
///         in_edges:  intrusive_list,
///     }
/// }
/// ```
#[proc_macro]
pub fn implement(input: TokenStream) -> TokenStream {
    let imp = syn::parse_macro_input!(input as ImplementInput);
    match expand_implement(&imp) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_implement(imp: &ImplementInput) -> syn::Result<TokenStream2> {
    let last = imp.pattern_path.segments.last().unwrap().ident.clone();
    let courier = format_ident!("__metagraph_pattern_{}", last);
    if imp.pattern_path.segments.len() < 2 {
        // A single-segment path gives us no prefix to hang the courier on;
        // point at the missing module import rather than the mangled name.
        return Err(syn::Error::new(
            last.span(),
            format!(
                "`{}` is a bare pattern name; import the pattern's module and pass its path (e.g. `metagraph_exper::stdlib::{last}`)",
                last
            ),
        ));
    }
    // The courier's `pub use` landed in the module holding the `pattern!`
    // invocation — that is the path's prefix, without the pattern name.
    let mut prefix = imp.pattern_path.clone();
    prefix.segments.pop();
    let prefix_ts = path_tokens(&prefix);
    let full_ts = path_tokens(&imp.pattern_path);
    let rt = runtime_target();
    let store = &imp.store;
    let bindings_ts = grammar::bindings_tokens(&imp.bindings);
    let out = quote! {
        #prefix_ts::#courier ! {
            (#rt::__implement_gen ; @pattern { #full_ts } ; #store { #bindings_ts })
        }
    };
    if std::env::var("MG_DEBUG").is_ok() {
        eprintln!("implement! emits: {out}");
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// __implement_gen! — two stages over one entry point
// ---------------------------------------------------------------------------

/// Full state riding through the resolution chain, inside the courier's
/// rest tail. `roots` is the queue of registration couriers still to visit
/// (in first-occurrence order over the schema's arrows, each already
/// resolved to its absolute path by the declaring-crate resolver — see
/// [`resolve_schema_path`]); `metas` the tuples collected so far —
/// `metas[i]` belongs to the i-th root of that order.
pub(crate) struct ChainConfig {
    pub(crate) schema: grammar::GenSchema,
    /// The pattern path as written in `implement!` (prefix + pattern name).
    pub(crate) pattern_path: Path,
    pub(crate) store: Ident,
    pub(crate) bindings: Vec<Binding>,
    pub(crate) roots: Vec<Path>,
    pub(crate) metas: Vec<RootMeta>,
}

enum GenInput {
    /// `{ ("pkg", { schema }) } ; @pattern { P } ; Store { bindings }` —
    /// fresh from a pattern's schema courier (flat *or* composed — the
    /// packaging wrapper's body keyword dispatches): parse, validate, start
    /// the resolution chain.
    Stage1 { schema: grammar::GenSchema, pattern_path: Path, store: Ident, bindings: Vec<Binding> },
    /// `{ meta tuple } ; @config { ... }` — one registration courier hop:
    /// the tuple just arrived; the config carries the remaining queue.
    ChainStep { meta: RootMeta, config: ChainConfig },
}

fn parse_schema_group(group: proc_macro2::Group) -> syn::Result<grammar::GenSchema> {
    grammar::parse_gen_schema(group)
}

impl Parse for GenInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let first: proc_macro2::Group = input.parse()?;
        if first.delimiter() != proc_macro2::Delimiter::Brace {
            return Err(syn::Error::new(first.span(), "expected a `{ .. }` group"));
        }
        input.parse::<Token![;]>()?;
        input.parse::<Token![@]>()?;
        let kw: Ident = input.parse()?;
        match kw.to_string().as_str() {
            "pattern" => {
                let schema = parse_schema_group(first)?;
                let pcontent;
                braced!(pcontent in input);
                let pattern_path = pcontent.parse()?;
                input.parse::<Token![;]>()?;
                let store: Ident = input.parse()?;
                let bindings = parse_bindings(input)?;
                Ok(GenInput::Stage1 { schema, pattern_path, store, bindings })
            }
            "config" => {
                let meta = syn::parse2(first.stream())?;
                let config = parse_chain_config(input)?;
                Ok(GenInput::ChainStep { meta, config })
            }
            other => Err(syn::Error::new(kw.span(), format!("expected `@pattern` or `@config`, found `@{other}`"))),
        }
    }
}

fn parse_chain_config(input: ParseStream) -> syn::Result<ChainConfig> {
    let content;
    braced!(content in input);
    let schema_group: proc_macro2::Group = content.parse()?;
    let schema = parse_schema_group(schema_group)?;
    content.parse::<Token![;]>()?;
    content.parse::<Token![@]>()?;
    let kw: Ident = content.parse()?;
    if kw != "pattern" {
        return Err(syn::Error::new(kw.span(), "expected `@pattern` inside @config"));
    }
    let pcontent;
    braced!(pcontent in content);
    let pattern_path = pcontent.parse()?;
    content.parse::<Token![;]>()?;
    let store: Ident = content.parse()?;
    let bindings = parse_bindings(&content)?;
    content.parse::<Token![;]>()?;
    content.parse::<Token![@]>()?;
    let kw: Ident = content.parse()?;
    if kw != "roots" {
        return Err(syn::Error::new(kw.span(), "expected `@roots` inside @config"));
    }
    let rcontent;
    bracketed!(rcontent in content);
    let mut roots = Vec::new();
    while !rcontent.is_empty() {
        roots.push(rcontent.parse()?);
        if rcontent.peek(Token![,]) {
            rcontent.parse::<Token![,]>()?;
        }
    }
    content.parse::<Token![;]>()?;
    content.parse::<Token![@]>()?;
    let kw: Ident = content.parse()?;
    if kw != "meta" {
        return Err(syn::Error::new(kw.span(), "expected `@meta` inside @config"));
    }
    let mcontent;
    bracketed!(mcontent in content);
    let mut metas = Vec::new();
    while !mcontent.is_empty() {
        let g: proc_macro2::Group = mcontent.parse()?;
        metas.push(syn::parse2(g.stream())?);
        if mcontent.peek(Token![,]) {
            mcontent.parse::<Token![,]>()?;
        }
    }
    Ok(ChainConfig { schema, pattern_path, store, bindings, roots, metas })
}

/// The chain state's *content*, flat: `{ ("pkg", { schema }) } ; @pattern
/// { P } ; Store { bindings } ; @roots [ .. ] ; @meta [ .. ]`. Callers wrap
/// it in the one delimiter group the courier grammar requires
/// (`@config { .. }`).
fn chain_config_tokens(config: &ChainConfig) -> TokenStream2 {
    let schema_ts = grammar::gen_schema_tokens(&config.schema);
    let mp = path_tokens(&config.pattern_path);
    let store = &config.store;
    let bindings_ts = grammar::bindings_tokens(&config.bindings);
    let roots = config.roots.iter().map(path_tokens);
    // Each collected tuple rides as its own brace group inside @meta[..].
    let metas = config.metas.iter().map(|m| {
        let t = root_meta_tokens(m);
        quote!({ #t })
    });
    quote! {
        { #schema_ts } ; @pattern { #mp } ; #store { #bindings_ts } ; @roots [ #(#roots),* ] ; @meta [ #(#metas),* ]
    }
}

#[doc(hidden)]
#[proc_macro]
pub fn __implement_gen(input: TokenStream) -> TokenStream {
    let gen = match syn::parse::<GenInput>(input.clone()) {
        Ok(g) => g,
        Err(e) => {
            if std::env::var("MG_DEBUG").is_ok() {
                eprintln!("__implement_gen parse error: {e}\ninput: {input}");
            }
            return e.to_compile_error().into();
        }
    };
    match expand_gen(gen) {
        Ok(ts) => {
            if std::env::var("MG_DEBUG").is_ok() {
                eprintln!("__implement_gen emits: {ts}");
            }
            ts.into()
        }
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_gen(gen: GenInput) -> syn::Result<TokenStream2> {
    match gen {
        GenInput::Stage1 { schema, pattern_path, store, bindings } => {
            expand_stage1(schema, pattern_path, store, bindings)
        }
        GenInput::ChainStep { meta, mut config } => {
            config.metas.push(meta);
            advance_chain(config)
        }
    }
}

/// Resolution start: validate what the schema alone can validate (bindings
/// still carry the user's spans here), queue the unique root patterns, and
/// emit the first registration-courier hop.
fn expand_stage1(
    schema: grammar::GenSchema,
    pattern_path: Path,
    store: Ident,
    bindings: Vec<Binding>,
) -> syn::Result<TokenStream2> {
    match &schema {
        grammar::GenSchema::Flat(_) => stage1_flat(schema, pattern_path, store, bindings),
        grammar::GenSchema::Composed(c) => stage1_composed(c, pattern_path, store, bindings),
    }
}

/// Flat branch of resolution start.
fn stage1_flat(
    schema: grammar::GenSchema,
    pattern_path: Path,
    store: Ident,
    bindings: Vec<Binding>,
) -> syn::Result<TokenStream2> {
    let schema = match schema {
        grammar::GenSchema::Flat(p) => p,
        grammar::GenSchema::Composed(_) => unreachable!("dispatched above"),
    };
    let body = &schema.body;
    if let Some(e) = body.entities.iter().find(|e| e.open) {
        // Spans have degraded to the call site by now; quote the tokens.
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "pattern `{}` declares open entity `{}`; open entities are closed by `compose_pattern!`, not `implement!`",
                pattern_name(&pattern_path),
                e.name
            ),
        ));
    }
    if let Some(a) = body.arrows.iter().find(|a| a.inst.is_none()) {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "arrow `{}: {} -> {}` is a root pattern's own declaration; root patterns are instantiated by other patterns, not implemented directly",
                a.name, a.source, a.target
            ),
        ));
    }

    // Binding checks that need no root metadata: unknown arrow, duplicate,
    // sum-arrow-given-binding, qualified key against a flat pattern. These
    // run here, while the binding idents still carry the `implement!` call
    // site's spans. Duplicate/unknown compare *qualified* keys.
    let mut seen: Vec<String> = Vec::new();
    for b in &bindings {
        let key = b.key_str();
        if seen.contains(&key) {
            return Err(syn::Error::new(
                b.arrow.span(),
                format!("duplicate binding for arrow `{key}`"),
            ));
        }
        seen.push(key);
        if let Some(inst) = &b.instance {
            return Err(syn::Error::new(
                b.arrow.span(),
                format!(
                    "binding `{inst}.{}` is qualified, but pattern `{}` is not composed; use the bare arrow name",
                    b.arrow,
                    pattern_name(&pattern_path)
                ),
            ));
        }
        match body.arrows.iter().find(|a| a.name == b.arrow) {
            None => {
                return Err(syn::Error::new(
                    b.arrow.span(),
                    format!(
                        "unknown arrow `{}` in `implement!` bindings (not declared by pattern `{}`)",
                        b.arrow,
                        pattern_name(&pattern_path)
                    ),
                ));
            }
            Some(a) if a.sum.is_some() => {
                let (ua, ub) = a.sum.as_ref().unwrap();
                return Err(syn::Error::new(
                    b.arrow.span(),
                    format!(
                        "derived arrow `{}` (`:= sum({}, {})`) is never bound to a backing",
                        b.arrow, ua, ub
                    ),
                ));
            }
            _ => {}
        }
    }

    // Queue the unique instantiation roots, in first-occurrence order,
    // resolved to their absolute paths by the declaring-crate resolver (the
    // resolution also fails here, with a clean message, when the implementing
    // crate lacks a direct dependency on every member's declaring crate).
    let roots = resolved_roots(&schema)?;

    let config = ChainConfig {
        schema: grammar::GenSchema::Flat(schema),
        pattern_path,
        store,
        bindings,
        roots,
        metas: Vec::new(),
    };
    advance_chain(config)
}

/// Composed branch of resolution start: bindings key by `instance.arrow`,
/// and the root queue spans every member's arrows, each resolved against
/// that member's declaring package (implement-time root-path resolution is
/// keyed by member instance).
fn stage1_composed(
    c: &grammar::ComposedSchema,
    pattern_path: Path,
    store: Ident,
    bindings: Vec<Binding>,
) -> syn::Result<TokenStream2> {
    let member_of = |inst: &str| c.members.iter().find(|(i, _, _)| i == inst);
    for b in &bindings {
        let key = b.key_str();
        let Some(inst) = &b.instance else {
            return Err(syn::Error::new(
                b.arrow.span(),
                format!(
                    "binding `{}` is bare, but pattern `{}` is composed; key every backing as `instance.arrow`",
                    b.arrow,
                    pattern_name(&pattern_path)
                ),
            ));
        };
        let Some((_, _, body)) = member_of(&inst.to_string()) else {
            return Err(syn::Error::new(
                b.arrow.span(),
                format!(
                    "binding `{key}` names unknown instance `{inst}` (pattern `{}` instantiates: {})",
                    pattern_name(&pattern_path),
                    c.instances.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
                ),
            ));
        };
        match body.arrows.iter().find(|a| a.name == b.arrow) {
            None => {
                return Err(syn::Error::new(
                    b.arrow.span(),
                    format!("unknown arrow `{key}` in `implement!` bindings (not declared by instance `{inst}`)"),
                ));
            }
            Some(a) if a.sum.is_some() => {
                let (ua, ub) = a.sum.as_ref().unwrap();
                return Err(syn::Error::new(
                    b.arrow.span(),
                    format!(
                        "derived arrow `{key}` (`:= sum({}, {})`) is never bound to a backing",
                        ua, ub
                    ),
                ));
            }
            _ => {}
        }
    }
    // Duplicate qualified keys.
    let mut seen: Vec<String> = Vec::new();
    for b in &bindings {
        let key = b.key_str();
        if seen.contains(&key) {
            return Err(syn::Error::new(
                b.arrow.span(),
                format!("duplicate binding for arrow `{key}`"),
            ));
        }
        seen.push(key);
    }

    // Queue the unique instantiation roots across all members, resolved per
    // member package.
    let mut roots: Vec<Path> = Vec::new();
    for (_, pkg, body) in &c.members {
        for a in &body.arrows {
            let p = a.inst.as_ref().expect("members are instantiation patterns");
            let ts = resolve_schema_path(p, pkg)?;
            let key = ts.to_string();
            if !roots.iter().any(|r| path_tokens(r).to_string() == key) {
                roots.push(syn::parse2(ts).expect("resolver emits a parseable path"));
            }
        }
    }

    let config = ChainConfig {
        schema: grammar::GenSchema::Composed(c.clone()),
        pattern_path,
        store,
        bindings,
        roots,
        metas: Vec::new(),
    };
    advance_chain(config)
}

/// Queue state machine: either bounce through the next root's registration
/// courier, or — queue empty — run codegen with the collected tuples.
fn advance_chain(config: ChainConfig) -> syn::Result<TokenStream2> {
    if config.roots.is_empty() {
        return codegen::expand(&config);
    }
    let mut config = config;
    let next = config.roots.remove(0);
    emit_hop(&next, config)
}

fn emit_hop(root: &Path, config: ChainConfig) -> syn::Result<TokenStream2> {
    // `root` is already resolver-absolute (stage1 resolved the queue), so the
    // registration courier's `pub use` prefix needs no further resolution —
    // it is the module holding the root `pattern!` invocation.
    let last = root.segments.last().unwrap();
    if !matches!(last.arguments, syn::PathArguments::None) {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("root path `{}` must end in a plain ident", path_tokens(root)),
        ));
    }
    let mut prefix = root.clone();
    prefix.segments.pop();
    if prefix.segments.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "root path `{}` is a bare name; roots are addressed by module-qualified paths (e.g. `stdlib::{}`)",
                path_tokens(root),
                last.ident
            ),
        ));
    }
    let courier = format_ident!("__metagraph_pattern_{}", last.ident);
    let prefixed = path_tokens(&prefix);
    let rt = runtime_target();
    let cfg = chain_config_tokens(&config);
    Ok(quote! {
        #prefixed::#courier ! {
            (#rt::__implement_gen ; @config { #cfg })
        }
    })
}

fn pattern_name(p: &Path) -> String {
    p.segments.last().map(|s| s.ident.to_string()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Path resolution
// ---------------------------------------------------------------------------

/// Absolute path of the runtime crate, chosen at invocation time with
/// `proc-macro-crate` (plus the dependency-rename it honors). `Itself`
/// resolves through the runtime root's `extern crate self as
/// metagraph_exper`.
pub(crate) fn runtime_path() -> TokenStream2 {
    match proc_macro_crate::crate_name("metagraph-exper") {
        Ok(proc_macro_crate::FoundCrate::Itself) => quote!(::metagraph_exper),
        Ok(proc_macro_crate::FoundCrate::Name(n)) => {
            let n = Ident::new(&n, proc_macro2::Span::call_site());
            quote!(::#n)
        }
        Err(_) => quote!(::metagraph_exper),
    }
}

/// The runtime path as a *courier forwarding target*: same as
/// [`runtime_path`] minus the leading `::`. Courier transcribers match the
/// target as `$($fwd:tt)::+` (the spike-pinned form — a `path` fragment
/// cannot be followed by `!`), and a leading colon is not a token that
/// matcher accepts. Macro paths need the colons less than item paths do.
pub(crate) fn runtime_target() -> TokenStream2 {
    let tts: Vec<_> = runtime_path().into_iter().collect();
    let starts_colons = matches!(
        tts.first(),
        Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == ':'
    );
    if starts_colons {
        tts.into_iter().skip(2).collect()
    } else {
        tts.into_iter().collect()
    }
}

// ---------------------------------------------------------------------------
// The declaring-crate resolver (plans/milestone-2, Open entities)
// ---------------------------------------------------------------------------

/// Absolute root of the crate that declared a schema, as seen from the crate
/// being compiled right now (the implement/compose site). `Itself` →
/// `crate`; a renamed dependency → its local name; no direct dependency → a
/// clean error naming the declaring package — the resolver's one constraint.
pub(crate) fn declaring_crate_root(pkg: &str) -> syn::Result<TokenStream2> {
    // The one package whose `Itself` must NOT spell `crate` is the runtime
    // crate itself: its doctests are separate crates where `crate::stdlib`
    // misses, while `::metagraph_exper` resolves everywhere the guaranteed
    // `extern crate self` alias does (in-crate, doctests, and — via
    // [`runtime_path`] — through a rename).
    if pkg == "metagraph-exper" {
        return Ok(runtime_path());
    }
    match proc_macro_crate::crate_name(pkg) {
        Ok(proc_macro_crate::FoundCrate::Itself) => Ok(quote!(crate)),
        Ok(proc_macro_crate::FoundCrate::Name(n)) => {
            let n = Ident::new(&n, proc_macro2::Span::call_site());
            Ok(quote!(::#n))
        }
        Err(e) => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "this pattern was declared by crate `{pkg}`, which the implementing crate does not depend on directly ({e}); add the dependency (a rename is fine) to implement it here"
            ),
        )),
    }
}

/// Resolve an instantiation-source path from the schema DSL *against the
/// declaring crate* (whose package name rides beside the schema): the path
/// was written there, so it means what it means there. Leading-`::` paths
/// pass verbatim; `crate::`/`self::`-rooted paths swap their head for the
/// declaring crate's local root; anything else (`stdlib::Nodes`) is prefixed
/// with it. A bare single-segment path is rejected — the courier it would
/// name lives in a module, not at the declaring crate's root.
pub(crate) fn resolve_schema_path(p: &Path, pkg: &str) -> syn::Result<TokenStream2> {
    if p.leading_colon.is_some() {
        return Ok(path_tokens(p));
    }
    let first = p.segments.first().map(|s| s.ident.to_string()).unwrap_or_default();
    let root = declaring_crate_root(pkg)?;
    if first == "crate" || first == "self" {
        let rest: Vec<&Ident> = p.segments.iter().skip(1).map(|s| &s.ident).collect();
        return Ok(quote!(#root:: #(#rest)::*));
    }
    if p.segments.len() < 2 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "instantiation source `{}` (in a pattern declared by crate `{pkg}`) is a bare name; roots are addressed by module-qualified paths (e.g. `crate::roots::{}`)",
                path_tokens(p),
                p.segments.last().map(|s| s.ident.to_string()).unwrap_or_default()
            ),
        ));
    }
    let segs: Vec<&Ident> = p.segments.iter().map(|s| &s.ident).collect();
    Ok(quote!(#root:: #(#segs)::*))
}

/// Unique instantiation roots of a packaged schema, in first-occurrence
/// order over its arrows — the order `metas[i]` is keyed by — each resolved
/// to its absolute path (dedup is by *resolved* spelling, so `stdlib::Nodes`
/// and `::metagraph_exper::stdlib::Nodes` in one schema visit one hop).
pub(crate) fn resolved_roots(schema: &grammar::PackagedSchema) -> syn::Result<Vec<Path>> {
    let mut roots: Vec<Path> = Vec::new();
    for a in &schema.body.arrows {
        let p = a.inst.as_ref().unwrap();
        let ts = resolve_schema_path(p, &schema.pkg)?;
        let key = ts.to_string();
        if !roots.iter().any(|r| path_tokens(r).to_string() == key) {
            roots.push(syn::parse2(ts).expect("resolver emits a parseable path"));
        }
    }
    Ok(roots)
}



