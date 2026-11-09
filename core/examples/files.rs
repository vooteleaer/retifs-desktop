//! `cargo run -p retifs-desktop-core --example files -- NS [DIR]`: a library's
//! folder as the files module shows it, and the first file in it read.

use retifs_desktop_core::Daemon;

fn main() {
    let mut args = std::env::args().skip(1);
    let ns = args.next().expect("NS");
    let dir = args.next().unwrap_or_else(|| "/".into());
    let d = Daemon::new();
    let t = std::time::Instant::now();
    let entries = d.list(&ns, &dir).unwrap_or_else(|e| panic!("{e}"));
    println!("{} entries in {dir} ({} ms)", entries.len(), t.elapsed().as_millis());
    for e in entries.iter().take(10) {
        println!("  {}{}  {} B", e.name, if e.dir { "/" } else { "" }, e.size);
    }
    let t = std::time::Instant::now();
    let again = d.list(&ns, &dir).map(|e| e.len()).unwrap_or(0);
    println!("again: {again} entries ({} ms, from the kept tree)", t.elapsed().as_millis());
    if let Some(f) = entries.iter().find(|e| !e.dir) {
        let mut out = Vec::new();
        let n = d.read_file(&ns, &f.path, &mut out).unwrap_or_else(|e| panic!("{e}"));
        println!("read {}: {n} B (says {} B)", f.path, f.size);
    }
}
