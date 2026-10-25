fn main() {
    let t = std::time::Instant::now();
    for f in encheo_discovery::discover(std::time::Duration::from_secs(3)) {
        println!("{:?}  {}  {}", f.kind, f.name, f.address);
    }
    eprintln!("done in {:.1}s", t.elapsed().as_secs_f64());
}
