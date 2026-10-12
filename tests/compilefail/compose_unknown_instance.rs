// compose-time structural check (endpoints_exist): a fusion naming an
// instance the composition does not declare.
mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Bad {
            instances: main: metagraph_exper::stdlib::Digraph;
            main.Node <=> ghost.Node;
        }
    }
}

fn main() {}
