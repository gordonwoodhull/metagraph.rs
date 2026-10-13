// backing!-time grammar rejection with a declaration-site span (M3 step 7):
// contract annotations draw from a closed set, quoted.
mod bad {
    use metagraph::backing;

    backing! {
        bad_backing: A -> B, many {
            iter Walk { cur: u32 }
            iterate(a) { Walk { cur: 0 } }
            advance(w) { handle(w.cur) }
            contract: shared_membership
        }
    }
}

fn main() {}
