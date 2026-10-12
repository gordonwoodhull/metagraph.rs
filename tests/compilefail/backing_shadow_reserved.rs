// backing!-time grammar rule (M3 step 7, hygiene caveat 3 as a grammar
// rule): the role rewrite is textual and unconditional, so a body binding
// a reserved ident (`let rec = ..`) would be silently captured — rejected
// at declaration time, quoting the closed set.
mod shady {
    use metagraph_exper::backing;

    backing! {
        shady_list: A -> B, many {
            on A { first: Option<BId> = stem }
            on B { next: Option<BId> = stem }
            iter Walk { cur: Option<BId> }
            on_insert(a, b) {
                let rec = a;
                rec(b).next = rec(a).first;
                rec(a).first = Some(b);
            }
            iterate(a) { Walk { cur: rec(a).first } }
            advance(w) {
                let id = w.cur?;
                w.cur = rec(id).next;
                Some(handle(id))
            }
            validate primitive { }
        }
    }
}

fn main() {}
