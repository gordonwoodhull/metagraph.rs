// implement!-time binding check (composed): a `many` arrow with no
// binding — `arena` vs `intrusive_list` is an explicit choice.
mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        H {
            instances:
                outer: metagraph_exper::stdlib::Digraph,
                inner: metagraph_exper::stdlib::Digraph,
                sub:   m2_fixtures::shapes::Subgraph;
            sub.MainNode <-> inner.Node;
            sub.MainEdge <-> inner.Edge;
            outer.Node  <=> sub.Subgraph;
        }
    }
}

use metagraph_exper::implement;

implement! {
    comp::H as HStore {
        outer.nodes:     arena,
        outer.out_edges: intrusive_list,
        // outer.in_edges unbound
        inner.nodes:     arena,
        inner.out_edges: intrusive_list,
        inner.in_edges:  intrusive_list,
        sub.subnodes:    arena,
        sub.subedges:    arena,
        sub.subnode:     hash_index,
        sub.subedge:     hash_index,
    }
}

fn main() {}
