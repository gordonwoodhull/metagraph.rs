//! The `backing!` declaration grammar (plans/milestone-3): what `backing!`
//! parses and re-emits into its courier, and what `__implement_gen!` parses
//! back out at the implement site. One module, both directions, with
//! round-trip tests — the same shared-grammar commitment as `grammar.rs`.
//!
//! The clause inventory, its mandatory/optional split, and the selection
//! rules are plan-pinned (milestone-3, "The clause grammar, pinned (round
//! 4)"); this module is the parser-author invention the plan anticipates.
//! One documented extension: contributed-field naming formulas. The plan
//! pins a per-site formula per backing (`value` emits as the arrow's local
//! name, `first`/`next` as `first_<stem>`/`next_<stem>`, `index` as
//! `<arrow>_index`, csr's stems bare) and states there is no single
//! mechanical rule — so the formula is stated per field as
//! `name: Type = arrow|stem|suffix`, defaulting to the bare declared name.
//! Authored bodies reference the *logical* field name; codegen renames to
//! the emitted one per binding.

use proc_macro2::{Ident as Ident2, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::parse::{Parse, ParseStream};
use syn::{braced, Ident, Token};

/// A parsed `backing!` declaration: `name: A -> B, cardinality { clauses }`.
/// The entity variables are pinned to `A` (self) and `B` (target); a keyed
/// declaration adds the key variable `K` via `key(K)`.
#[derive(Clone)]
pub struct BackingDecl {
    pub name: Ident,
    pub card: BackingCard,
    pub clauses: Vec<Clause>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackingCard {
    One,
    Many,
    Keyed,
}

impl BackingDecl {
    /// The first clause matching the selector, if any.
    pub fn clause(&self, sel: impl Fn(&Clause) -> bool) -> Option<&Clause> {
        self.clauses.iter().find(|c| sel(c))
    }
    pub fn has(&self, sel: impl Fn(&Clause) -> bool) -> bool {
        self.clauses.iter().any(|c| sel(c))
    }
    /// `true` when the declaration carries `phase:` annotations (csr is the
    /// only phased backing in M3).
    pub fn is_phased(&self) -> bool {
        self.has(|c| matches!(c, Clause::Phase { .. }))
    }
}

impl Parse for BackingDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let a: Ident = input.parse()?;
        if a != "A" {
            return Err(syn::Error::new(
                a.span(),
                "a backing declaration binds the self entity variable `A` (write `A -> B`)",
            ));
        }
        input.parse::<Token![->]>()?;
        let b: Ident = input.parse()?;
        if b != "B" {
            return Err(syn::Error::new(
                b.span(),
                "a backing declaration binds the target entity variable `B` (write `A -> B`)",
            ));
        }
        input.parse::<Token![,]>()?;
        let card = if input.peek(Ident) {
            let kw: Ident = input.parse()?;
            match kw.to_string().as_str() {
                "one" => BackingCard::One,
                "many" => BackingCard::Many,
                "key" => {
                    let content;
                    syn::parenthesized!(content in input);
                    let k: Ident = content.parse()?;
                    if k != "K" {
                        return Err(syn::Error::new(
                            k.span(),
                            "a keyed declaration binds the key variable `K` (write `key(K)`)",
                        ));
                    }
                    BackingCard::Keyed
                }
                other => {
                    return Err(syn::Error::new(
                        kw.span(),
                        format!(
                            "expected a cardinality (`one`, `many`, or `key(K)`), found `{other}`"
                        ),
                    ))
                }
            }
        } else {
            return Err(syn::Error::new(
                input.span(),
                "a backing declaration needs a cardinality (`one`, `many`, or `key(K)`) before its clause block",
            ));
        };
        let content;
        braced!(content in input);
        let mut clauses = Vec::new();
        while !content.is_empty() {
            clauses.push(content.parse()?);
        }
        let decl = BackingDecl { name, card, clauses };
        validate_inventory(&decl)?;
        Ok(decl)
    }
}

/// The parsed clause inventory. Every clause kind the pinned table knows;
/// the parser rejects anything else with the closed set quoted.
#[derive(Clone)]
pub enum Clause {
    /// `on A { .. }` / `on B { .. }` / `on store [build|frozen] { .. }`.
    On {
        target: OnTarget,
        label: Option<PhaseLabel>,
        fields: Vec<ContribField>,
        /// Span of the `on` keyword, for error reporting.
        span: proc_macro2::Span,
    },
    /// `ctor_arg` — the field is fed by a constructor argument named after
    /// the arrow; requires an `add_<A>`-orchestrated self.
    CtorArg { span: proc_macro2::Span },
    /// `read(a) { .. }` — the `one` access protocol, id-space (codegen
    /// wraps the result into the target handle).
    Read { body: TokenStream2, span: proc_macro2::Span },
    /// `lookup(a, k) { .. }` — the keyed access protocol.
    Lookup { body: TokenStream2, span: proc_macro2::Span },
    /// `iter Walk { cur: u32, .. }` — the iterator struct's authored fields
    /// (codegen adds the store ref, and at embedded placements the host).
    Iter { walk: Ident, fields: Vec<(Ident, syn::Type)>, span: proc_macro2::Span },
    /// `iterate(a) { Walk { .. } }` — constructs the Walk.
    Iterate { body: TokenStream2, span: proc_macro2::Span },
    /// `advance(w) { .. }` — authors the whole `next()` body; `handle(x)`
    /// mints the target handle.
    Advance { body: TokenStream2, span: proc_macro2::Span },
    /// `on_insert(a, b) { .. }` (many) / `on_insert(a, k, b) { .. }` (keyed).
    OnInsert { keyed: bool, body: TokenStream2, span: proc_macro2::Span },
    /// `on_add(a) { .. }` — per-add growth of store-level per-self state.
    OnAdd { body: TokenStream2, span: proc_macro2::Span },
    /// `compact { .. }` — the compaction body, legal only on phased backings.
    Compact { body: TokenStream2, span: proc_macro2::Span },
    /// `validate { .. }` / `validate by { .. }` / `validate primitive { .. }`.
    Validate { variant: ValidateVariant, body: TokenStream2, span: proc_macro2::Span },
    /// `membership: existence`.
    MembershipExistence { span: proc_macro2::Span },
    /// `requires: <predicates>` — the closed applicability vocabulary.
    Requires { items: Vec<Ident>, span: proc_macro2::Span },
    /// `contract: <annotations>` — the closed contract vocabulary.
    Contract { items: Vec<Ident>, span: proc_macro2::Span },
    /// `iteration_order: <value>`.
    IterationOrder { value: Ident, span: proc_macro2::Span },
    /// `phase: <properties>` — spike 6's vocabulary; a comma list of bare
    /// idents or `ident: value` pairs.
    Phase { items: Vec<PhaseItem>, span: proc_macro2::Span },
}

impl Clause {
    /// A stable name for repetition/selection diagnostics.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Clause::On { .. } => "on",
            Clause::CtorArg { .. } => "ctor_arg",
            Clause::Read { .. } => "read",
            Clause::Lookup { .. } => "lookup",
            Clause::Iter { .. } => "iter",
            Clause::Iterate { .. } => "iterate",
            Clause::Advance { .. } => "advance",
            Clause::OnInsert { .. } => "on_insert",
            Clause::OnAdd { .. } => "on_add",
            Clause::Compact { .. } => "compact",
            Clause::Validate { variant, .. } => match variant {
                ValidateVariant::Plain => "validate",
                ValidateVariant::By => "validate by",
                ValidateVariant::Primitive => "validate primitive",
            },
            Clause::MembershipExistence { .. } => "membership",
            Clause::Requires { .. } => "requires",
            Clause::Contract { .. } => "contract",
            Clause::IterationOrder { .. } => "iteration_order",
            Clause::Phase { .. } => "phase",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OnTarget {
    A,
    B,
    Store,
}

impl OnTarget {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let t: Ident = input.parse()?;
        match t.to_string().as_str() {
            "A" => Ok(OnTarget::A),
            "B" => Ok(OnTarget::B),
            "store" => Ok(OnTarget::Store),
            other => Err(syn::Error::new(
                t.span(),
                format!("`on` targets `A`, `B`, or `store` (the backing's own entity variables), found `{other}`"),
            )),
        }
    }
    fn name(&self) -> &'static str {
        match self {
            OnTarget::A => "A",
            OnTarget::B => "B",
            OnTarget::Store => "store",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PhaseLabel {
    Build,
    Frozen,
}

impl PhaseLabel {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let l: Ident = input.parse()?;
        match l.to_string().as_str() {
            "build" => Ok(PhaseLabel::Build),
            "frozen" => Ok(PhaseLabel::Frozen),
            other => Err(syn::Error::new(
                l.span(),
                format!(
                    "store phase labels come from the closed set {{build, frozen}} (the `phase:` annotations describe properties, not labels); found `{other}`"
                ),
            )),
        }
    }
}

/// One contributed field: the logical name, its declared type (over the
/// `AId`/`BId`/`KId` type roles), and the emitted-name formula.
#[derive(Clone)]
pub struct ContribField {
    pub logical: Ident,
    pub ty: syn::Type,
    pub formula: NameFormula,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum NameFormula {
    /// The declared name is the emitted name (csr's stems).
    #[default]
    Bare,
    /// The instantiation's local arrow name (field's `value` -> `source`).
    Arrow,
    /// `<declared>_<link_stem(arrow)>` (intrusive's `first` -> `first_out`).
    Stem,
    /// `<arrow>_<declared>` (hash_index's `index` -> `subnode_index`).
    Suffix,
}

impl NameFormula {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let f: Ident = input.parse()?;
        match f.to_string().as_str() {
            "arrow" => Ok(NameFormula::Arrow),
            "stem" => Ok(NameFormula::Stem),
            "suffix" => Ok(NameFormula::Suffix),
            other => Err(syn::Error::new(
                f.span(),
                format!(
                    "naming formulas are `arrow` (the arrow's local name), `stem` (`<name>_<arrow-stem>`), or `suffix` (`<arrow>_<name>`); found `{other}`"
                ),
            )),
        }
    }
}

/// Apply a field's naming formula for one binding. `link_stem` is the
/// arrow's stem via M1's `_edges`-strip.
pub fn formula_name(
    f: &ContribField,
    formula: NameFormula,
    arrow: &Ident,
    link_stem: &Ident,
) -> Ident2 {
    match formula {
        NameFormula::Bare => Ident2::new(&f.logical.to_string(), f.logical.span()),
        NameFormula::Arrow => Ident2::new(&arrow.to_string(), f.logical.span()),
        NameFormula::Stem => Ident2::new(
            &format!("{}_{}", f.logical, link_stem),
            f.logical.span(),
        ),
        NameFormula::Suffix => Ident2::new(
            &format!("{}_{}", arrow, f.logical),
            f.logical.span(),
        ),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValidateVariant {
    Plain,
    By,
    Primitive,
}

#[derive(Clone, Debug)]
pub struct PhaseItem {
    pub name: Ident,
    /// `ident: value` items carry their value; bare properties don't.
    pub value: Option<Ident>,
}

fn parse_annotation_items(input: ParseStream) -> syn::Result<Vec<Ident>> {
    let mut items = Vec::new();
    loop {
        let item: Ident = input.parse()?;
        items.push(item);
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        } else {
            break;
        }
    }
    Ok(items)
}

fn parse_phase_items(input: ParseStream) -> syn::Result<Vec<PhaseItem>> {
    let mut items = Vec::new();
    loop {
        let name: Ident = input.parse()?;
        let value = if input.peek(Token![:]) {
            input.parse::<Token![:]>()?;
            Some(input.parse()?)
        } else {
            None
        };
        items.push(PhaseItem { name, value });
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        } else {
            break;
        }
    }
    Ok(items)
}

/// Parse the `on`-block field list: `name: Type [= formula],*`.
fn parse_contrib_fields(input: ParseStream) -> syn::Result<Vec<ContribField>> {
    let mut fields = Vec::new();
    while !input.is_empty() {
        let logical: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let ty: syn::Type = input.parse()?;
        let formula = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            NameFormula::parse(input)?
        } else {
            NameFormula::Bare
        };
        fields.push(ContribField { logical, ty, formula });
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
    }
    Ok(fields)
}

impl Parse for Clause {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let kw: Ident = input.parse()?;
        let span = kw.span();
        match kw.to_string().as_str() {
            "on" => {
                let target = OnTarget::parse(input)?;
                let label = if target == OnTarget::Store && input.peek(Ident) && input.peek2(syn::token::Brace) {
                    Some(PhaseLabel::parse(input)?)
                } else {
                    None
                };
                let content;
                braced!(content in input);
                let fields = parse_contrib_fields(&content)?;
                Ok(Clause::On { target, label, fields, span })
            }
            "ctor_arg" => Ok(Clause::CtorArg { span }),
            "read" => {
                let content;
                syn::parenthesized!(content in input);
                let p: Ident = content.parse()?;
                if p != "a" {
                    return Err(syn::Error::new(
                        p.span(),
                        "`read`'s parameter is the self id `a` (write `read(a)`)",
                    ));
                }
                let body: TokenStream2 = {
                    let content;
                    braced!(content in input);
                    content.parse()?
                };
                Ok(Clause::Read { body, span })
            }
            "lookup" => {
                let content;
                syn::parenthesized!(content in input);
                let p: Ident = content.parse()?;
                if p != "a" {
                    return Err(syn::Error::new(
                        p.span(),
                        "`lookup`'s first parameter is the self id `a` (write `lookup(a, k)`)",
                    ));
                }
                content.parse::<Token![,]>()?;
                let k: Ident = content.parse()?;
                if k != "k" {
                    return Err(syn::Error::new(
                        k.span(),
                        "`lookup`'s second parameter is the key id `k` (write `lookup(a, k)`)",
                    ));
                }
                let body: TokenStream2 = {
                    let content;
                    braced!(content in input);
                    content.parse()?
                };
                Ok(Clause::Lookup { body, span })
            }
            "iter" => {
                let walk: Ident = input.parse()?;
                let content;
                braced!(content in input);
                let mut fields = Vec::new();
                while !content.is_empty() {
                    let f: Ident = content.parse()?;
                    content.parse::<Token![:]>()?;
                    let ty: syn::Type = content.parse()?;
                    fields.push((f, ty));
                    if content.peek(Token![,]) {
                        content.parse::<Token![,]>()?;
                    }
                }
                Ok(Clause::Iter { walk, fields, span })
            }
            "iterate" => {
                let content;
                syn::parenthesized!(content in input);
                let p: Ident = content.parse()?;
                if p != "a" {
                    return Err(syn::Error::new(
                        p.span(),
                        "`iterate`'s parameter is the self id `a` (write `iterate(a)`)",
                    ));
                }
                let body: TokenStream2 = {
                    let content;
                    braced!(content in input);
                    content.parse()?
                };
                Ok(Clause::Iterate { body, span })
            }
            "advance" => {
                let content;
                syn::parenthesized!(content in input);
                let p: Ident = content.parse()?;
                if p != "w" {
                    return Err(syn::Error::new(
                        p.span(),
                        "`advance`'s parameter is the walk `w` (write `advance(w)`)",
                    ));
                }
                let body: TokenStream2 = {
                    let content;
                    braced!(content in input);
                    content.parse()?
                };
                Ok(Clause::Advance { body, span })
            }
            "on_insert" => {
                let content;
                syn::parenthesized!(content in input);
                let p: Ident = content.parse()?;
                if p != "a" {
                    return Err(syn::Error::new(
                        p.span(),
                        "`on_insert`'s first parameter is the self id `a` (write `on_insert(a, b)` or `on_insert(a, k, b)`)",
                    ));
                }
                let (keyed, b_param) = if content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                    let second: Ident = content.parse()?;
                    if second == "k" && content.peek(Token![,]) {
                        content.parse::<Token![,]>()?;
                        let b: Ident = content.parse()?;
                        (true, b)
                    } else {
                        (false, second)
                    }
                } else {
                    (false, content.parse()?)
                };
                if b_param != "b" {
                    return Err(syn::Error::new(
                        b_param.span(),
                        "`on_insert`'s target parameter is the target id `b` (write `on_insert(a, b)`)",
                    ));
                }
                let body: TokenStream2 = {
                    let content;
                    braced!(content in input);
                    content.parse()?
                };
                Ok(Clause::OnInsert { keyed, body, span })
            }
            "on_add" => {
                let content;
                syn::parenthesized!(content in input);
                let p: Ident = content.parse()?;
                if p != "a" {
                    return Err(syn::Error::new(
                        p.span(),
                        "`on_add`'s parameter is the self id `a` (write `on_add(a)`)",
                    ));
                }
                let body: TokenStream2 = {
                    let content;
                    braced!(content in input);
                    content.parse()?
                };
                Ok(Clause::OnAdd { body, span })
            }
            "compact" => {
                let body: TokenStream2 = {
                    let content;
                    braced!(content in input);
                    content.parse()?
                };
                Ok(Clause::Compact { body, span })
            }
            "validate" => {
                let variant = if input.peek(Ident) {
                    let v: Ident = input.parse()?;
                    match v.to_string().as_str() {
                        "by" => ValidateVariant::By,
                        "primitive" => ValidateVariant::Primitive,
                        other => {
                            return Err(syn::Error::new(
                                v.span(),
                                format!(
                                    "validate variants are `by` and `primitive` (selected per binding from the arrow's membership facts); found `{other}`"
                                ),
                            ))
                        }
                    }
                } else {
                    ValidateVariant::Plain
                };
                let body: TokenStream2 = {
                    let content;
                    braced!(content in input);
                    content.parse()?
                };
                Ok(Clause::Validate { variant, body, span })
            }
            "membership" => {
                input.parse::<Token![:]>()?;
                let v: Ident = input.parse()?;
                if v != "existence" {
                    return Err(syn::Error::new(
                        v.span(),
                        format!(
                            "membership kinds come from the closed set {{existence}}; found `{}`",
                            v
                        ),
                    ));
                }
                Ok(Clause::MembershipExistence { span })
            }
            "requires" => {
                input.parse::<Token![:]>()?;
                let items = parse_annotation_items(input)?;
                Ok(Clause::Requires { items, span })
            }
            "contract" => {
                input.parse::<Token![:]>()?;
                let items = parse_annotation_items(input)?;
                Ok(Clause::Contract { items, span })
            }
            "iteration_order" => {
                input.parse::<Token![:]>()?;
                let value: Ident = input.parse()?;
                match value.to_string().as_str() {
                    "reverse_insertion" | "forward_insertion" => {
                        Ok(Clause::IterationOrder { value, span })
                    }
                    other => Err(syn::Error::new(
                        value.span(),
                        format!(
                            "iteration orders come from the closed set {{reverse_insertion, forward_insertion}}; found `{other}`"
                        ),
                    )),
                }
            }
            "phase" => {
                input.parse::<Token![:]>()?;
                let items = parse_phase_items(input)?;
                Ok(Clause::Phase { items, span })
            }
            other => Err(syn::Error::new(
                span,
                format!(
                    "unknown clause `{other}` in backing declaration (expected one of: `on A`/`on B`/`on store`, `ctor_arg`, `read(a)`, `lookup(a, k)`, `iter Walk`, `iterate(a)`, `advance(w)`, `on_insert(..)`, `on_add(a)`, `compact`, `validate [by|primitive]`, `membership: existence`, `requires: ..`, `contract: ..`, `iteration_order: ..`, `phase: ..`)"
                ),
            )),
        }
    }
}

/// The pinned clause inventory (round 4) as enforcement: legality per
/// cardinality, the repetition rule (`on` once per (target, label) pair, the
/// validate family once per variant, everything else once), and the closed
/// annotation vocabularies.
fn validate_inventory(decl: &BackingDecl) -> syn::Result<()> {
    let card_name = match decl.card {
        BackingCard::One => "one",
        BackingCard::Many => "many",
        BackingCard::Keyed => "key(K)",
    };

    // ---- repetition ----
    let mut seen_on: Vec<(OnTarget, Option<PhaseLabel>)> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let mut seen_validate: Vec<ValidateVariant> = Vec::new();
    for c in &decl.clauses {
        match c {
            Clause::On { target, label, span, .. } => {
                // Phase labels key the type-state store; they are legal
                // only on a phased backing (one carrying `phase:`
                // annotations), same rule `compact` rides on.
                if *label == Some(PhaseLabel::Build) && !decl.is_phased() {
                    return Err(syn::Error::new(
                        *span,
                        format!(
                            "`on store build` is legal only on a phased backing (one carrying `phase:` annotations); backing `{}` is unphased",
                            decl.name
                        ),
                    ));
                }
                if *label == Some(PhaseLabel::Frozen) && !decl.is_phased() {
                    return Err(syn::Error::new(
                        *span,
                        format!(
                            "`on store frozen` is legal only on a phased backing (one carrying `phase:` annotations); backing `{}` is unphased",
                            decl.name
                        ),
                    ));
                }
                let key = (*target, *label);
                if seen_on.contains(&key) {
                    let label_txt = label
                        .map(|l| match l {
                            PhaseLabel::Build => " build",
                            PhaseLabel::Frozen => " frozen",
                        })
                        .unwrap_or("");
                    return Err(syn::Error::new(
                        *span,
                        format!(
                            "repeated `on {}{}` clause in backing `{}`; each contribution site appears once",
                            target.name(),
                            label_txt,
                            decl.name
                        ),
                    ));
                }
                seen_on.push(key);
            }
            Clause::Validate { variant, span, .. } => {
                if seen_validate.contains(variant) {
                    return Err(syn::Error::new(
                        *span,
                        format!(
                            "repeated `validate {:?}` clause in backing `{}`; each variant appears once",
                            variant, decl.name
                        ),
                    ));
                }
                seen_validate.push(*variant);
            }
            kind => {
                let name = kind.kind_name();
                if seen.contains(&name) {
                    return Err(syn::Error::new(
                        c_span(kind),
                        format!(
                            "repeated `{name}` clause in backing `{}`; a clause repeats only as its selector distinguishes it",
                            decl.name
                        ),
                    ));
                }
                seen.push(name);
            }
        }
    }

    // ---- legality per cardinality (the pinned table) ----
    let reject = |span: proc_macro2::Span, what: &str| -> syn::Result<()> {
        Err(syn::Error::new(
            span,
            format!(
                "`{what}` is not a legal clause for a `{card_name}` backing (see the clause table in plans/milestone-3)"
            ),
        ))
    };
    for c in &decl.clauses {
        match c {
            Clause::CtorArg { span } => {
                if decl.card != BackingCard::One {
                    return reject(*span, "ctor_arg");
                }
            }
            Clause::Read { span, .. } => {
                if decl.card != BackingCard::One {
                    return reject(*span, "read(a)");
                }
            }
            Clause::Lookup { span, .. } => {
                if decl.card != BackingCard::Keyed {
                    return reject(*span, "lookup(a, k)");
                }
            }
            Clause::Iter { span, .. }
            | Clause::Iterate { span, .. }
            | Clause::Advance { span, .. } => {
                if decl.card != BackingCard::Many {
                    return reject(*span, c.kind_name());
                }
            }
            Clause::OnInsert { keyed, span, .. } => {
                if decl.card == BackingCard::One {
                    return reject(*span, "on_insert");
                }
                if *keyed && decl.card != BackingCard::Keyed {
                    return reject(*span, "on_insert(a, k, b)");
                }
                if !*keyed && decl.card == BackingCard::Keyed {
                    return reject(*span, "on_insert(a, b)");
                }
            }
            Clause::Compact { span, .. } => {
                if decl.card != BackingCard::Many {
                    return reject(*span, "compact");
                }
                if !decl.is_phased() {
                    return Err(syn::Error::new(
                        *span,
                        format!(
                            "`compact` is legal only on a phased backing (one carrying `phase:` annotations); backing `{}` is unphased",
                            decl.name
                        ),
                    ));
                }
            }
            Clause::MembershipExistence { span } => {
                if decl.card != BackingCard::Many {
                    return reject(*span, "membership: existence");
                }
            }
            Clause::IterationOrder { span, .. } => {
                if decl.card != BackingCard::Many {
                    return reject(*span, "iteration_order");
                }
            }
            _ => {}
        }
    }

    // ---- reserved-set shadowing (hygiene caveat 3 as a grammar rule) ----
    // The role rewrite is textual and unconditional, so a body that binds
    // a reserved ident (`let rec = ..`, `fn handle(..)`) would be silently
    // captured. Reject at declaration time, quoting the closed set.
    fn clause_body(c: &Clause) -> Option<&proc_macro2::TokenStream> {
        match c {
            Clause::Read { body, .. }
            | Clause::Lookup { body, .. }
            | Clause::Iterate { body, .. }
            | Clause::Advance { body, .. }
            | Clause::OnInsert { body, .. }
            | Clause::OnAdd { body, .. }
            | Clause::Compact { body, .. }
            | Clause::Validate { body, .. } => Some(body),
            _ => None,
        }
    }
    for c in &decl.clauses {
        if let Some(body) = clause_body(c) {
            if let Some(hit) = crate::roles::check_reserved_bindings(body) {
                return Err(syn::Error::new(
                    c_span(c),
                    format!(
                        "authored body for `{}` in backing `{}` binds reserved ident `{hit}` — the role vocabulary is closed and rewritten unconditionally (reserved: {}); rename the local",
                        c.kind_name(),
                        decl.name,
                        crate::roles::RESERVED_BINDINGS.join(", "),
                    ),
                ));
            }
        }
    }

    // ---- mandatory clauses (the pinned table's `req` rows) ----
    let has = |f: fn(&Clause) -> bool| decl.has(f);
    match decl.card {
        BackingCard::One => {
            if !has(|c| matches!(c, Clause::Read { .. })) {
                return Err(syn::Error::new(
                    decl.name.span(),
                    format!(
                        "a `one` backing must declare the access protocol `read(a) {{ .. }}`; backing `{}` has none",
                        decl.name
                    ),
                ));
            }
        }
        BackingCard::Many => {
            for (present, what) in [
                (has(|c| matches!(c, Clause::Iter { .. })), "iter Walk { .. }"),
                (has(|c| matches!(c, Clause::Iterate { .. })), "iterate(a) { .. }"),
                (has(|c| matches!(c, Clause::Advance { .. })), "advance(w) { .. }"),
            ] {
                if !present {
                    return Err(syn::Error::new(
                        decl.name.span(),
                        format!(
                            "a `many` backing must declare `{what}` (keyed traits carry no GAT; `one` arrows read through `read(a)`); backing `{}` has none",
                            decl.name
                        ),
                    ));
                }
            }
        }
        BackingCard::Keyed => {
            if !has(|c| matches!(c, Clause::Lookup { .. })) {
                return Err(syn::Error::new(
                    decl.name.span(),
                    format!(
                        "a keyed backing must declare the access protocol `lookup(a, k) {{ .. }}`; backing `{}` has none",
                        decl.name
                    ),
                ));
            }
        }
    }

    // ---- closed annotation vocabularies ----
    for c in &decl.clauses {
        match c {
            Clause::Requires { items, .. } => {
                for item in items {
                    match item.to_string().as_str() {
                        "hosts_arena" | "direct_records" => {}
                        other => {
                            return Err(syn::Error::new(
                                item.span(),
                                format!(
                                    "applicability predicates come from the closed set {{hosts_arena, direct_records}}; found `{other}` (extending the vocabulary is a macros-crate change)"
                                ),
                            ))
                        }
                    }
                }
            }
            Clause::Contract { items, .. } => {
                for item in items {
                    if item != "exclusive_membership" {
                        return Err(syn::Error::new(
                            item.span(),
                            format!(
                                "contract annotations come from the closed set {{exclusive_membership}}; found `{}`",
                                item
                            ),
                        ));
                    }
                }
            }
            Clause::Phase { items, span } => {
                if items.is_empty() {
                    return Err(syn::Error::new(
                        *span,
                        "`phase:` needs at least one property",
                    ));
                }
                for item in items {
                    match (item.name.to_string().as_str(), &item.value) {
                        ("append_only_build", None) | ("requires_compaction_at_boundary", None) => {}
                        ("freeze_scope", Some(v)) if v == "per_store" => {}
                        ("read_precondition", Some(v)) if v == "post_compaction" => {}
                        ("freeze_scope", _) => {
                            return Err(syn::Error::new(
                                item.name.span(),
                                "`freeze_scope` takes its value from the closed set {per_store} (write `freeze_scope: per_store`)",
                            ))
                        }
                        ("read_precondition", _) => {
                            return Err(syn::Error::new(
                                item.name.span(),
                                "`read_precondition` takes its value from the closed set {post_compaction} (write `read_precondition: post_compaction`)",
                            ))
                        }
                        (other, _) => {
                            return Err(syn::Error::new(
                                item.name.span(),
                                format!(
                                    "phase properties come from the closed set {{append_only_build, requires_compaction_at_boundary, freeze_scope: per_store, read_precondition: post_compaction}}; found `{other}`"
                                ),
                            ))
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn c_span(c: &Clause) -> proc_macro2::Span {
    match c {
        Clause::On { span, .. }
        | Clause::CtorArg { span }
        | Clause::Read { span, .. }
        | Clause::Lookup { span, .. }
        | Clause::Iter { span, .. }
        | Clause::Iterate { span, .. }
        | Clause::Advance { span, .. }
        | Clause::OnInsert { span, .. }
        | Clause::OnAdd { span, .. }
        | Clause::Compact { span, .. }
        | Clause::Validate { span, .. }
        | Clause::MembershipExistence { span }
        | Clause::Requires { span, .. }
        | Clause::Contract { span, .. }
        | Clause::IterationOrder { span, .. }
        | Clause::Phase { span, .. } => *span,
    }
}

// ---------------------------------------------------------------------------
// Re-emission (round-trips through `BackingDecl::parse`)
// ---------------------------------------------------------------------------

fn tokens_or_empty(ts: &TokenStream2) -> TokenStream2 {
    // Braces always: the parser expects a block even when the body is empty,
    // and an empty `{}` re-parses to an empty body.
    let inner = ts.clone();
    quote!({ #inner })
}

fn phase_label_tokens(l: Option<PhaseLabel>) -> TokenStream2 {
    match l {
        Some(PhaseLabel::Build) => quote!(build),
        Some(PhaseLabel::Frozen) => quote!(frozen),
        None => TokenStream2::new(),
    }
}

/// Re-emit a parsed declaration in the shared courier grammar.
pub fn backing_decl_tokens(decl: &BackingDecl) -> TokenStream2 {
    let name = &decl.name;
    let card = match decl.card {
        BackingCard::One => quote!(one),
        BackingCard::Many => quote!(many),
        BackingCard::Keyed => quote!(key(K)),
    };
    let clauses = decl.clauses.iter().map(|c| match c {
        Clause::On { target, label, fields, .. } => {
            let t = match target {
                OnTarget::A => quote!(A),
                OnTarget::B => quote!(B),
                OnTarget::Store => quote!(store),
            };
            let l = phase_label_tokens(*label);
            let fields = fields.iter().map(|f| {
                let logical = &f.logical;
                let ty = &f.ty;
                let formula = match f.formula {
                    NameFormula::Bare => TokenStream2::new(),
                    NameFormula::Arrow => quote!(= arrow),
                    NameFormula::Stem => quote!(= stem),
                    NameFormula::Suffix => quote!(= suffix),
                };
                quote!(#logical: #ty #formula,)
            });
            quote!(on #t #l { #(#fields)* })
        }
        Clause::CtorArg { .. } => quote!(ctor_arg),
        Clause::Read { body, .. } => {
            let b = tokens_or_empty(body);
            quote!(read(a) #b)
        }
        Clause::Lookup { body, .. } => {
            let b = tokens_or_empty(body);
            quote!(lookup(a, k) #b)
        }
        Clause::Iter { walk, fields, .. } => {
            let fields = fields.iter().map(|(f, ty)| quote!(#f: #ty,));
            quote!(iter #walk { #(#fields)* })
        }
        Clause::Iterate { body, .. } => {
            let b = tokens_or_empty(body);
            quote!(iterate(a) #b)
        }
        Clause::Advance { body, .. } => {
            let b = tokens_or_empty(body);
            quote!(advance(w) #b)
        }
        Clause::OnInsert { keyed, body, .. } => {
            let b = tokens_or_empty(body);
            if *keyed {
                quote!(on_insert(a, k, b) #b)
            } else {
                quote!(on_insert(a, b) #b)
            }
        }
        Clause::OnAdd { body, .. } => {
            let b = tokens_or_empty(body);
            quote!(on_add(a) #b)
        }
        Clause::Compact { body, .. } => {
            let b = tokens_or_empty(body);
            quote!(compact #b)
        }
        Clause::Validate { variant, body, .. } => {
            let b = tokens_or_empty(body);
            match variant {
                ValidateVariant::Plain => quote!(validate #b),
                ValidateVariant::By => quote!(validate by #b),
                ValidateVariant::Primitive => quote!(validate primitive #b),
            }
        }
        Clause::MembershipExistence { .. } => quote!(membership: existence),
        Clause::Requires { items, .. } => {
            quote!(requires: #(#items),*)
        }
        Clause::Contract { items, .. } => {
            quote!(contract: #(#items),*)
        }
        Clause::IterationOrder { value, .. } => quote!(iteration_order: #value),
        Clause::Phase { items, .. } => {
            let items = items.iter().map(|i| {
                let n = &i.name;
                match &i.value {
                    Some(v) => quote!(#n: #v),
                    None => quote!(#n),
                }
            });
            quote!(phase: #(#items),*)
        }
    });
    quote!(#name: A -> B, #card { #(#clauses)* })
}

/// A backing declaration riding with its declaring package — the same
/// declaring-crate resolver carriage as schemas: `("pkg", { decl })`.
#[derive(Clone)]
pub struct PackagedBacking {
    pub pkg: String,
    pub decl: BackingDecl,
}

impl Parse for PackagedBacking {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        syn::parenthesized!(content in input);
        let pkg: syn::LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        let body;
        braced!(body in content);
        Ok(PackagedBacking { pkg: pkg.value(), decl: body.parse()? })
    }
}

pub fn packaged_backing_tokens(p: &PackagedBacking) -> TokenStream2 {
    let pkg = &p.pkg;
    let decl = backing_decl_tokens(&p.decl);
    quote!((#pkg, { #decl }))
}

// ToTokens for re-emission of types inside contributions.
impl ToTokens for BackingDecl {
    fn to_tokens(&self, ts: &mut TokenStream2) {
        ts.extend(backing_decl_tokens(self));
    }
}

// ---------------------------------------------------------------------------
// Round-trip and rejection tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_decl(src: &str) -> BackingDecl {
        syn::parse_str(src).expect("decl parses")
    }

    /// Structural signature comparing two parses (spans differ in flight).
    fn decl_sig(d: &BackingDecl) -> String {
        let mut s = format!("{}:{:?}:", d.name, d.card);
        for c in &d.clauses {
            s.push_str(c.kind_name());
            match c {
                Clause::On { target, label, fields, .. } => {
                    s.push_str(&format!(
                        "({:?}{:?}:[{}])",
                        target,
                        label,
                        fields
                            .iter()
                            .map(|f| format!(
                                "{}:{}:{:?}",
                                f.logical,
                                f.ty.to_token_stream(),
                                f.formula
                            ))
                            .collect::<Vec<_>>()
                            .join(",")
                    ));
                }
                Clause::Iter { walk, fields, .. } => {
                    s.push_str(&format!(
                        "({}:[{}])",
                        walk,
                        fields
                            .iter()
                            .map(|(f, t)| format!("{}:{}", f, t.to_token_stream()))
                            .collect::<Vec<_>>()
                            .join(",")
                    ));
                }
                _ => s.push('!'),
            }
            s.push(';');
        }
        s
    }

    fn assert_roundtrip(src: &str) {
        let once = parse_decl(src);
        let emitted = backing_decl_tokens(&once);
        let twice = syn::parse2::<BackingDecl>(emitted.clone())
            .unwrap_or_else(|e| panic!("reparse failed: {e}\nemitted: {emitted}"));
        assert_eq!(decl_sig(&once), decl_sig(&twice), "src: {src}");
        let reemitted = backing_decl_tokens(&twice);
        assert_eq!(
            emitted.to_string(),
            reemitted.to_string(),
            "idempotent emission; src: {src}"
        );
    }

    fn assert_rejected(src: &str, needle: &str) {
        let err = syn::parse_str::<BackingDecl>(src).err().expect("expected a rejection");
        let msg = err.to_string();
        assert!(
            msg.contains(needle),
            "expected message containing `{needle}`, got: {msg}"
        );
    }

    #[test]
    fn roundtrip_field() {
        assert_roundtrip(
            "field: A -> B, one { on A { value: BId = arrow } ctor_arg read(a) { rec(a).value } validate { for ra in records(A) { let v = rec(ra).value; } } }",
        );
    }

    #[test]
    fn roundtrip_arena() {
        assert_roundtrip(
            "arena: A -> B, many { membership: existence iter Walk { cur: u32, len: u32 } iterate(a) { Walk { cur: 0, len: b_arena_of(a).len() } } advance(w) { (w.cur < w.len).then(|| handle(w.cur)) } requires: hosts_arena }",
        );
    }

    #[test]
    fn roundtrip_intrusive() {
        assert_roundtrip(
            "intrusive_list: A -> B, many { on A { first: Option<BId> = stem } on B { next: Option<BId> = stem } iter Walk { cur: Option<BId> } on_insert(a, b) { rec(b).next = rec(a).first; rec(a).first = Some(b); } iterate(a) { Walk { cur: rec(a).first } } advance(w) { let id = w.cur?; w.cur = rec(id).next; handle(id) } iteration_order: reverse_insertion contract: exclusive_membership requires: direct_records validate by { for ra in records(A) { } } validate primitive { for ra in records(A) { } } }",
        );
    }

    #[test]
    fn roundtrip_hash_index() {
        assert_roundtrip(
            "hash_index: A -> B, key(K) { on A { index: ::std::collections::HashMap<KId, BId> = suffix } on_insert(a, k, b) { map_from(a).insert(k, b); } lookup(a, k) { map_from(a).get(&k).copied() } validate { for (owner, m) in maps(A) { for (k, v) in m.iter() { } } } }",
        );
    }

    #[test]
    fn roundtrip_csr() {
        assert_roundtrip(
            "csr_adjacency: A -> B, many { on store build { buckets: Vec<Vec<BId>> } on store frozen { offsets: Vec<u32>, adj: Vec<BId> } on_add(a) { store.buckets[idx(a)].push(store.__last_b); } on_insert(a, b) { store.buckets[idx(a)].push(b); } compact { } iter Walk { lo: u32, hi: u32 } iterate(a) { Walk { lo: store.offsets[idx(a)], hi: store.offsets[idx(a) + 1] } } advance(w) { handle(w.lo) } iteration_order: forward_insertion phase: append_only_build, requires_compaction_at_boundary, freeze_scope: per_store, read_precondition: post_compaction requires: direct_records validate { } }",
        );
    }

    #[test]
    fn roundtrip_pkg() {
        let src = r#"("metagraph-exper", { field: A -> B, one { on A { value: BId = arrow } read(a) { rec(a).value } } })"#;
        let once: PackagedBacking = syn::parse_str(src).expect("parse");
        assert_eq!(once.pkg, "metagraph-exper");
        let emitted = packaged_backing_tokens(&once);
        let twice: PackagedBacking = syn::parse2(emitted.clone()).expect("reparse");
        assert_eq!(decl_sig(&once.decl), decl_sig(&twice.decl));
        assert_eq!(
            emitted.to_string(),
            packaged_backing_tokens(&twice).to_string(),
            "idempotent"
        );
    }

    // ---- one rejection fixture per closed set (step 1 acceptance) ----

    #[test]
    fn reject_unknown_clause() {
        assert_rejected(
            "mine: A -> B, many { iter Walk { cur: u32 } iterate(a) { Walk { cur: 0 } } advance(w) { handle(w.cur) } frobnicate }",
            "unknown clause `frobnicate`",
        );
    }

    #[test]
    fn reject_bad_store_label() {
        assert_rejected(
            "mine: A -> B, many { on store archival { buckets: Vec<BId> } iter Walk { cur: u32 } iterate(a) { Walk { cur: 0 } } advance(w) { handle(w.cur) } }",
            "closed set {build, frozen}",
        );
    }

    #[test]
    fn reject_unknown_contract_item() {
        assert_rejected(
            "mine: A -> B, many { iter Walk { cur: u32 } iterate(a) { Walk { cur: 0 } } advance(w) { handle(w.cur) } contract: shared_membership }",
            "closed set {exclusive_membership}",
        );
    }

    #[test]
    fn reject_unknown_phase_property() {
        assert_rejected(
            "mine: A -> B, many { iter Walk { cur: u32 } iterate(a) { Walk { cur: 0 } } advance(w) { handle(w.cur) } phase: append_only_build, speculative }",
            "phase properties come from the closed set",
        );
    }

    #[test]
    fn reject_bad_iteration_order_value() {
        assert_rejected(
            "mine: A -> B, many { iter Walk { cur: u32 } iterate(a) { Walk { cur: 0 } } advance(w) { handle(w.cur) } iteration_order: insertion }",
            "closed set {reverse_insertion, forward_insertion}",
        );
    }

    #[test]
    fn reject_repeated_clause() {
        assert_rejected(
            "mine: A -> B, many { iter Walk { cur: u32 } iterate(a) { Walk { cur: 0 } } advance(w) { handle(w.cur) } iteration_order: forward_insertion iteration_order: reverse_insertion }",
            "repeated `iteration_order` clause",
        );
    }

    #[test]
    fn reject_repeated_on_site() {
        assert_rejected(
            "mine: A -> B, many { on A { x: BId } on A { y: BId } iter Walk { cur: u32 } iterate(a) { Walk { cur: 0 } } advance(w) { handle(w.cur) } }",
            "repeated `on A` clause",
        );
    }

    #[test]
    fn reject_wrong_cardinality_clauses() {
        assert_rejected("mine: A -> B, one { read(a) { rec(a).value } on_insert(a, b) { } }", "not a legal clause for a `one` backing");
        assert_rejected("mine: A -> B, one { read(a) { rec(a).value } iter Walk { cur: u32 } }", "not a legal clause for a `one` backing");
        assert_rejected("mine: A -> B, many { iter Walk { cur: u32 } iterate(a) { Walk { cur: 0 } } advance(w) { handle(w.cur) } read(a) { handle(a) } }", "not a legal clause for a `many` backing");
        assert_rejected("mine: A -> B, key(K) { lookup(a, k) { map_from(a).get(&k).copied() } on_insert(a, b) { } }", "not a legal clause for a `key(K)` backing");
    }

    #[test]
    fn reject_compact_on_unphased() {
        assert_rejected(
            "mine: A -> B, many { iter Walk { cur: u32 } iterate(a) { Walk { cur: 0 } } advance(w) { handle(w.cur) } compact { } }",
            "legal only on a phased backing",
        );
    }

    #[test]
    fn reject_labeled_store_on_unphased() {
        assert_rejected(
            "mine: A -> B, many { on store build { buckets: Vec<BId> } iter Walk { cur: u32 } iterate(a) { Walk { cur: 0 } } advance(w) { handle(w.cur) } }",
            "`on store build` is legal only on a phased backing",
        );
        assert_rejected(
            "mine: A -> B, many { on store frozen { adj: Vec<BId> } iter Walk { cur: u32 } iterate(a) { Walk { cur: 0 } } advance(w) { handle(w.cur) } }",
            "`on store frozen` is legal only on a phased backing",
        );
    }

    #[test]
    fn reject_missing_access_protocol() {
        assert_rejected("mine: A -> B, one { ctor_arg }", "access protocol `read(a) { .. }`");
        assert_rejected("mine: A -> B, many { iter Walk { cur: u32 } }", "must declare");
        assert_rejected("mine: A -> B, key(K) { on_insert(a, k, b) { } }", "access protocol `lookup(a, k) { .. }`");
    }

    #[test]
    fn reject_unknown_predicate() {
        assert_rejected(
            "mine: A -> B, many { iter Walk { cur: u32 } iterate(a) { Walk { cur: 0 } } advance(w) { handle(w.cur) } requires: hosts_everything }",
            "closed set {hosts_arena, direct_records}",
        );
    }

    #[test]
    fn reject_bad_phase_value() {
        assert_rejected(
            "mine: A -> B, many { iter Walk { cur: u32 } iterate(a) { Walk { cur: 0 } } advance(w) { handle(w.cur) } phase: freeze_scope: per_graph }",
            "closed set {per_store}",
        );
    }
}
