//! Compose-time structural validation (plans/milestone-2, "Structural
//! validation"): the five final-hop checks, run where the courier chain
//! guarantees all member schemas and fusions are in hand. Spans there are
//! honest only for the fusion tokens (which the user wrote in
//! `compose_pattern!`); member-schema tokens have been re-spanned once per
//! hop, so errors quote the offending tokens in the message text.
//!
//! The checks are plain functions over the composed schema — spike 4's
//! shape — so each is directly fixture-testable.

use std::collections::HashMap;

use proc_macro2::Span;

use crate::grammar::{ComposedSchema, EntityDecl, FusionOp};

/// (instance index, entity index) key for the union-find.
type EntKey = (usize, usize);

struct Classes {
    /// Union-find parent over sequential (instance, entity) indices.
    parent: Vec<usize>,
    /// Colocation classes only: canonical key -> member entity keys.
    members: HashMap<usize, Vec<EntKey>>,
}

impl Classes {
    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            let r = self.find(self.parent[x]);
            self.parent[x] = r;
            r
        } else {
            x
        }
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        self.parent[ra.max(rb)] = ra.min(rb);
    }
}

fn err(span: Span, msg: String) -> syn::Error {
    syn::Error::new(span, msg)
}

/// The full final-hop checklist. Order matters only for which error a
/// doubly-broken composition reports first.
pub(crate) fn validate_composition(c: &ComposedSchema) -> syn::Result<()> {
    endpoints_exist(c)?;
    fusion_shapes(c)?;
    open_entities_closed_once(c)?;
    classes_legal(c)?;
    Ok(())
}

/// Fusion endpoints exist: every `instance.Entity` names a real member. This
/// is grammar-level, but only the final hop sees all member schemas.
fn endpoints_exist(c: &ComposedSchema) -> syn::Result<()> {
    for f in &c.fusions {
        for (inst, ent) in [&f.lhs, &f.rhs] {
            let Some((_, _, body)) = c.members.iter().find(|(i, _, _)| i == inst) else {
                return Err(err(
                    inst.span(),
                    format!(
                        "fusion names unknown instance `{inst}` (this composition instantiates: {})",
                        c.instances.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
                    ),
                ));
            };
            if !body.entities.iter().any(|e| e.name == *ent) {
                return Err(err(
                    ent.span(),
                    format!("fusion names unknown entity `{inst}.{ent}`"),
                ));
            }
        }
    }
    Ok(())
}

/// `<->` pairs exactly one open entity with one closed entity, and neither
/// endpoint is root-marked (a root owns no id type to unify); `<=>` never
/// touches open entities.
fn fusion_shapes(c: &ComposedSchema) -> syn::Result<()> {
    let ent_of = |inst: &proc_macro2::Ident, ent: &proc_macro2::Ident| -> Option<&EntityDecl> {
        c.members
            .iter()
            .find(|(i, _, _)| i == inst)
            .and_then(|(_, _, b)| b.entities.iter().find(|e| e.name == *ent))
    };
    for f in &c.fusions {
        let lb = ent_of(&f.lhs.0, &f.lhs.1).expect("endpoints_exist ran");
        let rb = ent_of(&f.rhs.0, &f.rhs.1).expect("endpoints_exist ran");
        let named = |(i, e): &(proc_macro2::Ident, proc_macro2::Ident)| format!("{i}.{e}");
        match f.op {
            FusionOp::Coloc => {
                if lb.open || rb.open {
                    return Err(err(
                        f.lhs.1.span(),
                        format!(
                            "`{} <=> {}`: open entities are closed by `<->` only, never `<=>`",
                            named(&f.lhs),
                            named(&f.rhs)
                        ),
                    ));
                }
            }
            FusionOp::Ref => {
                if lb.root || rb.root {
                    return Err(err(
                        f.lhs.1.span(),
                        format!(
                            "`{} <-> {}`: a root-marked entity owns no id type to unify",
                            named(&f.lhs),
                            named(&f.rhs)
                        ),
                    ));
                }
                if lb.open == rb.open {
                    return Err(err(
                        f.lhs.1.span(),
                        format!(
                            "`{} <-> {}`: reference fusion pairs exactly one open entity with one closed entity",
                            named(&f.lhs),
                            named(&f.rhs)
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Open entities are closed by exactly one `<->`. Zero closures leaves the
/// entity unresolved; double reference closure (`a.X <-> b.N` *and*
/// `a.X <-> c.N`) would unify `b.N`'s and `c.N`'s ids through one open
/// entity.
fn open_entities_closed_once(c: &ComposedSchema) -> syn::Result<()> {
    for (inst, _, body) in &c.members {
        for e in &body.entities {
            if !e.open {
                continue;
            }
            let closures: Vec<String> = c
                .fusions
                .iter()
                .filter(|f| {
                    f.op == FusionOp::Ref
                        && ((f.lhs.0 == *inst && f.lhs.1 == e.name)
                            || (f.rhs.0 == *inst && f.rhs.1 == e.name))
                })
                .map(|f| {
                    format!("{}.{} <-> {}.{}", f.lhs.0, f.lhs.1, f.rhs.0, f.rhs.1)
                })
                .collect();
            match closures.len() {
                1 => {}
                0 => {
                    return Err(err(
                        e.name.span(),
                        format!(
                            "open entity `{inst}.{}` is not closed by any `<->` fusion — composition leaves no open entities",
                            e.name
                        ),
                    ));
                }
                n => {
                    return Err(err(
                        e.name.span(),
                        format!(
                            "open entity `{inst}.{}` is closed by {n} `<->` fusions ({}); exactly one is allowed — double closure would unify two closed id types through one open entity",
                            e.name,
                            closures.join(", ")
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Every colocation-fusion class has exactly one arena placement: at most
/// one root-marked member, element members from at most one instance, and
/// all members agreeing on placement (root-level vs embedded-in-this-class).
/// This rejects double-closed entities, cross-multiplicity colocations, two
/// roots in one class, and peer-shaped cross-instance colocation
/// (`a.X <=> c.Y`, elements of two instances). The host shape — one
/// instance's element hosting other instances' roots — is the legal
/// cross-instance form. A consequence accepted as a design decision: every
/// legal colocation class has at most two members (one element plus at most
/// one root), so `<=>` transitivity is vacuous in practice and one host
/// element hosts exactly one instance's arenas.
fn classes_legal(c: &ComposedSchema) -> syn::Result<()> {
    // Sequential keys and a (instance, entity) -> key map.
    let mut key_of: HashMap<EntKey, usize> = HashMap::new();
    let mut entity_at: Vec<EntKey> = Vec::new();
    for (i, (_, _, body)) in c.members.iter().enumerate() {
        for (e, _) in body.entities.iter().enumerate() {
            key_of.insert((i, e), entity_at.len());
            entity_at.push((i, e));
        }
    }
    let mut classes = Classes { parent: (0..entity_at.len()).collect(), members: HashMap::new() };
    for f in &c.fusions {
        if f.op != FusionOp::Coloc {
            continue;
        }
        let li = c.members.iter().position(|(i, _, _)| *i == f.lhs.0).expect("validated");
        let ri = c.members.iter().position(|(i, _, _)| *i == f.rhs.0).expect("validated");
        let le = c.members[li]
            .2
            .entities
            .iter()
            .position(|e| e.name == f.lhs.1)
            .expect("validated");
        let re = c.members[ri]
            .2
            .entities
            .iter()
            .position(|e| e.name == f.rhs.1)
            .expect("validated");
        let (a, b) = (key_of[&(li, le)], key_of[&(ri, re)]);
        classes.union(a, b);
    }
    for (k, at) in entity_at.iter().enumerate() {
        let r = classes.find(k);
        classes.members.entry(r).or_default().push(*at);
    }

    let root_of: Vec<Option<usize>> = c
        .members
        .iter()
        .map(|(_, _, b)| b.entities.iter().position(|e| e.root))
        .collect();
    // An instance's arenas embed iff its root-marked entity is
    // colocation-fused into a class that also contains an element entity
    // (the host). Reference fusion never affects placement.
    let embedded_in: Vec<Option<usize>> = (0..c.members.len())
        .map(|i| {
            let Some(re) = root_of[i] else { return None };
            let k = key_of[&(i, re)];
            let r = classes.find(k);
            let hosts_elements = classes.members[&r]
                .iter()
                .any(|&(mi, me)| root_of[mi] != Some(me));
            hosts_elements.then_some(r)
        })
        .collect();

    let name_of = |at: EntKey| -> String {
        let (i, e) = at;
        format!("{}.{}", c.members[i].0, c.members[i].2.entities[e].name)
    };

    for (_r, members) in &classes.members {
        // Classes formed only transitively over roots-plus-elements; a
        // single unfused entity is a trivial class nobody declared — skip
        // those (they place themselves).
        if members.len() < 2 {
            continue;
        }
        let roots: Vec<String> = members
            .iter()
            .filter(|&&(mi, me)| root_of[mi] == Some(me))
            .map(|&at| name_of(at))
            .collect();
        if roots.len() > 1 {
            return Err(err(
                Span::call_site(),
                format!(
                    "colocation class {{ {} }} fuses {} root-marked entities ({}); a root owns no id type, and at most one root may join a class",
                    members.iter().map(|&at| name_of(at)).collect::<Vec<_>>().join(", "),
                    roots.len(),
                    roots.join(", ")
                ),
            ));
        }
        let elem_insts: Vec<usize> = members
            .iter()
            .filter(|&&(mi, me)| root_of[mi] != Some(me))
            .map(|&(mi, _)| mi)
            .collect();
        let distinct: Vec<usize> = {
            let mut d = elem_insts.clone();
            d.sort_unstable();
            d.dedup();
            d
        };
        if distinct.len() > 1 {
            return Err(err(
                Span::call_site(),
                format!(
                    "colocation class {{ {} }} fuses elements of multiple instances ({}); one id type would index several instances' arenas — the legal cross-instance form is one instance's element hosting another instance's *root*",
                    members.iter().map(|&at| name_of(at)).collect::<Vec<_>>().join(", "),
                    distinct
                        .iter()
                        .map(|&i| c.members[i].0.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        // Same-instance element-element fusion: sort-sharing within one
        // instance — admitted by the at-most-one-instance reading alone
        // and defined by nothing (a flat schema over shared sorts is
        // reachable only inside one `pattern!` block). The round-4
        // merge pinned the arm; the code landed with M3.
        if roots.is_empty() && distinct.len() == 1 {
            return Err(err(
                Span::call_site(),
                format!(
                    "colocation class {{ {} }} fuses elements of one instance ({}); sort-sharing within an instance is not composition — a flat schema over shared sorts is reachable only inside one `pattern!` block",
                    members.iter().map(|&at| name_of(at)).collect::<Vec<_>>().join(", "),
                    c.members[distinct[0]].0
                ),
            ));
        }
        // Self-hosting: the class's root and its element member belong
        // to one instance — the instance's arenas would embed into
        // their own records. The legal host shape is cross-instance.
        if roots.len() == 1 && distinct.len() == 1 {
            let root_inst = members
                .iter()
                .find(|&&(mi, me)| root_of[mi] == Some(me))
                .map(|&(mi, _)| mi)
                .expect("roots.len() == 1");
            if root_inst == distinct[0] {
                return Err(err(
                    Span::call_site(),
                    format!(
                        "colocation class {{ {} }} fuses instance {}'s root with its own element (self-hosting): its arenas would embed into their own records — the legal host shape is one instance's element hosting *another* instance's root",
                        members.iter().map(|&at| name_of(at)).collect::<Vec<_>>().join(", "),
                        c.members[root_inst].0
                    ),
                ));
            }
        }
        // Placement agreement among element members (a class's element
        // members all place together; disagreement is a cross-multiplicity
        // colocation reached transitively).
        if let Some(&first_elem) = members.iter().find(|&&(mi, me)| root_of[mi] != Some(me)) {
            let want = embedded_in[first_elem.0];
            for &(mi, me) in members.iter().filter(|&&(mi, me)| root_of[mi] != Some(me)) {
                if embedded_in[mi] != want {
                    return Err(err(
                        Span::call_site(),
                        format!(
                            "colocation class {{ {} }} mixes arena placements: `{}` places its arenas per-host while `{}` places them at the store root — one id type cannot index both",
                            members.iter().map(|&at| name_of(at)).collect::<Vec<_>>().join(", "),
                            name_of((mi, me)),
                            name_of(first_elem)
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::{ArrowDecl, EntityDecl as E, Fusion};
    use proc_macro2::Ident;
    use quote::format_ident;

    fn ident(s: &str) -> Ident {
        format_ident!("{}", s)
    }

    fn entity(name: &str, root: bool, open: bool) -> E {
        E { name: ident(name), root, open }
    }

    fn arrow(name: &str, inst: &str, source: &str, target: &str) -> ArrowDecl {
        ArrowDecl {
            name: ident(name),
            inst: Some(syn::parse_str(inst).unwrap()),
            source: ident(source),
            target: ident(target),
            many: None,
            by: None,
            key: None,
            sum: None,
        }
    }

    fn fusion(lhs: &str, op: FusionOp, rhs: &str) -> Fusion {
        let mk = |s: &str| {
            let (i, e) = s.split_once('.').unwrap();
            (ident(i), ident(e))
        };
        Fusion { lhs: mk(lhs), op, rhs: mk(rhs) }
    }

    fn composed(
        members: Vec<(&str, Vec<EntityDecl>, Vec<ArrowDecl>)>,
        fusions: Vec<Fusion>,
    ) -> ComposedSchema {
        ComposedSchema {
            instances: members.iter().map(|(i, _, _)| ident(i)).collect(),
            members: members
                .into_iter()
                .map(|(i, e, a)| (ident(i), "test".to_string(), crate::grammar::SchemaBody { entities: e, arrows: a }))
                .collect(),
            fusions,
            pkg: "test".to_string(),
        }
    }

    fn digraph(inst: &str) -> (&str, Vec<EntityDecl>, Vec<ArrowDecl>) {
        (
            inst,
            vec![entity("Graph", true, false), entity("Node", false, false), entity("Edge", false, false)],
            vec![
                arrow("nodes", "stdlib::Nodes", "Graph", "Node"),
                arrow("out_edges", "stdlib::OutEdges", "Node", "Edge"),
                arrow("source", "stdlib::Source", "Edge", "Node"),
            ],
        )
    }

    #[test]
    fn hierarchy_shape_passes() {
        // outer.Node <=> sub.Subgraph; sub.MainNode <-> inner.Node.
        let sub = (
            "sub",
            vec![
                entity("Subgraph", true, false),
                entity("SubNode", false, false),
                entity("MainNode", false, true),
            ],
            vec![
                arrow("subnodes", "stdlib::Nodes", "Subgraph", "SubNode"),
                arrow("main_node", "crate::r::MainNode", "SubNode", "MainNode"),
            ],
        );
        let c = composed(
            vec![digraph("outer"), digraph("inner"), sub],
            vec![
                fusion("outer.Node", FusionOp::Coloc, "sub.Subgraph"),
                fusion("sub.MainNode", FusionOp::Ref, "inner.Node"),
            ],
        );
        validate_composition(&c).expect("Hierarchy shape is legal");
    }

    #[test]
    fn refinement_shape_passes() {
        let chain = (
            "chain",
            vec![entity("Chain", true, false), entity("Elem", false, true)],
            vec![arrow("elems", "stdlib::OutEdges", "Chain", "Elem")],
        );
        let c = composed(
            vec![digraph("outer"), digraph("inner"), chain],
            vec![
                fusion("outer.Edge", FusionOp::Coloc, "chain.Chain"),
                fusion("chain.Elem", FusionOp::Ref, "inner.Edge"),
            ],
        );
        validate_composition(&c).expect("Refinement shape is legal");
    }

    #[test]
    fn unknown_endpoint_rejected() {
        let c = composed(vec![digraph("a"), digraph("b")], vec![fusion("a.Nope", FusionOp::Coloc, "b.Node")]);
        let e = validate_composition(&c).unwrap_err().to_string();
        assert!(e.contains("unknown entity `a.Nope`"), "{e}");
    }

    #[test]
    fn unknown_instance_rejected() {
        let c = composed(vec![digraph("a")], vec![fusion("zz.Node", FusionOp::Coloc, "a.Node")]);
        let e = validate_composition(&c).unwrap_err().to_string();
        assert!(e.contains("unknown instance `zz`"), "{e}");
    }

    #[test]
    fn coloc_on_open_rejected() {
        let sub = (
            "sub",
            vec![entity("Sub", true, false), entity("Main", false, true)],
            vec![], // arrows irrelevant here
        );
        let c = composed(
            vec![digraph("a"), sub],
            vec![fusion("a.Node", FusionOp::Coloc, "sub.Main")],
        );
        let e = validate_composition(&c).unwrap_err().to_string();
        assert!(e.contains("never `<=>`"), "{e}");
    }

    #[test]
    fn ref_on_two_closed_rejected() {
        let c = composed(
            vec![digraph("a"), digraph("b")],
            vec![fusion("a.Node", FusionOp::Ref, "b.Node")],
        );
        let e = validate_composition(&c).unwrap_err().to_string();
        assert!(e.contains("exactly one open entity with one closed"), "{e}");
    }

    #[test]
    fn ref_on_root_rejected() {
        let sub = (
            "sub",
            vec![entity("Sub", true, false), entity("Main", false, true)],
            vec![],
        );
        let c = composed(
            vec![digraph("a"), sub],
            vec![fusion("a.Graph", FusionOp::Ref, "sub.Main")],
        );
        let e = validate_composition(&c).unwrap_err().to_string();
        assert!(e.contains("no id type to unify"), "{e}");
    }

    #[test]
    fn unclosed_open_rejected() {
        let sub = (
            "sub",
            vec![entity("Sub", true, false), entity("Main", false, true)],
            vec![],
        );
        let c = composed(vec![digraph("a"), sub], vec![]);
        let e = validate_composition(&c).unwrap_err().to_string();
        assert!(e.contains("not closed by any `<->`"), "{e}");
    }

    #[test]
    fn double_reference_closure_rejected() {
        let sub = (
            "sub",
            vec![entity("Sub", true, false), entity("Main", false, true)],
            vec![],
        );
        let c = composed(
            vec![digraph("a"), digraph("b"), sub],
            vec![
                fusion("sub.Main", FusionOp::Ref, "a.Node"),
                fusion("sub.Main", FusionOp::Ref, "b.Node"),
            ],
        );
        let e = validate_composition(&c).unwrap_err().to_string();
        assert!(e.contains("2 `<->` fusions"), "{e}");
    }

    #[test]
    fn two_roots_in_one_class_rejected() {
        let c = composed(
            vec![digraph("a"), digraph("b")],
            vec![fusion("a.Graph", FusionOp::Coloc, "b.Graph")],
        );
        let e = validate_composition(&c).unwrap_err().to_string();
        assert!(e.contains("root-marked entities"), "{e}");
    }

    #[test]
    fn peer_shaped_cross_instance_coloc_rejected() {
        let c = composed(
            vec![digraph("a"), digraph("b")],
            vec![fusion("a.Node", FusionOp::Coloc, "b.Edge")],
        );
        let e = validate_composition(&c).unwrap_err().to_string();
        assert!(e.contains("elements of multiple instances"), "{e}");
    }

    #[test]
    fn double_closed_elements_rejected() {
        // a.Node <=> b.Node and a.Node <=> c.Node: three elements, two
        // instances, transitively one class.
        let c = composed(
            vec![digraph("a"), digraph("b"), digraph("c")],
            vec![
                fusion("a.Node", FusionOp::Coloc, "b.Node"),
                fusion("a.Node", FusionOp::Coloc, "c.Node"),
            ],
        );
        let e = validate_composition(&c).unwrap_err().to_string();
        assert!(e.contains("elements of multiple instances"), "{e}");
    }
}
