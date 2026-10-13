// compose-time structural check (fusion_shapes): open entities are closed
// by `<->` only, never `<=>` — colocation would merge an unresolved
// entity's records into a host.
mod comp {
    use metagraph::compose_pattern;

    compose_pattern! {
        Bad {
            instances:
                main: metagraph::stdlib::Digraph,
                sub:  m2_fixtures::shapes::Subgraph;
            sub.MainNode <=> main.Node;
        }
    }
}

fn main() {}
