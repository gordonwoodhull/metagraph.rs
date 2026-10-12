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
mod grammar;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{braced, bracketed, Ident, Path, Token};

use grammar::{
    parse_bindings, path_tokens, root_meta_tokens, schema_body_tokens, snake, Binding,
    Cardinality, RootMeta, SchemaBody,
};

// ---------------------------------------------------------------------------
// pattern!
// ---------------------------------------------------------------------------

struct PatternInput {
    name: Ident,
    body: SchemaBody,
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
            (($fwd:path ; $($rest:tt)*)) => {
                $fwd ! { { #meta_ts } ; $($rest)* }
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

    let schema_tokens = schema_body_tokens(&pat.body);

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
            (($fwd:path ; $($rest:tt)*)) => {
                $fwd ! { { #schema_tokens } ; $($rest)* }
            };
        }
        #[doc(hidden)]
        #[allow(unused_imports)]
        pub use #courier;
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
    let rt = runtime_path();
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
/// (in first-occurrence order over the schema's arrows); `metas` the tuples
/// collected so far — `metas[i]` belongs to the i-th root of that order.
pub(crate) struct ChainConfig {
    pub(crate) schema: SchemaBody,
    /// The pattern path as written in `implement!` (prefix + pattern name).
    pub(crate) pattern_path: Path,
    pub(crate) store: Ident,
    pub(crate) bindings: Vec<Binding>,
    pub(crate) roots: Vec<Path>,
    pub(crate) metas: Vec<RootMeta>,
}

enum GenInput {
    /// `{ schema } ; @pattern { P } ; Store { bindings }` — fresh from the
    /// schema courier: parse, validate, start the resolution chain.
    Stage1 { schema: SchemaBody, pattern_path: Path, store: Ident, bindings: Vec<Binding> },
    /// `{ meta tuple } ; @config { ... }` — one registration courier hop:
    /// the tuple just arrived; the config carries the remaining queue.
    ChainStep { meta: RootMeta, config: ChainConfig },
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
                let schema = syn::parse2(first.stream())?;
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
    let schema = syn::parse2(schema_group.stream())?;
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

/// The chain state's *content*, flat: `{ schema } ; @pattern { P } ;
/// Store { bindings } ; @roots [ .. ] ; @meta [ .. ]`. Callers wrap it in
/// the one delimiter group the courier grammar requires (`@config { .. }`).
fn chain_config_tokens(config: &ChainConfig) -> TokenStream2 {
    let schema_ts = schema_body_tokens(&config.schema);
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
    schema: SchemaBody,
    pattern_path: Path,
    store: Ident,
    bindings: Vec<Binding>,
) -> syn::Result<TokenStream2> {
    if let Some(e) = schema.entities.iter().find(|e| e.open) {
        // Spans have degraded to the call site by now; quote the tokens.
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "pattern `{}` declares open entity `{}`; open entities are closed by `compose_pattern!` (M2), not `implement!`",
                pattern_name(&pattern_path),
                e.name
            ),
        ));
    }
    if let Some(a) = schema.arrows.iter().find(|a| a.inst.is_none()) {
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
        match schema.arrows.iter().find(|a| a.name == b.arrow) {
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

    // Queue the unique instantiation roots, in first-occurrence order.
    let mut roots: Vec<Path> = Vec::new();
    for a in &schema.arrows {
        let p = a.inst.as_ref().unwrap();
        let key = path_tokens(p).to_string();
        if !roots.iter().any(|r| path_tokens(r).to_string() == key) {
            roots.push(p.clone());
        }
    }

    let config = ChainConfig { schema, pattern_path, store, bindings, roots, metas: Vec::new() };
    advance_chain(config)
}

/// Queue state machine: either bounce through the next root's registration
/// courier, or — queue empty — run codegen with the collected tuples.
fn advance_chain(mut config: ChainConfig) -> syn::Result<TokenStream2> {
    match config.roots.is_empty() {
        true => codegen::expand_flat(&config),
        false => {
            let next = config.roots.remove(0);
            emit_hop(&next, config)
        }
    }
}

fn emit_hop(root: &Path, config: ChainConfig) -> syn::Result<TokenStream2> {
    let last = root.segments.last().unwrap();
    if !matches!(last.arguments, syn::PathArguments::None) {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("root path `{}` must end in a plain ident", path_tokens(root)),
        ));
    }
    // The registration courier's `pub use` landed in the module holding the
    // root `pattern!` invocation: the root path's prefix, without the root
    // pattern's own name. (The trait itself lives at the full path.)
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
    let prefixed = resolve_schema_path(&prefix);
    let rt = runtime_path();
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

/// Resolve an instantiation source path from the schema DSL against the
/// `implement!` invocation site: `crate`-rooted and leading-`::` paths pass
/// through verbatim (declaring-crate-local and explicit-absolute spellings);
/// anything else (`stdlib::Nodes`) is prefixed with the runtime crate root.
pub(crate) fn resolve_schema_path(p: &Path) -> TokenStream2 {
    let crate_rooted = p
        .segments
        .first()
        .map(|s| s.ident == "crate" || s.ident == "self")
        .unwrap_or(false);
    if crate_rooted || p.leading_colon.is_some() {
        return path_tokens(p);
    }
    let rt = runtime_path();
    let pt = path_tokens(p);
    quote!(#rt::#pt)
}

/// Unique instantiation roots of a schema, in first-occurrence order — the
/// order `metas[i]` is keyed by.
pub(crate) fn unique_roots(schema: &SchemaBody) -> Vec<Path> {
    let mut roots: Vec<Path> = Vec::new();
    for a in &schema.arrows {
        if let Some(p) = &a.inst {
            let key = path_tokens(p).to_string();
            if !roots.iter().any(|r| path_tokens(r).to_string() == key) {
                roots.push(p.clone());
            }
        }
    }
    roots
}



