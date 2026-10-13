// implement!-time cardinality check (composed): a keyed arrow binds
// `hash_index` or nothing legal — `arena` has the wrong cardinality.
mod comp {
    use metagraph::compose_pattern;

    compose_pattern! {
        H {
            instances:
                outer: metagraph::stdlib::Digraph,
                inner: metagraph::stdlib::Digraph,
                sub:   m2_fixtures::shapes::Subgraph;
            sub.MainNode <-> inner.Node;
            sub.MainEdge <-> inner.Edge;
            outer.Node  <=> sub.Subgraph;
        }
    }
}

use metagraph::implement;

implement! {
    comp::H as HStore {
        outer.nodes:     arena,
        outer.out_edges: intrusive_list,
        outer.in_edges:  intrusive_list,
        inner.nodes:     arena,
        inner.out_edges: intrusive_list,
        inner.in_edges:  intrusive_list,
        sub.subnodes:    arena,
        sub.subedges:    arena,
        sub.subnode:     arena,
    }
}

fn main() {}
