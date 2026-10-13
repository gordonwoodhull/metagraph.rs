//! M4 step 6 — write-trait codegen acceptance: spike 8's copy and
//! node-replacement transforms recast against the GENERATED verb surface.
//!
//! Spike 8 hand-wrote its traits under ONE marker per backend; the
//! generated surface has M1's per-arrow read markers and M4's per-entity
//! write markers, so the transforms thread one marker parameter per bound
//! trait — the plan's "bound on `OutEdges<M>` and `InsertOutEdges<M>`
//! together" works because both name the SAME arrow. `HandleToRef`'s
//! per-entity marker is the same struct `Add`/`Remove` use, so the
//! equality web collapses onto it. Membership-assert `Insert<Arrow>`
//! traits exist for genuinely primitive `many` arrows, but Digraph's
//! `out_edges` is `by`-derived — the copy bounds on `AddNode + AddEdge`
//! alone (the constructor fires the derived membership hooks). `flush`
//! stays concrete — no `Commit` trait.
//!
//! Spike 8 instantiated its transforms at two unrelated markers over one
//! hand-written backend; here one marker set serves two store types (a
//! second MARKER set requires a composed pattern — composed Tx lands at
//! step 8), so genericity is exercised across two `Tx` types instead —
//! noted as the step-6 reading of "two marker instantiations".

use std::collections::HashMap;
use std::hash::Hash;

use metagraph_exper::stdlib::digraph::{AddEdge, AddNode, RemoveEdge, RemoveNode};
use metagraph_exper::stdlib::{InEdges, Nodes, OutEdges, Source, Target};
use metagraph_exper::tx::HandleToRef;

mod ka {
    use metagraph_exper::implement;

    implement! {
        metagraph_exper::stdlib::Digraph as KaStore {
            nodes:     arena,
            out_edges: intrusive_list,
            in_edges:  intrusive_list,
        }
    }
}

mod kb {
    use metagraph_exper::implement;

    implement! {
        metagraph_exper::stdlib::Digraph as KbStore {
            nodes:     arena,
            out_edges: intrusive_list,
            in_edges:  intrusive_list,
        }
    }
}

/// Pure copy into an empty destination: every dst node is NEW (`AddNode`,
/// Pending refs), edges via the per-pattern constructor `AddEdge` — the
/// `by`-derived membership follows the ctor args' hooks, so no
/// `InsertOutEdges` bound appears. The collect-and-reverse is spike 8's
/// order contract: intrusive lists iterate in reverse insertion order, so
/// re-inserting in iteration order would reverse every list. One marker
/// parameter per bound trait: `MN` nodes, `MO` out-edges, `MS`/`MT` the
/// edge endpoints, `MEN`/`MEE` the per-entity write traits.
fn copy_graph<Src, Dst, MN, MO, MS, MT, MEN, MEE>(src: &Src, dst: &mut Dst)
where
    Src: Nodes<MN>,
    Src::Node: OutEdges<MO> + Copy + Eq + Hash,
    <Src::Node as OutEdges<MO>>::Edge:
        Source<MS, Node = Src::Node> + Target<MT, Node = Src::Node>,
    Dst: AddNode<MEN>
        + AddEdge<
            MEE,
            Source = <Dst as AddNode<MEN>>::NodeRef,
            Target = <Dst as AddNode<MEN>>::NodeRef,
        >,
    <Dst as AddNode<MEN>>::NodeRef: Copy,
{
    let mut node_map: HashMap<Src::Node, <Dst as AddNode<MEN>>::NodeRef> = HashMap::new();
    for n in src.nodes() {
        node_map.insert(n, dst.add_node());
    }
    for n in src.nodes() {
        let mut ends: Vec<(Src::Node, Src::Node)> =
            n.out_edges().map(|e| (e.source(), e.target())).collect();
        ends.reverse();
        for (s, t) in ends {
            dst.add_edge(node_map[&s], node_map[&t]);
        }
    }
}

/// Old-store-referencing splice: kept nodes are named by `Existing` refs
/// minted from handles through `HandleToRef` (its per-entity marker is
/// the same struct `AddNode`/`AddEdge` use — the full bound web); the
/// fresh replacement node is `Pending` from `AddNode`. Kept-kept edges
/// stage NOTHING — staging a duplicate would corrupt the copy.
fn replace_node<Src, Dst, MN, MO, MS, MT, MEN, MEE>(
    src: &Src,
    dst: &mut Dst,
    is_victim: impl Fn(&Src::Node) -> bool,
) where
    Src: Nodes<MN>,
    Src::Node: OutEdges<MO> + Copy + Eq + Hash,
    <Src::Node as OutEdges<MO>>::Edge:
        Source<MS, Node = Src::Node> + Target<MT, Node = Src::Node>,
    Dst: AddNode<MEN>
        + AddEdge<
            MEE,
            Source = <Dst as AddNode<MEN>>::NodeRef,
            Target = <Dst as AddNode<MEN>>::NodeRef,
        >
        + RemoveNode<MEN, NodeRef = <Dst as AddNode<MEN>>::NodeRef>
        + RemoveEdge<MEE, EdgeRef = <Dst as AddEdge<MEE>>::EdgeRef>
        + HandleToRef<MEN, Handle = Src::Node, Ref = <Dst as AddNode<MEN>>::NodeRef>
        + HandleToRef<
            MEE,
            Handle = <Src::Node as OutEdges<MO>>::Edge,
            Ref = <Dst as AddEdge<MEE>>::EdgeRef,
        >,
    <Dst as AddNode<MEN>>::NodeRef: Copy,
{
    // Pass 1: classify nodes. Victims are staged for removal; kept nodes
    // are named by Existing refs minted from the read-side handles.
    let nodes: Vec<Src::Node> = src.nodes().collect();
    let mut kept: HashMap<Src::Node, <Dst as AddNode<MEN>>::NodeRef> = HashMap::new();
    let mut victims = Vec::new();
    for &n in &nodes {
        if is_victim(&n) {
            victims.push(n);
            dst.remove_node(<Dst as HandleToRef<MEN>>::to_ref(dst, n));
        } else {
            kept.insert(n, <Dst as HandleToRef<MEN>>::to_ref(dst, n));
        }
    }
    let is_victim = |n: &Src::Node| victims.contains(n);

    // The fresh replacement node (Pending ref, minted by AddNode).
    let fresh = dst.add_node();

    // Pass 2: rewire edges. Every edge is visited exactly once via its
    // source's out-list.
    for &n in &nodes {
        for e in n.out_edges() {
            let (s, t) = (e.source(), e.target());
            match (is_victim(&s), is_victim(&t)) {
                (false, false) => {}
                (true, true) => {
                    dst.remove_edge(<Dst as HandleToRef<MEE>>::to_ref(dst, e));
                    dst.add_edge(fresh, fresh);
                }
                (true, false) => {
                    dst.remove_edge(<Dst as HandleToRef<MEE>>::to_ref(dst, e));
                    dst.add_edge(fresh, kept[&t]);
                }
                (false, true) => {
                    dst.remove_edge(<Dst as HandleToRef<MEE>>::to_ref(dst, e));
                    dst.add_edge(kept[&s], fresh);
                }
            }
        }
    }
}

/// An observable signature: counts plus per-node out/in iteration orders,
/// with handles rendered as their position in node-iteration order. The
/// read-side marker-generic probe — src and dst stores compare without
/// knowing the pattern.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Signature {
    node_count: usize,
    edge_count: usize,
    /// Per node, in node-iteration order: (out-list, in-list), each entry
    /// (index of source, index of target) in iteration order.
    adjacency: Vec<(Vec<(usize, usize)>, Vec<(usize, usize)>)>,
}

fn signature<Src, MN, MO, MI, MS, MT, MSI, MTI>(src: &Src) -> Signature
where
    Src: Nodes<MN>,
    Src::Node: OutEdges<MO> + InEdges<MI> + Copy + Eq + Hash,
    <Src::Node as OutEdges<MO>>::Edge:
        Source<MS, Node = Src::Node> + Target<MT, Node = Src::Node>,
    <Src::Node as InEdges<MI>>::Edge:
        Source<MSI, Node = Src::Node> + Target<MTI, Node = Src::Node>,
{
    let nodes: Vec<Src::Node> = src.nodes().collect();
    let index: HashMap<Src::Node, usize> =
        nodes.iter().copied().enumerate().map(|(i, n)| (n, i)).collect();
    let at = |n: Src::Node| index[&n];

    let adjacency: Vec<(Vec<(usize, usize)>, Vec<(usize, usize)>)> = nodes
        .iter()
        .map(|&n| {
            let out = n.out_edges().map(|e| (at(e.source()), at(e.target()))).collect();
            let inc = n.in_edges().map(|e| (at(e.source()), at(e.target()))).collect();
            (out, inc)
        })
        .collect();
    // Every edge appears in exactly one out-list (membership consistency).
    let edge_count: usize = adjacency.iter().map(|(o, _)| o.len()).sum();
    Signature { node_count: nodes.len(), edge_count, adjacency }
}

/// Build Königsberg directly (edge insertion order is observable — the
/// copy must reproduce it). The classic multigraph: two banks, two
/// islands, seven bridges, each crossing walkable both ways.
fn koenigsberg() -> ka::KaStore {
    let mut s = ka::KaStore::new();
    let a = s.add_node();
    let b = s.add_node();
    let c = s.add_node();
    let d = s.add_node();
    for (x, y) in [(a, b), (a, b), (a, c), (a, c), (a, d), (b, d), (c, d)] {
        s.add_edge(x, y);
        s.add_edge(y, x);
    }
    s
}

#[test]
fn copy_into_empty_destination_is_signature_identical() {
    let src = koenigsberg();
    let dst_store = kb::KbStore::new();
    let mut tx = dst_store.edit();
    copy_graph(&src.graph(), &mut tx);
    let out = tx.flush().expect("flush");
    assert_eq!(signature(&out.store.graph()), signature(&src.graph()));
}

#[test]
fn replace_node_generic_matches_direct_staging() {
    use ka::NodeId;

    let store = koenigsberg();

    // Direct equivalent: stage through the concrete Tx API in EXACTLY the
    // order the generic algorithm stages ops (pass 1 in node order; pass 2
    // in per-node out-iteration order). Observable list order depends on
    // staging order, so the equivalence holds order-for-order.
    let direct = {
        let mut tx = store.edit();
        for n in store.graph().nodes() {
            if n.id() == NodeId(0) {
                tx.remove_node(n);
            }
        }
        let fresh = tx.add_node();
        for n in store.graph().nodes() {
            for e in n.out_edges() {
                let (s, t) = (e.source(), e.target());
                match (s.id() == NodeId(0), t.id() == NodeId(0)) {
                    (false, false) => {}
                    (true, true) => {
                        tx.remove_edge(e);
                        tx.add_edge(fresh, fresh);
                    }
                    (true, false) => {
                        tx.remove_edge(e);
                        tx.add_edge(fresh, t);
                    }
                    (false, true) => {
                        tx.remove_edge(e);
                        tx.add_edge(s, fresh);
                    }
                }
            }
        }
        tx.flush().expect("direct flush")
    };

    let mut tx = store.edit();
    replace_node(&store.graph(), &mut tx, |n| n.id() == NodeId(0));
    let generic = tx.flush().expect("generic flush");

    assert_eq!(signature(&direct.store.graph()), signature(&generic.store.graph()));
    assert_eq!(direct.report.node_map, generic.report.node_map);
    assert_eq!(direct.report.edge_map, generic.report.edge_map);
}

#[test]
fn remove_edge_stages_through_the_trait() {
    // The Remove<Edge> trait verb is the trait-bound form of the inherent
    // `remove_edge`: same op, same flush.
    use ka::EdgeId;

    let store = koenigsberg();
    let via_trait = {
        let mut tx = store.edit();
        RemoveEdge::remove_edge(&mut tx, ka::EdgeRef::existing(EdgeId(3)));
        tx.flush().expect("trait flush")
    };
    let via_inherent = {
        let mut tx = store.edit();
        tx.remove_edge(EdgeId(3));
        tx.flush().expect("inherent flush")
    };
    assert_eq!(via_trait.report.edge_map, via_inherent.report.edge_map);
    assert_eq!(signature(&via_trait.store.graph()), signature(&via_inherent.store.graph()));
    // The victim edge is gone (14 directed edges minus one).
    assert_eq!(via_trait.store.edge_count(), 13);
    assert_eq!(via_trait.store.node_count(), 4);
}
