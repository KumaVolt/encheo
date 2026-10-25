use crate::model::{Directories, Envelope, MetadataPage};
use crate::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

fn page(json: &str) -> MetadataPage {
    serde_json::from_str::<Envelope<MetadataPage>>(json).unwrap().container
}

#[test]
fn resources_keep_servers_and_order_connections() {
    let all: Vec<Resource> = serde_json::from_str(include_str!("../tests/fixtures/resources.json")).unwrap();
    let servers: Vec<_> = all.into_iter().filter(Resource::is_server).collect();
    assert_eq!(servers.len(), 1);
    let s = &servers[0];
    assert_eq!(s.access_token.as_deref(), Some("srv-token"));
    let order: Vec<&str> = s.connection_order().iter().map(|c| c.uri.as_str()).collect();
    assert_eq!(
        order,
        [
            "https://192-168-1-10.abc.plex.direct:32400", // local IPv4
            "https://fe80--1.abc.plex.direct:32400",      // local IPv6
            "https://1-2-3-4.abc.plex.direct:32400",      // remote
            "https://relay.abc.plex.direct:8443",         // relay last
        ]
    );
    assert_eq!(
        s.candidate_urls(),
        [
            "https://192-168-1-10.abc.plex.direct:32400",
            "http://192.168.1.10:32400", // plain-IP fallback for routers that block plex.direct
            "https://fe80--1.abc.plex.direct:32400",
            "http://[fe80::1]:32400",
            "https://1-2-3-4.abc.plex.direct:32400",
            "https://relay.abc.plex.direct:8443",
        ]
    );
}

#[test]
fn sections_parse() {
    let d: Envelope<Directories> = serde_json::from_str(include_str!("../tests/fixtures/sections.json")).unwrap();
    let kinds: Vec<_> = d.container.directory.iter().map(|l| l.kind.as_str()).collect();
    assert_eq!(kinds, ["movie", "show", "artist"]);
}

#[test]
fn movies_parse_with_media_and_progress() {
    let p = page(include_str!("../tests/fixtures/movies.json"));
    assert_eq!(p.total_size, Some(240));
    let arrival = &p.metadata[0];
    assert_eq!(arrival.kind, ItemKind::Movie);
    assert_eq!(arrival.genres.len(), 2);
    assert_eq!(arrival.directors[0].thumb.as_deref(), Some("https://metadata-static.plex.tv/d.jpg"));
    assert_eq!(arrival.ratings.len(), 2);
    assert_eq!(arrival.ratings[1].image, "rottentomatoes://image.rating.ripe");
    assert_eq!(arrival.media[0].video_dynamic_range.as_deref(), Some("HDR10"));
    assert_eq!(arrival.primary_part().unwrap().key, "/library/parts/9/1700/file.mkv");
    assert_eq!(arrival.progress(), Some(0.5));
    assert!(!arrival.is_watched());
    let heat = &p.metadata[1];
    assert!(heat.is_watched());
    assert_eq!(heat.progress(), None);
    assert!(heat.primary_part().is_none());
}

#[test]
fn episode_markers_and_label() {
    let p = page(include_str!("../tests/fixtures/episode.json"));
    let ep = &p.metadata[0];
    assert_eq!(ep.episode_label().as_deref(), Some("S1 · E1"));
    let kinds: Vec<_> = ep.markers.iter().map(|m| m.kind.clone()).collect();
    assert_eq!(kinds, [MarkerKind::Intro, MarkerKind::Credits, MarkerKind::Other]);
    assert_eq!(ep.markers[0].start_time_offset, 62_000);

    let part = ep.primary_part().unwrap();
    let subs: Vec<_> = part.streams_of(StreamKind::Subtitle).collect();
    assert_eq!(subs.len(), 3);
    assert!(subs[0].forced && !subs[0].selected);
    assert!(subs[1].selected);
    assert_eq!(subs[2].index, None); // external file
    assert_eq!(part.streams_of(StreamKind::Audio).next().unwrap().extended_display_title.as_deref(), Some("English (EAC3 Stereo)"));
}

#[test]
fn show_watched_from_leaf_counts() {
    let p = page(include_str!("../tests/fixtures/show.json"));
    assert!(p.metadata[0].is_watched());
}

#[test]
fn urls_and_token_hygiene() {
    let plex = Plex::new(ClientInfo::new("cid", "Test"));
    let s = plex.server_at("Home", "http://10.0.0.2:32400/", "tok");
    assert_eq!(s.base_url(), "http://10.0.0.2:32400");
    let part = Part { key: "/library/parts/9/1/f.mkv".into(), ..Default::default() };
    assert_eq!(s.stream_url(&part), "http://10.0.0.2:32400/library/parts/9/1/f.mkv?X-Plex-Token=tok");
    assert_eq!(
        s.image_url("/library/metadata/1/thumb/2", 300, 450),
        "http://10.0.0.2:32400/photo/:/transcode?width=300&height=450&minSize=1&upscale=1&url=%2Flibrary%2Fmetadata%2F1%2Fthumb%2F2&X-Plex-Token=tok"
    );
    assert_eq!(strip_token("http://h/a?X-Plex-Token=secret&b=1"), "http://h/a?b=1");
    assert_eq!(strip_token("http://h/a?X-Plex-Token=secret"), "http://h/a");

    let pin = Pin { id: 1, code: "ABCD".into(), auth_token: None, expires_in: None };
    let auth = plex.auth_url(&pin);
    assert!(auth.starts_with("https://app.plex.tv/auth#?clientID=cid&code=ABCD&context%5Bdevice%5D%5Bproduct%5D=Encheo"), "{auth}");
}

/// A one-thread HTTP server answering each request with `respond(path) -> (status, body)`.
/// Records the request heads it saw.
fn mock(respond: fn(&str) -> (u16, String)) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", l.local_addr().unwrap());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    std::thread::spawn(move || {
        for stream in l.incoming() {
            let mut stream = stream.unwrap();
            let mut r = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                head.push_str(&line);
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
            seen2.lock().unwrap().push(head);
            let (status, body) = respond(&path);
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (addr, seen)
}

#[tokio::test]
async fn pin_flow_and_headers() {
    let (base, seen) = mock(|path| match path {
        p if p.starts_with("/api/v2/pins?") => (201, r#"{"id":42,"code":"WXYZ","expiresIn":900,"authToken":null}"#.into()),
        "/api/v2/pins/42" => (200, r#"{"id":42,"code":"WXYZ","authToken":"user-token"}"#.into()),
        "/api/v2/pins/43" => (404, "{}".into()),
        _ => (500, "{}".into()),
    });
    let plex = Plex::new(ClientInfo::new("cid-1", "Den")).with_base_urls(&base, &base);
    let pin = plex.create_pin(false).await.unwrap();
    assert_eq!(pin.code, "WXYZ");
    assert_eq!(plex.wait_for_pin(&pin).await.unwrap(), "user-token");
    let gone = Pin { id: 43, ..pin };
    assert!(matches!(plex.check_pin(&gone).await, Err(Error::PinExpired)));

    let head = seen.lock().unwrap()[0].to_ascii_lowercase();
    assert!(head.contains("x-plex-client-identifier: cid-1"));
    assert!(head.contains("x-plex-product: encheo"));
    assert!(head.contains("accept: application/json"));
}

fn conn() -> Connection {
    Connection { uri: String::new(), protocol: "http".into(), address: String::new(), port: 0, local: false, relay: false, ipv6: false }
}

#[tokio::test]
async fn servers_then_connect_skips_dead_address() {
    let (base, _) = mock(|path| match path {
        "/identity" => (200, r#"{"MediaContainer":{"machineIdentifier":"abc"}}"#.into()),
        "/library/sections" => (200, include_str!("../tests/fixtures/sections.json").into()),
        _ => (404, "{}".into()),
    });
    let plex = Plex::new(ClientInfo::new("cid", "Den"));
    let resource = Resource {
        name: "Home".into(),
        client_identifier: "abc".into(),
        provides: "server".into(),
        owned: true,
        access_token: Some("t".into()),
        connections: vec![
            // Local but nothing listens there: must be skipped.
            Connection { uri: "http://127.0.0.1:9".into(), local: true, ..conn() },
            Connection { uri: base.clone(), ..conn() },
        ],
    };
    let server = plex.connect(&resource).await.unwrap();
    assert_eq!(server.base_url(), base);
    let libs = server.libraries().await.unwrap();
    assert_eq!(libs.iter().map(|l| l.title.as_str()).collect::<Vec<_>>(), ["Movies", "TV Shows", "Music"]);
}

#[test]
fn music_tracks_parse() {
    let p = page(include_str!("../tests/fixtures/music.json"));
    let t = &p.metadata[0];
    assert_eq!(t.kind, ItemKind::Track);
    assert_eq!(t.parent_rating_key.as_deref(), Some("900"));
    assert_eq!(t.parent_thumb.as_deref(), Some("/library/metadata/900/thumb/1"));
    assert_eq!(t.artist(), Some("Massive Attack"));
    assert_eq!(t.primary_part().unwrap().key, "/library/parts/88/1/03.flac");
    assert_eq!(p.metadata[1].artist(), Some("Massive Attack feat. Shara Nelson"));
}

#[test]
fn search_hubs_include_shows_from_directory() {
    let json = r#"{"MediaContainer":{"Hub":[
        {"type":"movie","Metadata":[{"ratingKey":"1","type":"movie","title":"Heat"}]},
        {"type":"show","Directory":[{"ratingKey":"2","type":"show","title":"The 'Burbs"}]},
        {"type":"actor","Directory":[{"id":3,"tag":"Al Pacino","type":"person"}]},
        {"type":"album","Metadata":[{"ratingKey":"4","type":"album","title":"24K Magic"}]}
    ]}}"#;
    let hubs: Envelope<crate::model::Hubs> = serde_json::from_str(json).unwrap();
    let titles: Vec<String> = crate::server::flatten_hubs(hubs.container).into_iter().map(|i| i.title).collect();
    assert_eq!(titles, ["Heat", "The 'Burbs", "24K Magic"]);
}

#[test]
fn show_preferences_parse() {
    let json = r#"{"MediaContainer":{"Metadata":[{"ratingKey":"9","Preferences":{"Setting":[
        {"id":"episodeSort","value":-1,"enumValues":"-1:Library default|0:Oldest first"},
        {"id":"audioLanguage","value":"ja","enumValues":":Account default|ja:日本語|en:English|es-419:Español (Latinoamérica)"},
        {"id":"subtitleMode","value":"-1","enumValues":"-1:Account default|0:Manually selected|2:Always enabled"}
    ]}}]}}"#;
    let e: Envelope<crate::model::WithPreferences> = serde_json::from_str(json).unwrap();
    let settings = e.container.metadata.into_iter().next().unwrap().preferences.unwrap().settings;
    assert_eq!(settings[0].value, "-1"); // numbers become strings
    assert_eq!(settings[1].value, "ja");
    assert_eq!(settings[1].choices().collect::<Vec<_>>(), ["", "ja", "en", "es-419"]);
}

#[tokio::test]
async fn watchlist_and_guid_lookup() {
    let (base, _) = mock(|path| match path {
        p if p.starts_with("/library/sections/watchlist/all") => (
            200,
            r#"{"MediaContainer":{"size":1,"Metadata":[{"ratingKey":"66","type":"movie","title":"Blades","year":2026,"guid":"plex://movie/66","thumb":"https://metadata-static.plex.tv/x.jpg"}]}}"#.into(),
        ),
        p if p.starts_with("/library/all?guid=plex%3A%2F%2Fmovie%2F66") => (200, r#"{"MediaContainer":{"size":1,"Metadata":[{"ratingKey":"45028","type":"movie","title":"Blades"}]}}"#.into()),
        p if p.starts_with("/library/all?guid=") => (200, r#"{"MediaContainer":{"size":0}}"#.into()),
        _ => (404, "{}".into()),
    });
    let plex = Plex::new(ClientInfo::new("cid", "Den")).with_base_urls(&base, &base);
    let list = plex.watchlist("tok").await.unwrap();
    assert_eq!(list[0].guid.as_deref(), Some("plex://movie/66"));
    let server = plex.server_at("Home", &base, "t");
    assert_eq!(server.find_by_guid("plex://movie/66").await.unwrap().unwrap().rating_key, "45028");
    assert!(server.find_by_guid("plex://movie/999").await.unwrap().is_none());
}
