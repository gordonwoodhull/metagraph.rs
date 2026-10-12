// implement!-time placement check: embedding depth > 1 — `mid` hosts
// arenas (inner fuses into its element) and is itself embedded into
// `outer`.
mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Deep {
            instances:
                outer: metagraph_exper::stdlib::Digraph,
                mid:   metagraph_exper::stdlib::Digraph,
                inner: metagraph_exper::stdlib::Digraph;
            outer.Node <=> mid.Graph;
            mid.Node   <=> inner.Graph;
        }
    }
}

use metagraph_exper::implement;

implement! {
    comp::Deep as DeepStore {
        outer.nodes:     arena,
        outer.out_edges: intrusive_list,
        outer.in_edges:  intrusive_list,
        mid.nodes:       arena,
        mid.out_edges:   intrusive_list,
        mid.in_edges:    intrusive_list,
        inner.nodes:     arena,
        inner.out_edges: intrusive_list,
        inner.in_edges:  intrusive_list,
    }
}

fn main() {}
