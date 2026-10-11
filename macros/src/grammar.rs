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
    /// `:= union(a, b)` — derived read over two sibling `many` arrows.
    pub union: Option<(Ident, Ident)>,
}

impl Parse for ArrowDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        // Optional instantiation path followed by a comma.
        let inst = if (input.peek(Ident) && input.peek2(Token![::]))
            || (input.peek(Token![crate]) && input.peek2(Token![::]))
            || (input.peek(Ident) && input.peek2(Token![,]))
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
        let mut union = None;
        loop {
            if input.peek(Token![;]) {
                input.parse::<Token![;]>()?;
                break;
            }
            if input.peek(Token![:]) && input.peek2(Token![=]) {
                // `:= union(a, b)` — terminates the declaration.
                input.parse::<Token![:]>()?;
                input.parse::<Token![=]>()?;
                let kw: Ident = input.parse()?;
                if kw != "union" {
                    return Err(
                        syn::Error::new(kw.span(), "only `union` derivation is supported")
                    );
                }
                let content;
                syn::parenthesized!(content in input);
                let a: Ident = content.parse()?;
                content.parse::<Token![,]>()?;
                let b: Ident = content.parse()?;
                input.parse::<Token![;]>()?;
                union = Some((a, b));
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
                other => {
                    return Err(syn::Error::new(
                        kw.span(),
                        format!(
                            "unexpected `{other}` in arrow declaration (expected `one`, `many`, `by`, or `:= union(..)`)"
                        ),
                    ));
                }
            }
        }
        Ok(ArrowDecl { name, inst, source, target, many, by, union })
    }
}

#[derive(Clone)]
pub struct SchemaBody {
    pub entities: Vec<EntityDecl>,
    pub arrows: Vec<ArrowDecl>,
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
        let union_ts = a
            .union
            .as_ref()
            .map(|(x, y)| quote!(:= union(#x, #y)))
            .unwrap_or_default();
        quote!(#n: #inst_ts #s -> #t #card_ts #by_ts #union_ts ;)
    });
    quote! {
        entities: #(#entities),* ; arrows: { #(#arrows)* }
    }
}

// ---------------------------------------------------------------------------
// Root registration metadata (the metadata tuple)
// ---------------------------------------------------------------------------

/// A root pattern's trait metadata, published by its registration courier:
/// (method name, associated type name, GAT name, cardinality, key associated
/// type name). The key slot is reserved for M2's keyed partial arrows and is
/// always `-` in M1. Method/GAT/associated-type names are the *root's*
/// declaration — never the instantiation's local alias (spike 1's E0407
/// finding).
#[derive(Clone, Debug)]
pub struct RootMeta {
    pub method: Ident,
    pub assoc: Ident,
    pub gat: Option<Ident>,
    pub many: bool,
    pub key: Option<Ident>,
}

impl RootMeta {
    /// Derive the metadata tuple from a root pattern's single arrow. The
    /// associated type is the target entity's name; the GAT is the
    /// camel-cased arrow name (many arrows only).
    pub fn of_root_arrow(arrow: &ArrowDecl) -> syn::Result<RootMeta> {
        let many = arrow.many.ok_or_else(|| {
            syn::Error::new(
                arrow.name.span(),
                "a root arrow must declare a cardinality (`one` or `many`)",
            )
        })?;
        let gat = many.then(|| {
            let g = camel(&arrow.name.to_string());
            Ident::new(&g, arrow.name.span())
        });
        Ok(RootMeta {
            method: arrow.name.clone(),
            assoc: arrow.target.clone(),
            gat,
            many,
            key: None, // M2 slot: keyed partial arrows
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
        let many = match card.to_string().as_str() {
            "many" => true,
            "one" => false,
            other => {
                return Err(syn::Error::new(
                    card.span(),
                    format!("expected cardinality `one` or `many`, found `{other}`"),
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
        Ok(RootMeta { method, assoc, gat, many, key })
    }
}

/// Re-emit a metadata tuple in the shared courier grammar.
pub fn root_meta_tokens(m: &RootMeta) -> TokenStream {
    let method = &m.method;
    let assoc = &m.assoc;
    let gat = m.gat.as_ref().map(|g| quote!(#g)).unwrap_or(quote!(-));
    let card = if m.many { quote!(many) } else { quote!(one) };
    let key = m.key.as_ref().map(|k| quote!(#k)).unwrap_or(quote!(-));
    quote!(#method, #assoc, #gat, #card, #key)
}

// ---------------------------------------------------------------------------
// implement! bindings
// ---------------------------------------------------------------------------

/// One `arrow: backing` pair of an `implement!` block.
#[derive(Clone)]
pub struct Binding {
    pub arrow: Ident,
    pub backing: Ident,
}

/// Parse the braced `{ arrow: backing, ... }` block.
pub fn parse_bindings(input: ParseStream) -> syn::Result<Vec<Binding>> {
    let content;
    braced!(content in input);
    let mut bindings = Vec::new();
    while !content.is_empty() {
        let arrow: Ident = content.parse()?;
        content.parse::<Token![:]>()?;
        let backing: Ident = content.parse()?;
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
        bindings.push(Binding { arrow, backing });
    }
    Ok(bindings)
}

/// Re-emit parsed bindings (canonically `arrow: backing,`).
pub fn bindings_tokens(bindings: &[Binding]) -> TokenStream {
    bindings
        .iter()
        .map(|b| {
            let a = &b.arrow;
            let k = &b.backing;
            quote!(#a: #k,)
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
                None => s.push_str("-"),
            }
            if let Some(b) = &a.by {
                s.push_str(&format!(",by {}", b));
            }
            if let Some((x, y)) = &a.union {
                s.push_str(&format!(",union({},{})", x, y));
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
                edges:     stdlib::Edges,    Node -> Edge := union(out_edges, in_edges);
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
    fn roundtrip_root_meta_tuple() {
        for src in [
            "nodes, Node, Nodes, many, -",
            "source, Node, -, one, -",
            "subnode, SubNode, Subnode, many, Key",
        ] {
            let ts: TokenStream = src.parse().expect("tuple parses");
            let once: RootMeta = syn::parse2(ts).expect("tuple parse");
            let emitted = root_meta_tokens(&once);
            let twice: RootMeta = syn::parse2(emitted).expect("tuple reparse");
            assert_eq!(once.method, twice.method, "src: {src}");
            assert_eq!(once.assoc, twice.assoc, "src: {src}");
            assert_eq!(once.gat, twice.gat, "src: {src}");
            assert_eq!(once.many, twice.many, "src: {src}");
            assert_eq!(once.key, twice.key, "src: {src}");
        }
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
