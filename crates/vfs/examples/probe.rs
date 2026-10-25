//! cargo run -p encheo-vfs --example probe -- <local|webdav|smb> <location> [user] [password]
use encheo_vfs::{Config, Kind, Source};
use std::io::{Read, Seek, SeekFrom};

fn walk(src: &Source, path: &str, depth: usize) -> Option<String> {
    let mut first_file = None;
    for e in src.list(path).expect("list") {
        println!("{}{}{}", "  ".repeat(depth), e.name, if e.is_dir { "/" } else { "" });
        if e.is_dir {
            let f = walk(src, &e.path, depth + 1);
            first_file = first_file.or(f);
        } else {
            first_file = first_file.or(Some(e.path.clone()));
        }
    }
    first_file
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let kind = match a[0].as_str() {
        "local" => Kind::Local,
        "webdav" => Kind::WebDav,
        _ => Kind::Smb,
    };
    let config = Config {
        kind,
        name: "probe".into(),
        location: a[1].clone(),
        username: a.get(2).cloned().unwrap_or_default(),
        password: a.get(3).cloned().unwrap_or_default(),
    };
    let t = std::time::Instant::now();
    let src = Source::connect(&config).expect("connect");
    println!("connected in {:?}", t.elapsed());
    let file = walk(&src, "", 0).expect("a media file");
    let mut f = src.open(&file).expect("open");
    let len = f.len().unwrap_or(0);
    f.seek(SeekFrom::Start(len / 2)).unwrap();
    let mut buf = vec![0; 64 * 1024];
    f.read_exact(&mut buf).unwrap();
    let t = std::time::Instant::now();
    let mut total = 0;
    f.seek(SeekFrom::Start(0)).unwrap();
    let mut big = vec![0; 1 << 20];
    while total < len.min(20 << 20) {
        let n = f.read(&mut big).unwrap();
        if n == 0 { break; }
        total += n as u64;
    }
    println!("{file}: {len} bytes; read middle ok; {:.1} MB in {:?}", total as f64 / 1e6, t.elapsed());
    println!("direct url: {:?}", src.direct_url(&file).map(|d| d.url));
}
