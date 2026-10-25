//! Against real servers (the Docker test setup). Set ENCHEO_TEST_SFTP=host:port to run.
use encheo_vfs::{Config, Kind, Source};

fn sftp(host_key: Option<String>) -> Option<Config> {
    let addr = std::env::var("ENCHEO_TEST_SFTP").ok()?;
    Some(Config {
        kind: Kind::Sftp,
        name: "t".into(),
        location: format!("sftp://{addr}/media"),
        username: "tester".into(),
        password: "secret".into(),
        host_key,
    })
}

#[test]
fn sftp_pins_the_host_key() {
    let Some(config) = sftp(None) else { return };
    let first = Source::connect(&config).expect("first connect");
    let key = first.host_key().expect("fingerprint");
    assert!(key.starts_with("SHA256:"), "{key}");
    // Same key: fine.
    Source::connect(&sftp(Some(key)).unwrap()).expect("pinned connect");
    // A different key: refused.
    let err = Source::connect(&sftp(Some("SHA256:not-this-server".into())).unwrap()).err().expect("must refuse");
    assert!(err.to_string().contains("identity key has changed"), "{err}");
}
