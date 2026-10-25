//! cargo run -p encheo-sublang --example scan -- <url or path>
fn main() {
    let src = std::env::args().nth(1).expect("url or path");
    let t = std::time::Instant::now();
    let budget = encheo_sublang::Budget::default();
    let texts = if src.starts_with("http") {
        encheo_sublang::untagged_subtitle_text(encheo_sublang::HttpReader::new(&src).unwrap(), &budget)
    } else {
        encheo_sublang::untagged_subtitle_text(std::fs::File::open(&src).unwrap(), &budget)
    }
    .unwrap();
    for (i, text) in &texts {
        let lang = encheo_sublang::detect(text);
        let sample: String = text.chars().take(60).collect::<String>().replace('\n', " / ");
        println!("#{i:<3} {:<12} {:>4.0}%  {sample}", lang.as_ref().map_or("?", |l| l.name), lang.map_or(0.0, |l| l.confidence * 100.0));
    }
    eprintln!("{} untagged tracks in {:.1}s", texts.len(), t.elapsed().as_secs_f64());
}
