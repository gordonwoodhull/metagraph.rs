//! Flat (non-composed) store codegen — the codegen stage of
//! `__implement_gen!`, entered once every registration courier has
//! published its metadata tuple. All codegen lives here, in real Rust, not
//! tt-munching.
//!
//! Layout rules (plans/milestone-1):
//! - every non-root entity gets a `Vec` arena, an id newtype, a record
//!   struct, and a `Copy` handle, unconditionally;
//! - the root-marked entity owns nothing but its handle and the root
//!   accessor — no root id, record, arena, or count (spike 10);
//! - trait identity (method, GAT, associated types) comes from the
//!   registration metadata, never the instantiation's local alias;
//! - handles hand-write `Eq`/`Hash` forwarding to (store pointer, id);
//! - backing mutations live in named doc-hidden hook functions; `add_*`
//!   constructors are thin orchestration over them.

use std::collections::HashMap;

use proc_macro2::{Ident as Ident2, TokenStream as TokenStream2};
use quote::{format_ident, quote};

use crate::grammar::{
    arena_fld, data_ty, id_ty, iter_ty, link_stem, marker_ty, path_tokens, snake, Binding,
    RootMeta,
};
use crate::{resolve_schema_path, ChainConfig};

/// A resolved arrow: declaration plus its root's metadata and backing.
struct ArrowInfo<'a> {
    decl: &'a crate::grammar::ArrowDecl,
    meta: &'a RootMeta,
    /// `None` for derived (`:= union`) arrows — never bound to a backing.
    backing: Option<String>,
}

pub(crate) fn expand_flat(config: &ChainConfig) -> syn::Result<TokenStream2> {
    let schema = &config.schema;
    let store = &config.store;

    // metas[i] belongs to the i-th unique root (first-occurrence order over
    // the schema's arrows) — the order the resolution chain queued them in.
    let unique_roots = crate::unique_roots(schema);
    if unique_roots.len() != config.metas.len() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
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

    let root_entity = schema
        .entities
        .iter()
        .find(|e| e.root)
        .expect("pattern! validated exactly one root-marked entity");
    let elements: Vec<&crate::grammar::EntityDecl> =
        schema.entities.iter().filter(|e| !e.root).collect();
    let is_root = |name: &proc_macro2::Ident| root_entity.name == *name;

    // ---- resolve arrows: backing bindings against root cardinalities ----
    let binding_of = |name: &str| -> Option<&Binding> {
        config.bindings.iter().find(|b| b.arrow == name)
    };
    let mut infos: Vec<ArrowInfo> = Vec::new();
    for decl in &schema.arrows {
        let meta = meta_of
            .get(&path_tokens(decl.inst.as_ref().unwrap()).to_string())
            .expect("every arrow's root was queued");
        let backing = if decl.union.is_some() {
            None
        } else if let Some(b) = binding_of(&decl.name.to_string()) {
            let b = b.backing.to_string();
            // Wrong-cardinality check, both directions. Spans have degraded
            // to the call site; quote the offending tokens in the message.
            let ok = match b.as_str() {
                "field" => !meta.many,
                "arena" | "intrusive_list" => meta.many,
                _ => true, // unknown backings get their own error below
            };
            if !ok {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "backing `{b}` has wrong cardinality for arrow `{}` (a `{}` arrow)",
                        decl.name,
                        if meta.many { "many" } else { "one" }
                    ),
                ));
            }
            Some(b)
        } else if meta.many {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "missing backing binding for `many` arrow `{}` (`arena` vs `intrusive_list` is an explicit choice)",
                    decl.name
                ),
            ));
        } else {
            Some("field".to_string()) // `one` arrows default to `field`
        };
        infos.push(ArrowInfo { decl, meta, backing });
    }
    for b in &config.bindings {
        if !["arena", "field", "intrusive_list"].contains(&b.backing.to_string().as_str()) {
            return Err(syn::Error::new(
                b.arrow.span(),
                format!(
                    "unknown backing `{}` for arrow `{}` (M1 built-ins: `arena`, `field`, `intrusive_list`)",
                    b.backing, b.arrow
                ),
            ));
        }
    }

    // `by <det>`: the determining arrow must be total (its root says `one`).
    for info in &infos {
        let Some(det_name) = &info.decl.by else { continue };
        let det = infos.iter().find(|i| &i.decl.name == det_name).ok_or_else(|| {
            // pattern! already guarantees the determinant is declared and
            // opposite; reaching here means it is not instantiable.
            syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`by {}` does not name a resolvable arrow", det_name),
            )
        })?;
        if det.meta.many {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "`by {}` names arrow `{}`, which is not total (`one`) — membership would be undefined",
                    det_name, det.decl.name
                ),
            ));
        }
    }

    // Backings that need a self record cannot be sourced at the root-marked
    // entity: it owns no record to hold their fields.
    for info in &infos {
        let needs_self_record = matches!(info.backing.as_deref(), Some("field" | "intrusive_list"));
        if needs_self_record && is_root(&info.decl.source) {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "arrow `{}` is sourced at the root-marked entity `{}`; the `{}` backing needs a field on the self record, and the root owns no record (it *is* the store)",
                    info.decl.name, info.decl.source,
                    info.backing.as_deref().unwrap_or("?")
                ),
            ));
        }
    }

    // ---- record fields per element entity ----
    let mut field_decls: HashMap<String, Vec<TokenStream2>> =
        elements.iter().map(|e| (e.name.to_string(), Vec::new())).collect();
    for info in &infos {
        let d = info.decl;
        match info.backing.as_deref() {
            Some("field") => {
                let f = &d.name;
                let tid = id_ty(&d.target);
                field_decls.get_mut(&d.source.to_string()).unwrap().push(quote!(
                    #[doc = concat!("field backing of arrow `", stringify!(#f), "`")]
                    #f: #tid,
                ));
            }
            Some("intrusive_list") => {
                let arrow_doc_name = &d.name;
                let head = format_ident!("first_{}", link_stem(&d.name));
                let next = format_ident!("next_{}", link_stem(&d.name));
                let tid = id_ty(&d.target);
                field_decls.get_mut(&d.source.to_string()).unwrap().push(quote!(
                    #[doc = concat!("intrusive-list head of arrow `", stringify!(#arrow_doc_name), "`")]
                    #head: ::core::option::Option<#tid>,
                ));
                field_decls.get_mut(&d.target.to_string()).unwrap().push(quote!(
                    #[doc = concat!("intrusive-list link of arrow `", stringify!(#arrow_doc_name), "`")]
                    #next: ::core::option::Option<#tid>,
                ));
            }
            _ => {}
        }
    }

    let mut out = TokenStream2::new();

    // ---- entity kit: id newtype + record, non-root entities only ----
    for e in &elements {
        let ent_doc_name = &e.name;
        let idt = id_ty(&e.name);
        let dat = data_ty(&e.name);
        let fields = &field_decls[&e.name.to_string()];
        out.extend(quote! {
            #[doc = concat!("Index of a `", stringify!(#ent_doc_name), "` in its arena.")]
            #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
            pub struct #idt(pub u32);
            #[doc = concat!("Record of a `", stringify!(#ent_doc_name), "`.")]
            #[derive(Debug)]
            struct #dat { #(#fields)* }
        });
    }

    // ---- constructors + named insert hooks ----
    let mut ctor_impls = TokenStream2::new();
    let mut hook_impls = TokenStream2::new();
    for e in &elements {
        let ent_doc_name = &e.name;
        // One argument per total `one` arrow originating at this entity
        // (all bound `one` arrows are field-backed; total by construction).
        let args: Vec<&ArrowInfo> = infos
            .iter()
            .filter(|i| i.backing.as_deref() == Some("field") && i.decl.source == e.name)
            .collect();
        let arg_defs = args.iter().map(|i| {
            let n = &i.decl.name;
            let tid = id_ty(&i.decl.target);
            quote!(#n: #tid,)
        });
        // Record literal: field args move in; intrusive links start empty —
        // the hooks below relink on insert.
        let mut record_fields = TokenStream2::new();
        for info in &infos {
            let d = info.decl;
            match info.backing.as_deref() {
                Some("field") if d.source == e.name => {
                    let n = &d.name;
                    record_fields.extend(quote!(#n,));
                }
                Some("intrusive_list") if d.source == e.name => {
                    let head = format_ident!("first_{}", link_stem(&d.name));
                    record_fields.extend(quote!(#head: ::core::option::Option::None,));
                }
                Some("intrusive_list") if d.target == e.name => {
                    let next = format_ident!("next_{}", link_stem(&d.name));
                    record_fields.extend(quote!(#next: ::core::option::Option::None,));
                }
                _ => {}
            }
        }
        // `by`-derived many arrows whose determining `one` arrow originates
        // here: the new record becomes that arrow's first member, inserted
        // by the arrow's named hook reading the determining argument.
        let mut hook_calls = TokenStream2::new();
        for info in &infos {
            let Some(det_name) = &info.decl.by else { continue };
            let det = args.iter().find(|i| &i.decl.name == det_name);
            if let Some(det) = det {
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
                #hook_calls
                id
            }
        });
    }

    // Named insert hooks: the single write path for list maintenance.
    // Push-front (reverse insertion order) is part of the backing's
    // contract — the test suite's traversal orders depend on it.
    for info in &infos {
        if info.backing.as_deref() != Some("intrusive_list") {
            continue;
        }
        let d = info.decl;
        let arrow_doc_name = &d.name;
        let hook = format_ident!("__insert_{}", d.name);
        let head = format_ident!("first_{}", link_stem(&d.name));
        let next = format_ident!("next_{}", link_stem(&d.name));
        let sid = id_ty(&d.source);
        let tid = id_ty(&d.target);
        let s_arena = arena_fld(&d.source);
        let t_arena = arena_fld(&d.target);
        hook_impls.extend(quote! {
            #[doc(hidden)]
            #[doc = concat!("intrusive_list insert hook of arrow `", stringify!(#arrow_doc_name), "`: push `member` onto the front of `self_id`'s list")]
            fn #hook(&mut self, self_id: #sid, member: #tid) {
                let next = self.#s_arena[self_id.0 as usize].#head;
                self.#t_arena[member.0 as usize].#next = next;
                self.#s_arena[self_id.0 as usize].#head = ::core::option::Option::Some(member);
            }
        });
        // Primitive membership (no `by`): the builder gets an explicit
        // insert operation; `by`-derived membership can never contradict
        // the determining field, so those arrows generate no insert op.
        if d.by.is_none() {
            let insert_name = format_ident!("insert_{}", d.name);
            ctor_impls.extend(quote! {
                #[doc = concat!("Insert `member` into `self_id`'s `", stringify!(#arrow_doc_name), "` list (primitive membership).")]
                pub fn #insert_name(&mut self, self_id: #sid, member: #tid) {
                    self.#hook(self_id, member);
                }
            });
        }
    }

    // ---- store struct ----
    let arena_fields = elements.iter().map(|e| {
        let f = arena_fld(&e.name);
        let dat = data_ty(&e.name);
        quote!(#f: ::std::vec::Vec<#dat>,)
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
    let root_snake = Ident2::new(&snake(&root_entity.name.to_string()), root_entity.name.span());
    let root_handle = &root_entity.name;
    let store_pattern_name = config
        .pattern_path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default();
    out.extend(quote! {
        #[doc = concat!("Store generated by `implement!` for pattern `", #store_pattern_name, "`.")]
        #[derive(Default, Debug)]
        pub struct #store {
            #(#arena_fields)*
        }

        impl #store {
            #[doc = "Empty store."]
            pub fn new() -> Self {
                Self::default()
            }

            #ctor_impls

            #hook_impls

            #(#count_fns)*

            #[doc = concat!("The root handle: the \"object\" view of this store (root entity `", stringify!(#root_handle), "`).")]
            pub fn #root_snake(&self) -> #root_handle<'_> {
                #root_handle { store: self }
            }

            #[doc = "Structural validation. M1 has nothing to check (every invariant holds by construction); M2 hangs key and per-backing checks on this boundary, and M4's flush calls it unconditionally."]
            pub fn validate(&self) -> ::core::result::Result<(), ::std::string::String> {
                ::core::result::Result::Ok(())
            }
        }
    });

    // ---- handles ----
    for e in &schema.entities {
        let h = &e.name;
        let (body, id_fn, eq_body, hash_body, dbg_body) = if e.root {
            (
                quote!(store: &'g #store),
                quote!(),
                quote!(::std::ptr::eq(self.store, other.store)),
                quote!((self.store as *const #store).hash(state);),
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
        proc_macro2::Span::call_site(),
    );

    // Iterator expression for a bound sibling many-arrow read through the
    // union arrow's self handle `self_h` (unions chain sibling iterators
    // directly, so generated code never depends on trait imports).
    let iter_expr = |d: &crate::grammar::ArrowDecl, self_h: &proc_macro2::Ident| -> syn::Result<TokenStream2> {
        let it = iter_ty(&d.name);
        match binding_of(&d.name.to_string()).map(|b| b.backing.to_string()) {
            Some(b) if b == "arena" => {
                let tgt_arena = arena_fld(&d.target);
                Ok(quote!(#it { store: self.store, next: 0, len: self.store.#tgt_arena.len() as u32 }))
            }
            Some(b) if b == "intrusive_list" => {
                let head = format_ident!("first_{}", link_stem(&d.name));
                let self_arena = arena_fld(self_h);
                Ok(quote!(#it {
                    store: self.store,
                    next: self.store.#self_arena[self.id.0 as usize].#head,
                }))
            }
            _ => Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("union sibling `{}` is not a bound `many` arrow", d.name),
            )),
        }
    };

    let mut iter_defs = TokenStream2::new();
    let mut trait_impls = TokenStream2::new();

    for info in &infos {
        let d = info.decl;
        let meta = info.meta;
        let arrow_doc_name = &d.name;
        let marker = marker_ty(&d.name);
        let marker_path = quote!(#marker_base_ts::#marker_mod::markers::#marker);
        let trait_path = resolve_schema_path(d.inst.as_ref().unwrap());
        let self_h = &d.source;
        let tgt_h = &d.target;
        let m = &meta.method;
        let assoc = &meta.assoc;
        let it = iter_ty(&d.name);
        let tid = id_ty(&d.target);

        if let Some((ua, ub)) = &d.union {
            let da = schema.arrows.iter().find(|x| x.name == *ua).ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), format!("union names unknown sibling `{ua}`"))
            })?;
            let db = schema.arrows.iter().find(|x| x.name == *ub).ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), format!("union names unknown sibling `{ub}`"))
            })?;
            let ita = iter_ty(&da.name);
            let itb = iter_ty(&db.name);
            let ea = iter_expr(da, self_h)?;
            let eb = iter_expr(db, self_h)?;
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

        match info.backing.as_deref() {
            Some("arena") => {
                let tgt_arena = arena_fld(&d.target);
                let gat = meta.gat.as_ref().unwrap();
                iter_defs.extend(quote! {
                    #[doc = concat!("Iterator of arena-backed arrow `", stringify!(#arrow_doc_name), "`: index range over the target arena.")]
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
                trait_impls.extend(quote! {
                    impl<'g> #trait_path<#marker_path> for #self_h<'g> {
                        type #assoc = #tgt_h<'g>;
                        type #gat<'a> = #it<'g> where Self: 'a;
                        fn #m(&self) -> #it<'g> {
                            #it { store: self.store, next: 0, len: self.store.#tgt_arena.len() as u32 }
                        }
                    }
                });
            }
            Some("intrusive_list") => {
                let head = format_ident!("first_{}", link_stem(&d.name));
                let next = format_ident!("next_{}", link_stem(&d.name));
                let self_arena = arena_fld(self_h);
                let tgt_arena = arena_fld(&d.target);
                let gat = meta.gat.as_ref().unwrap();
                iter_defs.extend(quote! {
                    #[doc = concat!("Iterator of intrusive-backed arrow `", stringify!(#arrow_doc_name), "`: linked-list walk, reverse insertion order.")]
                    pub struct #it<'g> {
                        store: &'g #store,
                        next: ::core::option::Option<#tid>,
                    }
                    impl<'g> ::core::iter::Iterator for #it<'g> {
                        type Item = #tgt_h<'g>;
                        fn next(&mut self) -> ::core::option::Option<#tgt_h<'g>> {
                            let id = self.next?;
                            let data = &self.store.#tgt_arena[id.0 as usize];
                            self.next = data.#next;
                            ::core::option::Option::Some(#tgt_h { store: self.store, id })
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
                                next: self.store.#self_arena[self.id.0 as usize].#head,
                            }
                        }
                    }
                });
            }
            Some("field") => {
                let f = &d.name;
                let self_arena = arena_fld(self_h);
                trait_impls.extend(quote! {
                    impl<'g> #trait_path<#marker_path> for #self_h<'g> {
                        type #assoc = #tgt_h<'g>;
                        fn #m(&self) -> #tgt_h<'g> {
                            #tgt_h {
                                store: self.store,
                                id: self.store.#self_arena[self.id.0 as usize].#f,
                            }
                        }
                    }
                });
            }
            other => {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "backing `{}` is not implemented for arrow `{}`",
                        other.unwrap_or("?"),
                        d.name
                    ),
                ));
            }
        }
    }

    out.extend(iter_defs);
    out.extend(trait_impls);
    Ok(out)
}
