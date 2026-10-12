// compose-time structural check (open_entities_closed_once): an open
// entity left unresolved — the composition leaves no open entities.
mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Bad {
            instances:
                main: metagraph_exper::stdlib::Digraph,
                sub:  m2_fixtures::shapes::Subgraph;
            // sub.MainNode and sub.MainEdge are open and never closed.
        }
    }
}

fn main() {}
