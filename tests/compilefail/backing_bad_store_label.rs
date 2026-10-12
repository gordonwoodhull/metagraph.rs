// backing!-time grammar rejection with a declaration-site span (M3 step 7):
// `on store` carries a closed phase-label set, quoted.
mod bad {
    use metagraph_exper::backing;

    backing! {
        bad_backing: A -> B, many {
            on store archival { buckets: Vec<BId> }
            iter Walk { cur: u32 }
            iterate(a) { Walk { cur: 0 } }
            advance(w) { handle(w.cur) }
        }
    }
}

fn main() {}
