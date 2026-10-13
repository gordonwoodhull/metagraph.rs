//! Shared schema grammar: what `pattern!` parses and re-emits into its
//! couriers, and what `__implement_gen!` parses back out. One module, both
//! directions, with round-trip tests — any drift between what `pattern!`
//! embeds and what `__implement_gen!` parses fails inside generated code,
//! where it is undebuggable from user-facing error messages
//! (plans/milestone-1, shared-grammar commitment).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{braced, Ident, Path, Token};

// ---------------------------------------------------------------------------
// Naming derivations (plans/milestone-1, Naming conventions)
// ---------------------------------------------------------------------------

/// `CamelCase` -> `camel_case`.
pub fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `snake_case` -> `CamelCase`.
pub fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for c in name.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

pub fn id_ty(e: &Ident) -> Ident {
    format_ident!("{}Id", e)
}

pub fn data_ty(e: &Ident) -> Ident {
    format_ident!("{}Data", e)
}

/// Iterator type of an arrow: `out_edges` -> `OutEdgesIter`.
pub fn iter_ty(a: &Ident) -> Ident {
    format_ident!("{}Iter", camel(&a.to_string()))
}

/// Marker type of an arrow instantiation: `out_edges` -> `OutEdges`.
pub fn marker_ty(a: &Ident) -> Ident {
    format_ident!("{}", camel(&a.to_string()))
}

/// Head/link field stem: `first_out`/`next_out` for arrow `out_edges`
/// (a trailing `_edges` is stripped — the one spelling the plan pins).
pub fn link_stem(a: &Ident) -> Ident {
    let s = a.to_string();
    let stem = s.strip_suffix("_edges").unwrap_or(&s);
    format_ident!("{}", stem)
}

pub fn arena_fld(e: &Ident) -> Ident {
    format_ident!("{}_arena", snake(&e.to_string()))
}

/// Join a path's idents with `::` (syn's own `ToTokens` for `Path` emits a
/// trailing colon on some versions; re-emit by hand).
pub fn path_tokens(p: &Path) -> TokenStream {
    let segs: Vec<&Ident> = p.segments.iter().map(|s| &s.ident).collect();
    let lead = if p.leading_colon.is_some() {
        quote!(::)
    } else {
        TokenStream::new()
    };
    quote!(#lead #(#segs)::*)
}

// ---------------------------------------------------------------------------
// Schema grammar
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct EntityDecl {
    pub name: Ident,
    /// `root E` — the pattern's root-marked entity: its record is the store
    /// itself, so it owns no id, record, or arena — only its handle and the
    /// store's root accessor. Exactly one entity per pattern carries this.
    pub root: bool,
    /// `E(open)` — declared but not owned; closed by reference fusion at
    /// composition time (M2). Parsed here so the marking survives the
    /// courier channel; M1 rejects open entities at `implement!` time.
    pub open: bool,
}

impl Parse for EntityDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut root = false;
        {
            let fork = input.fork();
            if let Ok(kw) = fork.parse::<Ident>() {
                if kw == "root" {
                    input.parse::<Ident>()?; // consume the `root` keyword
                    root = true;
                }
            }
        }
        let name: Ident = input.parse()?;
        let open = if input.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);
            let kw: Ident = content.parse()?;
            if kw != "open" {
                return Err(syn::Error::new(kw.span(), "only `(open)` is supported"));
            }
            true
        } else {
            false
        };
        if root && open {
            return Err(syn::Error::new(
                name.span(),
                "an entity cannot be both `root` and `(open)`",
            ));
        }
        Ok(EntityDecl { name, root, open })
    }
}

#[derive(Clone)]
pub struct ArrowDecl {
    /// Local arrow name: an alias when instantiating a root pattern. Read-side
    /// trait identity (method, GAT, associated types) comes from the root's
    /// declaration; write-side op names (`add_*`, `insert_*`) derive from this
    /// local name.
    pub name: Ident,
    /// Instantiation source path (e.g. `stdlib::OutEdges`); `None` on the
    /// single arrow of a root pattern. Resolved at the `implement!` site:
    /// `crate`-rooted and leading-`::` paths pass through verbatim, anything
    /// else is prefixed with the runtime crate root.
    pub inst: Option<Path>,
    pub source: Ident,
    pub target: Ident,
    /// `Some(true)` = many, `Some(false)` = one. Declared on root arrows
    /// only; instantiations inherit the root's cardinality.
    pub many: Option<bool>,
    /// `by <arrow>` — derived membership: the named total `one` arrow (which
    /// runs opposite: target-of-`by` -> source-of-`by`) determines membership.
    pub by: Option<Ident>,
    /// `key(K)` on a *root* arrow: declares the keyed partial cardinality (K
    /// is the root's own key entity, rebound per instantiation). On an
    /// *instantiation*: binds the key sort to a local entity (M2).
    pub key: Option<Ident>,
    /// `:= sum(a, b)` — derived read over two sibling `many` arrows.
    pub sum: Option<(Ident, Ident)>,
}

impl Parse for ArrowDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        // Optional instantiation path followed by a comma. A leading `::`
        // (explicit-absolute spelling, e.g. naming the runtime crate from a
        // fixture crate) is a path-sep token, not an ident.
        let inst = if (input.peek(Ident) && input.peek2(Token![::]))
            || (input.peek(Token![crate]) && input.peek2(Token![::]))
            || (input.peek(Ident) && input.peek2(Token![,]))
            || input.peek(Token![::])
        {
            let p: Path = input.parse()?;
            input.parse::<Token![,]>()?;
            Some(p)
        } else {
            None
        };
        let source: Ident = input.parse()?;
        input.parse::<Token![->]>()?;
        let target: Ident = input.parse()?;
        let mut many = None;
        let mut by = None;
        let mut key = None;
        let mut sum = None;
        loop {
            if input.peek(Token![;]) {
                input.parse::<Token![;]>()?;
                break;
            }
            if input.peek(Token![:]) && input.peek2(Token![=]) {
                // `:= sum(a, b)` — terminates the declaration.
                input.parse::<Token![:]>()?;
                input.parse::<Token![=]>()?;
                let kw: Ident = input.parse()?;
                if kw != "sum" {
                    return Err(
                        syn::Error::new(kw.span(), "only `sum` derivation is supported")
                    );
                }
                let content;
                syn::parenthesized!(content in input);
                let a: Ident = content.parse()?;
                content.parse::<Token![,]>()?;
                let b: Ident = content.parse()?;
                input.parse::<Token![;]>()?;
                sum = Some((a, b));
                break;
            }
            input.parse::<Token![,]>()?;
            if input.peek(Token![;]) {
                input.parse::<Token![;]>()?;
                break;
            }
            let kw: Ident = input.parse()?;
            match kw.to_string().as_str() {
                "one" => many = Some(false),
                "many" => many = Some(true),
                "by" => {
                    let det: Ident = input.parse()?;
                    by = Some(det);
                }
                "key" => {
                    let content;
                    syn::parenthesized!(content in input);
                    let k: Ident = content.parse()?;
                    key = Some(k);
                }
                other => {
                    return Err(syn::Error::new(
                        kw.span(),
                        format!(
                            "unexpected `{other}` in arrow declaration (expected `one`, `many`, `by`, `key(K)`, or `:= sum(..)`)"
                        ),
                    ));
                }
            }
        }
        Ok(ArrowDecl { name, inst, source, target, many, by, key, sum })
    }
}

#[derive(Clone)]
pub struct SchemaBody {
    pub entities: Vec<EntityDecl>,
    pub arrows: Vec<ArrowDecl>,
}

impl SchemaBody {
    /// The write-side constructor-argument arrows of a non-root entity, in
    /// arrow-declaration order, or `None` when the declaring block cannot
    /// prove the entity's total-one arity (M4's settled by-derived-only
    /// rule). An arg is an inline-declared `one` arrow or an arrow named as
    /// a sibling's `by` determinant (`by` requires a total `one`
    /// determinant — the implement!-time bound-one check enforces it, so
    /// the derived arity cannot be silently wrong). Any other instantiation
    /// arrow leaving the entity is opaque — its cardinality is root-side
    /// data the declaring block cannot resolve at expansion time — and
    /// vetoes the entity's `Add` trait (the verb stays Tx-inherent).
    pub fn entity_ctor_args(&self, ent: &Ident) -> Option<Vec<Ident>> {
        let mut args = Vec::new();
        for a in &self.arrows {
            if a.source != *ent {
                continue;
            }
            let provable_one = a.many == Some(false)
                || self
                    .arrows
                    .iter()
                    .any(|x| x.by.as_ref().map(|d| d == &a.name).unwrap_or(false));
            if provable_one {
                args.push(a.name.clone());
            } else if a.by.is_some() || a.sum.is_some() || a.key.is_some() {
                // A derived or keyed arrow is not a one-arrow candidate.
            } else {
                return None;
            }
        }
        Some(args)
    }

    /// The closed non-root entities — the ones the write side emits
    /// per-entity markers and `Add`/`Remove` traits for. Open entities are
    /// skipped: their records live in a fused peer store, so existence ops
    /// have nothing to name (a composed `Tx` sees them through their host
    /// entity when nested stores land).
    pub fn closed_elements(&self) -> impl Iterator<Item = &EntityDecl> {
        self.entities.iter().filter(|e| !e.root && !e.open)
    }
}

impl Parse for SchemaBody {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let kw: Ident = input.parse()?;
        if kw != "entities" {
            return Err(syn::Error::new(kw.span(), "expected `entities`"));
        }
        input.parse::<Token![:]>()?;
        let mut entities = Vec::new();
        loop {
            entities.push(input.parse()?);
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
                if input.peek(Token![;]) {
                    break;
                }
            } else {
                break;
            }
        }
        input.parse::<Token![;]>()?;
        let kw: Ident = input.parse()?;
        if kw != "arrows" {
            return Err(syn::Error::new(kw.span(), "expected `arrows`"));
        }
        if input.peek(Token![:]) {
            input.parse::<Token![:]>()?;
        }
        let content;
        braced!(content in input);
        let mut arrows = Vec::new();
        while !content.is_empty() {
            arrows.push(content.parse()?);
        }
        Ok(SchemaBody { entities, arrows })
    }
}

/// Re-emit a parsed schema in the shared courier grammar (round-trips through
/// `SchemaBody`'s parser). Used by `pattern!`'s schema courier.
pub fn schema_body_tokens(body: &SchemaBody) -> TokenStream {
    let entities = body.entities.iter().map(|e| {
        let n = &e.name;
        let root_ts = if e.root { quote!(root) } else { quote!() };
        let open_ts = if e.open { quote!((open)) } else { quote!() };
        quote!(#root_ts #n #open_ts)
    });
    let arrows = body.arrows.iter().map(|a| {
        let n = &a.name;
        let s = &a.source;
        let t = &a.target;
        let inst_ts = a.inst.as_ref().map(|p| {
            let pt = path_tokens(p);
            quote!(#pt,)
        }).unwrap_or_default();
        let card_ts = match a.many {
            Some(true) => quote!(, many),
            Some(false) => quote!(, one),
            None => quote!(),
        };
        let by_ts = a
            .by
            .as_ref()
            .map(|x| quote!(, by #x))
            .unwrap_or_default();
        let key_ts = a
            .key
            .as_ref()
            .map(|k| quote!(, key(#k)))
            .unwrap_or_default();
        let sum_ts = a
            .sum
            .as_ref()
            .map(|(x, y)| quote!(:= sum(#x, #y)))
            .unwrap_or_default();
        quote!(#n: #inst_ts #s -> #t #card_ts #by_ts #key_ts #sum_ts ;)
    });
    quote! {
        entities: #(#entities),* ; arrows: { #(#arrows)* }
    }
}

// ---------------------------------------------------------------------------
// Packaged schema (the declaring-crate resolver's carriage)
// ---------------------------------------------------------------------------

/// A schema riding with its declaring package: `("pkg-name", { body })`.
/// `pattern!` reads `CARGO_PKG_NAME` at its expansion site (the declaring
/// crate's rustc process — the probe at /tmp/m2-probe-root-resolver pinned
/// that the env var names the crate being compiled, not the eventual
/// consumer) and bakes it here, so every instantiation-source path inside
/// the schema can be resolved against the *declaring* crate at the
/// `implement!`/`compose_pattern!` site: `Itself` → `crate::…`, a rename →
/// the local dependency name, a missing direct dependency → a clean error.
/// Spike-pinned constraint: the schema body crosses courier hops as one
/// group token tree and is never re-tokenized in flight.
#[derive(Clone)]
pub struct PackagedSchema {
    pub pkg: String,
    pub body: SchemaBody,
}

impl Parse for PackagedSchema {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        syn::parenthesized!(content in input);
        let pkg: syn::LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        let body;
        braced!(body in content);
        Ok(PackagedSchema { pkg: pkg.value(), body: body.parse()? })
    }
}

/// Re-emit a packaged schema (round-trips through `PackagedSchema::parse`).
pub fn packaged_schema_tokens(p: &PackagedSchema) -> TokenStream {
    let pkg = &p.pkg;
    let body = schema_body_tokens(&p.body);
    quote!((#pkg, { #body }))
}

#[cfg(test)]
mod packaged_tests {
    use super::*;

    #[test]
    fn packaged_schema_roundtrip() {
        let src = r#"("fixture-crate", { entities: root T, SubNode, MainNode(open); arrows { a: crate::my_roots::MyRoot, SubNode -> MainNode; } })"#;
        let ts: TokenStream = src.parse().expect("src parses");
        let once: PackagedSchema = syn::parse2(ts).expect("parse");
        assert_eq!(once.pkg, "fixture-crate");
        let emitted = packaged_schema_tokens(&once);
        let twice: PackagedSchema = syn::parse2(emitted.clone()).expect("reparse");
        assert_eq!(twice.pkg, "fixture-crate");
        assert_eq!(
            once.body.entities.len(),
            twice.body.entities.len(),
            "emitted: {emitted}"
        );
        assert_eq!(
            emitted.to_string(),
            packaged_schema_tokens(&twice).to_string(),
            "idempotent emission"
        );
    }
}

// ---------------------------------------------------------------------------
// Composition: fusions and the composed schema
// ---------------------------------------------------------------------------

/// One entity-fusion declaration: `inst.Entity <op> inst.Entity ;`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FusionOp {
    /// `<=>` colocation fusion: symmetric, transitive; merges records.
    Coloc,
    /// `<->` reference fusion: unifies id types only.
    Ref,
}

#[derive(Clone, Debug)]
pub struct Fusion {
    pub lhs: (Ident, Ident),
    pub op: FusionOp,
    pub rhs: (Ident, Ident),
}

impl Parse for Fusion {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let li: Ident = input.parse()?;
        input.parse::<Token![.]>()?;
        let le: Ident = input.parse()?;
        input.parse::<Token![<]>()?;
        let op = if input.peek(Token![-]) {
            input.parse::<Token![-]>()?;
            input.parse::<Token![>]>()?;
            FusionOp::Ref
        } else {
            input.parse::<Token![=]>()?;
            input.parse::<Token![>]>()?;
            FusionOp::Coloc
        };
        let ri: Ident = input.parse()?;
        input.parse::<Token![.]>()?;
        let re: Ident = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(Fusion { lhs: (li, le), op, rhs: (ri, re) })
    }
}

impl Fusion {
    pub fn tokens(&self) -> TokenStream {
        let (li, le) = &self.lhs;
        let (ri, re) = &self.rhs;
        let op = match self.op {
            FusionOp::Coloc => quote!(<=>),
            FusionOp::Ref => quote!(<->),
        };
        quote!(#li.#le #op #ri.#re ;)
    }
}

/// A composed pattern's schema, embedded in its courier under the same
/// packaging wrapper as a flat one: `("composing-pkg", { instances: ..;
/// members: ..; fusions: { .. } })`. Each member schema rides with its own
/// declaring package as `("pkg", { schema })` inside the members list. The
/// composed pattern's own package matters only for a composition-of-a-
/// composed (frontier, untested) — member root resolution is keyed by the
/// *member* packages.
#[derive(Clone)]
pub struct ComposedSchema {
    /// Instance names, in chain (declaration) order.
    pub instances: Vec<Ident>,
    /// Per instance: (instance, declaring package, member schema).
    pub members: Vec<(Ident, String, SchemaBody)>,
    pub fusions: Vec<Fusion>,
    /// The package of the crate where `compose_pattern!` ran.
    pub pkg: String,
}

impl Parse for ComposedSchema {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let wrapper = SchemaWrapper::parse_around(input)?;
        let c: ComposedInner = syn::parse2(wrapper.body.stream())?;
        Ok(ComposedSchema {
            instances: c.instances,
            members: c.members,
            fusions: c.fusions,
            pkg: wrapper.pkg,
        })
    }
}

struct ComposedInner {
    instances: Vec<Ident>,
    members: Vec<(Ident, String, SchemaBody)>,
    fusions: Vec<Fusion>,
}

impl Parse for ComposedInner {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let kw: Ident = input.parse()?;
        if kw != "instances" {
            return Err(syn::Error::new(kw.span(), "expected `instances`"));
        }
        input.parse::<Token![:]>()?;
        let mut instances = Vec::new();
        loop {
            instances.push(input.parse()?);
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else {
                break;
            }
        }
        input.parse::<Token![;]>()?;
        let kw: Ident = input.parse()?;
        if kw != "members" {
            return Err(syn::Error::new(kw.span(), "expected `members`"));
        }
        input.parse::<Token![:]>()?;
        let mut members = Vec::new();
        loop {
            // One paren group per member: ("declaring-pkg", inst { schema }).
            let content;
            syn::parenthesized!(content in input);
            let pkg: syn::LitStr = content.parse()?;
            content.parse::<Token![,]>()?;
            let inst: Ident = content.parse()?;
            let inner;
            braced!(inner in content);
            let schema: SchemaBody = inner.parse()?;
            members.push((inst, pkg.value(), schema));
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else {
                break;
            }
        }
        input.parse::<Token![;]>()?;
        let kw: Ident = input.parse()?;
        if kw != "fusions" {
            return Err(syn::Error::new(kw.span(), "expected `fusions`"));
        }
        input.parse::<Token![:]>()?;
        let content;
        braced!(content in input);
        let mut fusions = Vec::new();
        while !content.is_empty() {
            fusions.push(content.parse()?);
        }
        Ok(ComposedInner { instances, members, fusions })
    }
}

/// The packaging wrapper's spine: `("pkg", { .. })`, body kept raw.
struct SchemaWrapper {
    pkg: String,
    body: proc_macro2::Group,
}

impl SchemaWrapper {
    /// Parse the wrapper from a stream that starts with `("pkg", {`.
    fn parse_around(input: ParseStream) -> syn::Result<Self> {
        let content;
        syn::parenthesized!(content in input);
        let pkg: syn::LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        let body: proc_macro2::Group = content.parse()?;
        if !content.is_empty() {
            return Err(syn::Error::new(content.span(), "trailing tokens after the schema body"));
        }
        Ok(SchemaWrapper { pkg: pkg.value(), body })
    }
}

/// Re-emit a composed schema in its packaging wrapper (round-trips through
/// `ComposedSchema::parse`).
pub fn composed_schema_tokens(c: &ComposedSchema) -> TokenStream {
    let insts = &c.instances;
    let members = c.members.iter().map(|(inst, pkg, body)| {
        let s = schema_body_tokens(body);
        let p = pkg.as_str();
        quote!((#p, #inst { #s }))
    });
    let fusions = c.fusions.iter().map(Fusion::tokens);
    let pkg = c.pkg.as_str();
    quote! {
        (#pkg, { instances: #(#insts),* ; members: #(#members),* ; fusions: { #(#fusions)* } })
    }
}

/// What a pattern's courier forwards: a flat schema or a composed one, each
/// packaged with its declaring crate. Dispatch is by the body's first
/// keyword (`entities:` vs `instances:`) under the packaging wrapper.
pub enum GenSchema {
    Flat(PackagedSchema),
    Composed(ComposedSchema),
}

/// Parse the schema group riding a courier payload: `("pkg", { body })`,
/// where body is headed `entities:` (flat) or `instances:` (composed).
pub fn parse_gen_schema(group: proc_macro2::Group) -> syn::Result<GenSchema> {
    use syn::parse::Parser as _;
    let wrapper = SchemaWrapper::parse_around
        .parse2(group.stream())
        .map_err(|e| syn::Error::new(e.span(), format!("malformed packaged schema: {e}")))?;
    let first = wrapper.body.stream().into_iter().next();
    match first {
        Some(proc_macro2::TokenTree::Ident(kw)) if kw == "instances" => {
            Ok(GenSchema::Composed(syn::parse2(group.stream())?))
        }
        _ => Ok(GenSchema::Flat(syn::parse2(group.stream())?)),
    }
}

/// Re-emit either schema kind in its packaging wrapper.
pub fn gen_schema_tokens(schema: &GenSchema) -> TokenStream {
    match schema {
        GenSchema::Flat(p) => packaged_schema_tokens(p),
        GenSchema::Composed(c) => composed_schema_tokens(c),
    }
}

#[cfg(test)]
mod composed_tests {
    use super::*;

    fn composed_sig(c: &ComposedSchema) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "i:[{}];",
            c.instances.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
        ));
        for (inst, pkg, body) in &c.members {
            s.push_str(&format!(
                "m:{inst}/{pkg}/{}e/{}a;",
                body.entities.len(),
                body.arrows.len()
            ));
        }
        s.push_str(&format!("f:{};p:{};", c.fusions.len(), c.pkg));
        s
    }

    #[test]
    fn composed_schema_roundtrip() {
        let src = r#"
        ("test-crate", { instances: main, sub ;
        members: ("metagraph", main { entities: root Graph, Node, Edge; arrows { nodes: stdlib::Nodes, Graph -> Node; } }), ("m2-fixtures", sub { entities: root Subgraph, SubNode, MainNode(open); arrows { subnode: crate::roots::SubNodeOf, Subgraph -> SubNode, key(MainNode); } }) ;
        fusions: { sub.MainNode <-> main.Node ; } })
        "#;
        let ts: TokenStream = src.parse().expect("src parses");
        let once: ComposedSchema = syn::parse2(ts).expect("parse");
        assert_eq!(
            composed_sig(&once),
            "i:[main, sub];m:main/metagraph/3e/1a;m:sub/m2-fixtures/3e/1a;f:1;p:test-crate;"
        );
        let emitted = composed_schema_tokens(&once);
        let twice: ComposedSchema = syn::parse2(emitted.clone()).expect("reparse");
        assert_eq!(composed_sig(&once), composed_sig(&twice), "emitted: {emitted}");
    }
}

// ---------------------------------------------------------------------------
// Root registration metadata (the metadata tuple)
// ---------------------------------------------------------------------------

/// A root arrow's cardinality — the metadata tuple's fourth slot. `keyed` is
/// M2's third value (beside `one`/`many`), declared on the root via `key(K)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cardinality {
    One,
    Many,
    Keyed,
}

/// A root arrow's write-trait metadata — the metadata tuple's sixth, braced
/// slot (M4): per arrow-named write trait, its (trait name, method name,
/// ref associated-type names). `k_ref` is the key ref's assoc name on a
/// keyed root's `Insert` trait, `-` otherwise. Names are the *root's*
/// declaration, like the read slot's names (spike 1's E0407 finding).
#[derive(Clone, Debug)]
pub struct RootWriteMeta {
    pub trait_: Ident,
    pub method: Ident,
    pub a_ref: Ident,
    pub k_ref: Option<Ident>,
    pub b_ref: Ident,
}

impl RootWriteMeta {
    /// Derive the write metadata from a root pattern's single arrow:
    /// `Insert<Arrow>` for many/keyed roots, `Set<Arrow>` for one roots;
    /// the method is the verb + the arrow's snake name; the ref assoc
    /// names are the role entities' `<E>Ref` spellings.
    pub fn of_root_arrow(arrow: &ArrowDecl) -> syn::Result<RootWriteMeta> {
        let card = if arrow.key.is_some() {
            Cardinality::Keyed
        } else {
            match arrow.many {
                Some(true) => Cardinality::Many,
                Some(false) => Cardinality::One,
                None => {
                    return Err(syn::Error::new(
                        arrow.name.span(),
                        "a root arrow must declare a cardinality (`one`, `many`, or `key(K)`)",
                    ))
                }
            }
        };
        let camel_arrow = camel(&arrow.name.to_string());
        let (trait_, method) = match card {
            Cardinality::One => (
                Ident::new(&format!("Set{camel_arrow}"), arrow.name.span()),
                Ident::new(&format!("set_{}", snake(&arrow.name.to_string())), arrow.name.span()),
            ),
            Cardinality::Many | Cardinality::Keyed => (
                Ident::new(&format!("Insert{camel_arrow}"), arrow.name.span()),
                Ident::new(&format!("insert_{}", snake(&arrow.name.to_string())), arrow.name.span()),
            ),
        };
        Ok(RootWriteMeta {
            trait_,
            method,
            a_ref: Ident::new(&format!("{}Ref", arrow.source), arrow.name.span()),
            k_ref: (card == Cardinality::Keyed)
                .then(|| Ident::new(&format!("{}Ref", arrow.key.as_ref().unwrap()), arrow.name.span())),
            b_ref: Ident::new(&format!("{}Ref", arrow.target), arrow.name.span()),
        })
    }
}

impl Parse for RootWriteMeta {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let trait_: Ident = input.parse()?;
        input.parse::<Token![,]>()?;
        let method = input.parse()?;
        input.parse::<Token![,]>()?;
        let a_ref = input.parse()?;
        input.parse::<Token![,]>()?;
        let k_ref = if input.peek(Token![-]) {
            input.parse::<Token![-]>()?;
            None
        } else {
            Some(input.parse()?)
        };
        input.parse::<Token![,]>()?;
        let b_ref = input.parse()?;
        Ok(RootWriteMeta { trait_, method, a_ref, k_ref, b_ref })
    }
}

/// Re-emit the write metadata as the tuple's sixth, braced element.
pub fn root_write_meta_tokens(m: &RootWriteMeta) -> TokenStream {
    let trait_ = &m.trait_;
    let method = &m.method;
    let a_ref = &m.a_ref;
    let k_ref = m.k_ref.as_ref().map(|k| quote!(#k)).unwrap_or(quote!(-));
    let b_ref = &m.b_ref;
    quote!({ #trait_, #method, #a_ref, #k_ref, #b_ref })
}

/// A root pattern's trait metadata, published by its registration courier:
/// (method name, associated type name, GAT name, cardinality, key associated
/// type name). The key slot carries the key's associated type name for a
/// keyed root (`Key`), `-` otherwise; the sixth, braced slot carries the
/// write-trait metadata (`-` when absent — an in-flight payload from an
/// older emission). Method/GAT/associated-type names are the *root's*
/// declaration — never the instantiation's local alias (spike 1's E0407
/// finding).
#[derive(Clone, Debug)]
pub struct RootMeta {
    pub method: Ident,
    pub assoc: Ident,
    pub gat: Option<Ident>,
    pub card: Cardinality,
    pub key: Option<Ident>,
    pub write: Option<RootWriteMeta>,
}

impl RootMeta {
    /// Derive the metadata tuple from a root pattern's single arrow. The
    /// associated type is the target entity's name; the GAT is the
    /// camel-cased arrow name (many arrows only); a `key(K)` clause declares
    /// the keyed cardinality, whose key rides as the associated type `Key`.
    pub fn of_root_arrow(arrow: &ArrowDecl) -> syn::Result<RootMeta> {
        let card = if arrow.key.is_some() {
            Cardinality::Keyed
        } else {
            match arrow.many {
                Some(true) => Cardinality::Many,
                Some(false) => Cardinality::One,
                None => {
                    return Err(syn::Error::new(
                        arrow.name.span(),
                        "a root arrow must declare a cardinality (`one`, `many`, or `key(K)`)",
                    ))
                }
            }
        };
        let gat = (card == Cardinality::Many).then(|| {
            let g = camel(&arrow.name.to_string());
            Ident::new(&g, arrow.name.span())
        });
        Ok(RootMeta {
            method: arrow.name.clone(),
            assoc: arrow.target.clone(),
            gat,
            card,
            key: (card == Cardinality::Keyed).then(|| Ident::new("Key", arrow.name.span())),
            write: Some(RootWriteMeta::of_root_arrow(arrow)?),
        })
    }
}

impl Parse for RootMeta {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let method = input.parse()?;
        input.parse::<Token![,]>()?;
        let assoc = input.parse()?;
        input.parse::<Token![,]>()?;
        let gat = if input.peek(Token![-]) {
            input.parse::<Token![-]>()?;
            None
        } else {
            Some(input.parse()?)
        };
        input.parse::<Token![,]>()?;
        let card: Ident = input.parse()?;
        let card = match card.to_string().as_str() {
            "many" => Cardinality::Many,
            "one" => Cardinality::One,
            "keyed" => Cardinality::Keyed,
            other => {
                return Err(syn::Error::new(
                    card.span(),
                    format!(
                        "expected cardinality `one`, `many`, or `keyed`, found `{other}`"
                    ),
                ))
            }
        };
        input.parse::<Token![,]>()?;
        let key = if input.peek(Token![-]) {
            input.parse::<Token![-]>()?;
            None
        } else {
            Some(input.parse()?)
        };
        // The sixth, braced write slot: `-` or absent (an in-flight payload
        // from an older emission degrades to no write metadata), a brace
        // group parses as the write metadata.
        let write = if input.is_empty() {
            None
        } else {
            input.parse::<Token![,]>()?;
            if input.peek(Token![-]) {
                input.parse::<Token![-]>()?;
                None
            } else {
                let g: proc_macro2::Group = input.parse()?;
                Some(syn::parse2(g.stream())?)
            }
        };
        Ok(RootMeta { method, assoc, gat, card, key, write })
    }
}

/// Re-emit a metadata tuple in the shared courier grammar.
pub fn root_meta_tokens(m: &RootMeta) -> TokenStream {
    let method = &m.method;
    let assoc = &m.assoc;
    let gat = m.gat.as_ref().map(|g| quote!(#g)).unwrap_or(quote!(-));
    let card = match m.card {
        Cardinality::Many => quote!(many),
        Cardinality::One => quote!(one),
        Cardinality::Keyed => quote!(keyed),
    };
    let key = m.key.as_ref().map(|k| quote!(#k)).unwrap_or(quote!(-));
    let write = m
        .write
        .as_ref()
        .map(root_write_meta_tokens)
        .unwrap_or_else(|| quote!(-));
    quote!(#method, #assoc, #gat, #card, #key, #write)
}

// ---------------------------------------------------------------------------
// implement! bindings
// ---------------------------------------------------------------------------

/// One `arrow: backing` (or `instance.arrow: backing`, for composed
/// patterns) pair of an `implement!` block. Qualified keys are M2's composed
/// binding grammar; the flat bare-arrow form stays legal and means what it
/// meant in M1. The `instance.*` whole-instance sugar some spike tests used
/// is *not* grammar.
///
/// The backing itself is either a bare ident (`arena` — the stdlib spelling,
/// resolved against the runtime crate's `backings` module) or a path
/// (`my_backings::my_list`, M3 — the module holding the `backing!`
/// invocation plus the backing's name, exactly the shape a pattern path
/// takes).
#[derive(Clone)]
pub struct Binding {
    /// `Some(inst)` for the qualified `inst.arrow` key form.
    pub instance: Option<Ident>,
    pub arrow: Ident,
    /// The backing's name: the path's last segment, or the bare ident.
    pub backing: Ident,
    /// `Some(p)` for the path form `prefix..::name`; the courier lives at
    /// `prefix..::__metagraph_backing_name`.
    pub backing_path: Option<Path>,
}

impl Binding {
    /// Qualified key as displayed in messages: `inst.arrow` or `arrow`.
    pub fn key_str(&self) -> String {
        match &self.instance {
            Some(i) => format!("{i}.{}", self.arrow),
            None => self.arrow.to_string(),
        }
    }
}

/// Parse the braced `{ arrow: backing, ... }` (or `{ inst.arrow: backing,
/// ... }`) block. A backing is a bare ident or a module-qualified path
/// (`out_edges: stdlib::intrusive_list`); a single-segment path is rejected
/// the way a bare pattern name is — the courier it would name lives in a
/// module, not at a crate root.
pub fn parse_bindings(input: ParseStream) -> syn::Result<Vec<Binding>> {
    let content;
    braced!(content in input);
    let mut bindings = Vec::new();
    while !content.is_empty() {
        let first: Ident = content.parse()?;
        let (instance, arrow) = if content.peek(Token![.]) {
            content.parse::<Token![.]>()?;
            (Some(first), content.parse::<Ident>()?)
        } else {
            (None, first)
        };
        content.parse::<Token![:]>()?;
        let (backing, path) = if (content.peek(Ident) && content.peek2(Token![::]))
            || content.peek(Token![::])
        {
            let p: Path = content.parse()?;
            if p.segments.len() < 2 {
                return Err(syn::Error::new(
                    p.segments.last().map(|s| s.ident.span()).unwrap_or_else(proc_macro2::Span::call_site),
                    format!(
                        "`{}` is a bare backing name written as a path; name the module holding the `backing!` invocation (e.g. `my_backings::{}`), or use the bare stdlib spelling",
                        path_tokens(&p),
                        p.segments.last().map(|s| s.ident.to_string()).unwrap_or_default()
                    ),
                ));
            }
            let last = p.segments.last().unwrap().ident.clone();
            (last, Some(p))
        } else {
            (content.parse()?, None)
        };
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
        bindings.push(Binding { instance, arrow, backing, backing_path: path });
    }
    Ok(bindings)
}

/// Re-emit parsed bindings (canonically `inst.arrow: backing,` /
/// `arrow: backing,`; the path form re-emits verbatim).
pub fn bindings_tokens(bindings: &[Binding]) -> TokenStream {
    bindings
        .iter()
        .map(|b| {
            let k = match &b.backing_path {
                Some(p) => path_tokens(p),
                None => {
                    let k = &b.backing;
                    quote!(#k)
                }
            };
            match &b.instance {
                Some(i) => {
                    let a = &b.arrow;
                    quote!(#i.#a: #k,)
                }
                None => {
                    let a = &b.arrow;
                    quote!(#a: #k,)
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Round-trip tests: parse(emit(parse(x))) must agree with parse(x).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;

    fn parse_schema(ts: &TokenStream) -> SchemaBody {
        syn::parse2(ts.clone()).expect("schema round-trip parse")
    }

    /// Structural signature used to compare two parses (spans differ).
    fn schema_sig(body: &SchemaBody) -> String {
        let mut s = String::new();
        for e in &body.entities {
            s.push_str(&format!(
                "e:{}{}{};",
                e.name,
                if e.root { "^root" } else { "" },
                if e.open { "^open" } else { "" }
            ));
        }
        for a in &body.arrows {
            s.push_str(&format!("a:{}:", a.name));
            if let Some(p) = &a.inst {
                s.push_str(&p.to_token_stream().to_string());
            }
            s.push_str(&format!(":{}->{}:", a.source, a.target));
            match a.many {
                Some(true) => s.push_str("many"),
                Some(false) => s.push_str("one"),
                None => s.push('-'),
            }
            if let Some(b) = &a.by {
                s.push_str(&format!(",by {}", b));
            }
            if let Some(k) = &a.key {
                s.push_str(&format!(",key({})", k));
            }
            if let Some((x, y)) = &a.sum {
                s.push_str(&format!(",sum({},{})", x, y));
            }
            s.push(';');
        }
        s
    }

    fn assert_schema_roundtrip(src: &str) {
        let ts: TokenStream = src.parse().expect("src parses");
        let once = parse_schema(&ts);
        let emitted = schema_body_tokens(&once);
        let twice = parse_schema(&emitted);
        assert_eq!(schema_sig(&once), schema_sig(&twice), "src: {src}");
        // Idempotence of emission: emitting the reparsed schema gives the
        // same token stream.
        assert_eq!(
            emitted.to_string(),
            schema_body_tokens(&twice).to_string(),
            "src: {src}"
        );
    }

    #[test]
    fn roundtrip_root_pattern_bodies() {
        assert_schema_roundtrip(
            "entities: root Graph, Node; arrows { nodes: Graph -> Node, many; }",
        );
        assert_schema_roundtrip(
            "entities: root Edge, Node; arrows { source: Edge -> Node, one; }",
        );
    }

    #[test]
    fn roundtrip_digraph_body() {
        assert_schema_roundtrip(
            r#"
            entities: root Graph, Node, Edge;
            arrows {
                nodes:     stdlib::Nodes,    Graph -> Node;
                out_edges: stdlib::OutEdges, Node -> Edge, by source;
                in_edges:  stdlib::InEdges,  Node -> Edge, by target;
                source:    stdlib::Source,   Edge -> Node;
                target:    stdlib::Target,   Edge -> Node;
                edges:     stdlib::Edges,    Node -> Edge := sum(out_edges, in_edges);
            }
            "#,
        );
    }

    #[test]
    fn roundtrip_crate_rooted_and_open_entities() {
        assert_schema_roundtrip(
            "entities: root T, SubNode, MainNode(open); arrows { a: crate::my_roots::MyRoot, SubNode -> MainNode; }",
        );
    }

    #[test]
    fn roundtrip_keyed_arrows() {
        // Root side: `key(K)` declares the cardinality; instantiation side:
        // `key(LocalEntity)` binds the key sort (no cardinality restated).
        assert_schema_roundtrip(
            "entities: root Subgraph, SubNode, MainNode; arrows { subnode: Subgraph -> SubNode, key(MainNode); }",
        );
        assert_schema_roundtrip(
            "entities: root Subgraph, SubNode, SubEdge, MainNode(open), MainEdge(open); arrows { subnode: crate::roots::SubNodeOf, Subgraph -> SubNode, key(MainNode); }",
        );
    }

    #[test]
    fn roundtrip_root_meta_tuple() {
        for src in [
            "nodes, Node, Nodes, many, -, { InsertNodes, insert_nodes, GraphRef, -, NodeRef }",
            "source, Node, -, one, -, { SetSource, set_source, EdgeRef, -, NodeRef }",
            "subnode, SubNode, -, keyed, Key, { InsertSubnode, insert_subnode, SubgraphRef, MainNodeRef, SubNodeRef }",
            // In-flight payloads from an older emission (no sixth slot, or
            // `-`) degrade to no write metadata.
            "nodes, Node, Nodes, many, -",
            "nodes, Node, Nodes, many, -, -",
        ] {
            let ts: TokenStream = src.parse().expect("tuple parses");
            let once: RootMeta = syn::parse2(ts).expect("tuple parse");
            let emitted = root_meta_tokens(&once);
            let twice: RootMeta = syn::parse2(emitted).expect("tuple reparse");
            assert_eq!(once.method, twice.method, "src: {src}");
            assert_eq!(once.assoc, twice.assoc, "src: {src}");
            assert_eq!(once.gat, twice.gat, "src: {src}");
            assert_eq!(once.card, twice.card, "src: {src}");
            assert_eq!(once.key, twice.key, "src: {src}");
            match (&once.write, &twice.write) {
                (Some(w), Some(w2)) => {
                    assert_eq!(w.trait_, w2.trait_, "src: {src}");
                    assert_eq!(w.method, w2.method, "src: {src}");
                    assert_eq!(w.a_ref, w2.a_ref, "src: {src}");
                    assert_eq!(w.k_ref, w2.k_ref, "src: {src}");
                    assert_eq!(w.b_ref, w2.b_ref, "src: {src}");
                }
                (None, None) => {}
                _ => panic!("write slot mismatch for {src}"),
            }
        }
    }

    #[test]
    fn entity_ctor_args_by_derived_only_rule() {
        use super::SchemaBody;
        use proc_macro2::{Ident as PIdent, Span};
        let parse_body = |src: &str| {
            syn::parse::Parser::parse2(
                |input: syn::parse::ParseStream| {
                    let b: SchemaBody = input.parse()?;
                    Ok(b)
                },
                src.parse::<TokenStream>().expect("body parses"),
            )
            .expect("body")
        };
        let id = |s: &str| PIdent::new(s, Span::call_site());
        // Digraph's Edge: `source`/`target` are provable as `by`
        // determinants, in declaration order.
        let digraph = parse_body(
            "entities: root Graph, Node, Edge; arrows { nodes: stdlib::Nodes, Graph -> Node; out_edges: stdlib::OutEdges, Node -> Edge, by source; in_edges: stdlib::InEdges, Node -> Edge, by target; source: stdlib::Source, Edge -> Node; target: stdlib::Target, Edge -> Node; edges: stdlib::Edges, Node -> Edge := sum(out_edges, in_edges); }",
        );
        assert_eq!(
            digraph.entity_ctor_args(&id("Edge")),
            Some(vec![id("source"), id("target")])
        );
        // Node: by-bearing and sum arrows are not one-arrow candidates.
        assert_eq!(digraph.entity_ctor_args(&id("Node")), Some(vec![]));
        // CsrDigraph's E: `source`/`target` are plain instantiations, named
        // by no `by` clause — opaque, the Add trait is vetoed.
        let csr = parse_body(
            "entities: root G, N, E; arrows { nodes: stdlib::Nodes, G -> N; out: stdlib::OutEdges, N -> E; source: stdlib::Source, E -> N; target: stdlib::Target, E -> N; }",
        );
        assert_eq!(csr.entity_ctor_args(&id("E")), None);
        // CsrDigraph's N: the primitive `out` instantiation is opaque too.
        assert_eq!(csr.entity_ctor_args(&id("N")), None);
    }

    #[test]
    fn bindings_parse_and_roundtrip() {
        use super::parse_bindings;
        let parse_all = |src: &str| {
            syn::parse::Parser::parse2(
                |input: syn::parse::ParseStream| parse_bindings(input),
                src.parse::<TokenStream>().expect("bindings parse"),
            )
            .expect("bindings")
        };
        let once = parse_all("{ nodes: arena, sub.subnodes: arena, sub.subnode: hash_index }");
        assert_eq!(once.len(), 3);
        assert_eq!(once[0].key_str(), "nodes");
        assert_eq!(once[1].key_str(), "sub.subnodes");
        assert_eq!(once[2].key_str(), "sub.subnode");
        let emitted = bindings_tokens(&once);
        let emitted_str = format!("{{ {emitted} }}");
        let twice = parse_all(&emitted_str);
        assert_eq!(
            once.iter().map(|b| b.key_str()).collect::<Vec<_>>(),
            twice.iter().map(|b| b.key_str()).collect::<Vec<_>>(),
            "emitted: {emitted_str}"
        );
        assert_eq!(
            emitted.to_string(),
            bindings_tokens(&twice).to_string(),
            "idempotent emission"
        );
    }

    #[test]
    fn bindings_parse_and_roundtrip_paths() {
        use super::parse_bindings;
        let parse_all = |src: &str| {
            syn::parse::Parser::parse2(
                |input: syn::parse::ParseStream| parse_bindings(input),
                src.parse::<TokenStream>().expect("bindings parse"),
            )
            .expect("bindings")
        };
        let once = parse_all(
            "{ nodes: arena, out_edges: my::intrusive_list, sub.subnode: ::mg::backings::hash_index }",
        );
        assert_eq!(once.len(), 3);
        assert_eq!(once[0].backing, "arena");
        assert!(once[0].backing_path.is_none());
        assert_eq!(once[1].backing, "intrusive_list");
        assert_eq!(
            once[1].backing_path.as_ref().unwrap().to_token_stream().to_string(),
            "my :: intrusive_list"
        );
        assert_eq!(once[2].key_str(), "sub.subnode");
        assert_eq!(once[2].backing, "hash_index");
        assert_eq!(
            once[2].backing_path.as_ref().unwrap().to_token_stream().to_string(),
            ":: mg :: backings :: hash_index"
        );
        let emitted = bindings_tokens(&once);
        let emitted_str = format!("{{ {emitted} }}");
        let twice = parse_all(&emitted_str);
        assert_eq!(
            once.iter()
                .map(|b| (
                    b.key_str(),
                    b.backing.to_string(),
                    b.backing_path
                        .as_ref()
                        .map(|p| path_tokens(p).to_string())
                        .unwrap_or_default()
                ))
                .collect::<Vec<_>>(),
            twice.iter()
                .map(|b| (
                    b.key_str(),
                    b.backing.to_string(),
                    b.backing_path
                        .as_ref()
                        .map(|p| path_tokens(p).to_string())
                        .unwrap_or_default()
                ))
                .collect::<Vec<_>>(),
            "emitted: {emitted_str}"
        );
        assert_eq!(
            emitted.to_string(),
            bindings_tokens(&twice).to_string(),
            "idempotent emission"
        );
        // A single-segment path is the bare name misspelled: rejected with
        // the module hint, exactly as bare pattern names are.
        let err = syn::parse::Parser::parse2(
            |input: syn::parse::ParseStream| parse_bindings(input),
            "{ nodes: ::arena }".parse::<TokenStream>().unwrap(),
        )
        .err()
        .expect("expected rejection");
        assert!(
            err.to_string().contains("bare backing name written as a path"),
            "got: {err}"
        );
    }

    #[test]
    fn naming_derivations() {
        assert_eq!(snake("OutEdges"), "out_edges");
        assert_eq!(camel("out_edges"), "OutEdges");
        assert_eq!(link_stem(&format_ident!("out_edges")), "out");
        assert_eq!(link_stem(&format_ident!("subnodes")), "subnodes");
        assert_eq!(arena_fld(&format_ident!("Node")), "node_arena");
        assert_eq!(iter_ty(&format_ident!("out_edges")), "OutEdgesIter");
        assert_eq!(marker_ty(&format_ident!("out_edges")), "OutEdges");
    }
}
