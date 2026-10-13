// backing!-time grammar rejection with a declaration-site span (M3 step 7):
// a clause repeats only as its selector distinguishes it — a bare
// repetition is rejected naming the clause.
mod bad {
    use metagraph::backing;

    backing! {
        bad_backing: A -> B, many {
            iter Walk { cur: u32 }
            iterate(a) { Walk { cur: 0 } }
            advance(w) { handle(w.cur) }
            iteration_order: forward_insertion
            iteration_order: reverse_insertion
        }
    }
}

fn main() {}
