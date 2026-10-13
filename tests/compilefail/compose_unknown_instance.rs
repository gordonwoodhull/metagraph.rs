// compose-time structural check (endpoints_exist): a fusion naming an
// instance the composition does not declare.
mod comp {
    use metagraph::compose_pattern;

    compose_pattern! {
        Bad {
            instances: main: metagraph::stdlib::Digraph;
            main.Node <=> ghost.Node;
        }
    }
}

fn main() {}
