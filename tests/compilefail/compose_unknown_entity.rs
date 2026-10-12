// compose-time structural check (endpoints_exist): a fusion naming an
// entity the named instance does not declare.
mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Bad {
            instances: main: metagraph_exper::stdlib::Digraph;
            main.Node <=> main.Vertex;
        }
    }
}

fn main() {}
