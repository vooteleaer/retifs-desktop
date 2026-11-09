//! `cargo run -p retifs-desktop-core --example purpose -- NS [WORD,…]`: what a
//! library is for (ADR-0032), set first if words are given.

use retifs_desktop_core::Daemon;

fn main() {
    let mut args = std::env::args().skip(1);
    let ns = args.next().expect("NS");
    let d = Daemon::new();
    if let Some(words) = args.next() {
        let words: Vec<String> = words.split(',').map(String::from).filter(|w| !w.is_empty()).collect();
        d.set_purpose(&ns, &words).unwrap_or_else(|e| panic!("{e}"));
    }
    let p = d.purpose(&ns).unwrap_or_else(|e| panic!("{e}"));
    println!("{ns}: {}", if p.is_empty() { "generic".to_string() } else { p.join(", ") });
}
