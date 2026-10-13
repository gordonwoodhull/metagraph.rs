// backing!-time grammar rejection with a declaration-site span (M3 step 7):
// the clause vocabulary is closed and quoted. (The unit-test arm lives in
// macros/src/backing.rs; this pins the span behavior at the macro layer.)
mod bad {
    use metagraph::backing;

    backing! {
        bad_backing: A -> B, many {
            iter Walk { cur: u32 }
            iterate(a) { Walk { cur: 0 } }
            advance(w) { handle(w.cur) }
            frobnicate
        }
    }
}

fn main() {}
