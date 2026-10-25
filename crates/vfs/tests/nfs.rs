//! A real NFSv3 conversation with an in-process server (no root, no Docker).
use encheo_vfs::{Config, Kind, Source};
use nfs3_server::memfs::{MemFs, MemFsConfig};
use nfs3_server::tcp::{NFSTcp, NFSTcpListener};
use std::io::{Read, Seek, SeekFrom};

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[test]
fn lists_and_reads_over_nfs() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let port = rt.block_on(async {
        let mut cfg = MemFsConfig::default();
        cfg.add_dir("/Movies");
        cfg.add_dir("/Movies/Heat (1995)");
        cfg.add_file("/Movies/Heat (1995)/Heat.1995.mkv", pattern(3 << 20));
        cfg.add_file("/Movies/notes.txt", b"x".to_vec());
        let listener = NFSTcpListener::bind("127.0.0.1:0", MemFs::new(cfg).unwrap()).await.unwrap();
        let port = listener.get_listen_port();
        tokio::spawn(async move { listener.handle_forever().await });
        port
    });
    let src = Source::connect(&Config {
        kind: Kind::Nfs,
        name: "t".into(),
        location: format!("nfs://127.0.0.1:{port}/"),
        username: String::new(),
        password: String::new(),
        host_key: None,
    })
    .expect("mount");
    let top: Vec<_> = src.list("").unwrap().into_iter().map(|e| (e.name, e.is_dir)).collect();
    assert_eq!(top, [("Movies".to_string(), true)]);
    let movies: Vec<_> = src.list("Movies").unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(movies, ["Heat (1995)"]); // notes.txt filtered out
    let files = src.list("Movies/Heat (1995)").unwrap();
    assert_eq!(files[0].size, Some(3 << 20));

    let mut f = src.open("Movies/Heat (1995)/Heat.1995.mkv").unwrap();
    assert_eq!(f.len(), Some(3 << 20));
    let at = 1_500_123u64;
    f.seek(SeekFrom::Start(at)).unwrap();
    let mut buf = vec![0; 300_000];
    f.read_exact(&mut buf).unwrap();
    assert_eq!(buf, pattern(3 << 20)[at as usize..at as usize + 300_000]);
    drop(rt);
}
