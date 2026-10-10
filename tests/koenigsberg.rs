//! Traversal tests over the seven bridges of Königsberg: four landmasses,
//! seven bridges, two of the A–B and A–C bridges doubled. Each undirected
//! bridge is modeled as a pair of directed edges, so the store holds 4 nodes
//! and 14 edges.
//!
//! Landmasses: A = Kneiphof island, B = north bank, C = south bank,
//! D = Lomse island.

use metagraph_exper::algo::{bfs, dfs};
use metagraph_exper::arena::{GraphStore, NodeId};
use metagraph_exper::traits::{Edges, InEdges, Nodes, OutEdges, Source, Target};

const A: u32 = 0;
const B: u32 = 1;
const C: u32 = 2;
const D: u32 = 3;

fn koenigsberg() -> GraphStore {
    let mut s = GraphStore::new();
    let a = s.add_node();
    let b = s.add_node();
    let c = s.add_node();
    let d = s.add_node();
    assert_eq!([a.0, b.0, c.0, d.0], [A, B, C, D]);

    // Two bridges A–B, two A–C, one A–D, one B–D, one C–D;
    // each bridge is a pair of directed edges.
    for (x, y) in [(a, b), (a, b), (a, c), (a, c), (a, d), (b, d), (c, d)] {
        s.add_edge(x, y);
        s.add_edge(y, x);
    }
    s
}

fn node_ids(order: Vec<metagraph_exper::arena::Node<'_>>) -> Vec<u32> {
    order.into_iter().map(|n| n.id().0).collect()
}

#[test]
fn store_shape() {
    let s = koenigsberg();
    assert_eq!(s.node_count(), 4);
    assert_eq!(s.edge_count(), 14);

    let g = s.graph();
    assert_eq!(g.nodes().count(), 4);
    let ids: Vec<u32> = g.nodes().map(|n| n.id().0).collect();
    assert_eq!(ids, [A, B, C, D]);
}

#[test]
fn adjacency_degrees() {
    let s = koenigsberg();
    let g = s.graph();
    let nodes: Vec<_> = g.nodes().collect();
    let (a, b, c, d) = (nodes[0], nodes[1], nodes[2], nodes[3]);

    // Kneiphof (A) had five bridges; the others three each.
    assert_eq!(a.out_edges().count(), 5);
    assert_eq!(a.in_edges().count(), 5);
    assert_eq!(a.edges().count(), 10);

    for n in [b, c, d] {
        assert_eq!(n.out_edges().count(), 3);
        assert_eq!(n.in_edges().count(), 3);
        assert_eq!(n.edges().count(), 6);
    }
}

#[test]
fn edge_endpoints() {
    let s = koenigsberg();
    let g = s.graph();
    let a = g.nodes().next().unwrap();

    // Adjacency lists are in reverse insertion order, so A's first
    // out-edge is the last one inserted from A: A -> D.
    let e = a.out_edges().next().unwrap();
    assert_eq!(e.source().id(), NodeId(A));
    assert_eq!(e.target().id(), NodeId(D));
}

#[test]
fn bfs_order() {
    let s = koenigsberg();
    let g = s.graph();
    let a = g.nodes().next().unwrap();

    // A's out-list iterates D, C, C, B, B (reverse insertion), so BFS
    // discovers D, then C, then B.
    assert_eq!(node_ids(bfs(a)), [A, D, C, B]);
}

#[test]
fn dfs_order() {
    let s = koenigsberg();
    let g = s.graph();
    let a = g.nodes().next().unwrap();

    // Stack-based DFS pops the most recently pushed neighbor first.
    assert_eq!(node_ids(dfs(a)), [A, B, D, C]);
}

#[test]
fn bfs_visits_every_node_once_from_any_start() {
    let s = koenigsberg();
    let g = s.graph();
    for start in g.nodes() {
        let mut ids = node_ids(bfs(start));
        ids.sort_unstable();
        assert_eq!(ids, [A, B, C, D]);
    }
}

#[test]
fn self_loop_appears_in_both_directions() {
    let mut s = GraphStore::new();
    let x = s.add_node();
    s.add_edge(x, x);

    let n = s.graph().nodes().next().unwrap();
    assert_eq!(n.out_edges().count(), 1);
    assert_eq!(n.in_edges().count(), 1);
    // documented behavior: `edges()` chains the two lists
    assert_eq!(n.edges().count(), 2);
}
