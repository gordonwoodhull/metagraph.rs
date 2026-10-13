// implement!-time generated-name check: instances `p` and `p_q` both
// qualify their colliding arrow names, and `insert_p_q_r` comes out
// ambiguous — instance p's arrow `q_r` vs instance p_q's arrow `r`.
mod comp {
    use metagraph::compose_pattern;

    compose_pattern! {
        Two {
            instances:
                p:   m2_fixtures::neg::Collide,
                p_q: m2_fixtures::neg::Collide;
        }
    }
}

use metagraph::implement;

implement! {
    comp::Two as TwoStore {
        p.m:     arena,
        p.q_r:   intrusive_list,
        p.r:     intrusive_list,
        p_q.m:   arena,
        p_q.q_r: intrusive_list,
        p_q.r:   intrusive_list,
    }
}

fn main() {}
