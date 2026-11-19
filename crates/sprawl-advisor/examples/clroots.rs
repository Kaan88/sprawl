// dev: list the Cleaners tab's quick roots and try reading them, one at a time and together
fn main() {
    let cat = sprawl_advisor::cleaners::catalogue_with(&[sprawl_advisor::RulePack::builtin()], false);
    let (roots, out) = sprawl_advisor::cleaners::quick_roots(&cat);
    println!("{} roots, {} left out", roots.len(), out.len());
    let progress = sprawl_core::Progress::default();
    for r in &roots {
        match sprawl_core::scan_many_inventory(std::slice::from_ref(r), &progress) {
            Ok(t) => println!("ok   {} ({} items)", r.display(), t.nodes.len()),
            Err(e) => println!("FAIL {}: {e}", r.display()),
        }
    }
    match sprawl_core::scan_many_inventory(&roots, &progress) {
        Ok(t) => println!("together ok ({} items)", t.nodes.len()),
        Err(e) => println!("together FAIL: {e}"),
    }
}
