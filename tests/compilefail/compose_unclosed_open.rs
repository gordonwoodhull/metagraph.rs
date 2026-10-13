// compose-time structural check (open_entities_closed_once): an open
// entity left unresolved — the composition leaves no open entities.
mod comp {
    use metagraph::compose_pattern;

    compose_pattern! {
        Bad {
            instances:
                main: metagraph::stdlib::Digraph,
                sub:  m2_fixtures::shapes::Subgraph;
            // sub.MainNode and sub.MainEdge are open and never closed.
        }
    }
}

fn main() {}
