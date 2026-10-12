// implement!-time applicability: a keyed arrow whose target class is
// embedded in a class its self is no part of. The authored
// value-host-agreement check (`host_of(v) != owner`) compares against a
// self-side anchor of the host id type; an unfused-root self has none, so
// the uniform body cannot bind and the placement is rejected outright —
// not silently emitted without the check.
mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Gap {
            instances:
                b:   metagraph_exper::stdlib::Digraph,
                sub: m2_fixtures::shapes::Subgraph,
                a:   m2_fixtures::neg::AnchoredKey;
            sub.MainNode <-> b.Node;
            sub.MainEdge <-> b.Edge;
            sub.Subgraph <=> b.Node;
            a.E <-> sub.SubNode;
        }
    }
}

use metagraph_exper::implement;

implement! {
    comp::Gap as GapStore {
        b.nodes:     arena,
        b.out_edges: intrusive_list,
        b.in_edges:  intrusive_list,
        sub.subnodes: arena,
        sub.subedges: arena,
        sub.subnode:  hash_index,
        sub.subedge:  hash_index,
        a.k:          hash_index,
    }
}

fn main() {}
