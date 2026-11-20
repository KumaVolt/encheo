use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

/// What the pretend Seerr saw: method and path, the Cookie header, the body.
#[derive(Clone, Debug)]
struct Seen {
    line: String,
    cookie: String,
    xsrf: String,
    body: String,
}

type Reply = (u16, &'static str, String);

/// A one-thread HTTP server: `respond(method and path, cookie) -> (status, extra headers, body)`.
fn mock(respond: fn(&str, &str) -> Reply) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", l.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    std::thread::spawn(move || {
        for stream in l.incoming() {
            let mut stream = stream.unwrap();
            let mut r = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            r.read_line(&mut first).unwrap();
            let (mut len, mut cookie, mut xsrf) = (0usize, String::new(), String::new());
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                let value = line.split_once(':').map(|(_, v)| v.trim().to_string()).unwrap_or_default();
                if lower.starts_with("content-length:") {
                    len = value.parse().unwrap_or(0);
                } else if lower.starts_with("cookie:") {
                    cookie = value;
                } else if lower.starts_with("x-xsrf-token:") {
                    xsrf = value;
                }
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            let line: String = first.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
            let (status, headers, reply) = respond(&line, &cookie);
            seen2.lock().unwrap().push(Seen { line, cookie, xsrf, body: String::from_utf8_lossy(&body).into() });
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    (addr, seen)
}

const SETTINGS: &str = r#"{"initialized":true,"applicationTitle":"Seerr","mediaServerType":1,"localLogin":true,"mediaServerLogin":true,"movie4kEnabled":false,"series4kEnabled":true,"partialRequestsEnabled":true,"plexClientIdentifier":"x"}"#;
const ME: &str = r#"{"id":7,"displayName":"Sam","email":"sam@example.com","permissions":32}"#;

fn signed_in(cookie: &str) -> bool {
    cookie.split("; ").any(|c| c == "connect.sid=s%3Aabc.def")
}

#[test]
fn addresses_people_type() {
    assert_eq!(candidates("requests.example.com"), ["https://requests.example.com", "http://requests.example.com"]);
    assert_eq!(candidates(" http://192.168.1.5:5055/ "), ["http://192.168.1.5:5055"]);
    assert_eq!(candidates("https://home.example/seerr/api/v1/"), ["https://home.example/seerr"]);
    assert!(candidates("").is_empty());
    assert!(candidates("ftp://nas").is_empty());
    assert!(candidates("two words").is_empty());
}

#[tokio::test]
async fn connect_signs_in_and_keeps_the_session() {
    let (base, seen) = mock(|line, cookie| match line {
        "GET /api/v1/settings/public" => (200, "", SETTINGS.into()),
        "POST /api/v1/auth/plex" => (200, "Set-Cookie: connect.sid=s%3Aabc.def; Path=/; Expires=Wed, 20 Dec 2026 10:00:00 GMT; HttpOnly; SameSite=Lax\r\n", ME.into()),
        "GET /api/v1/auth/me" if signed_in(cookie) => (200, "", ME.into()),
        "GET /api/v1/auth/me" => (403, "", r#"{"status":403,"error":"You do not have permission to access this endpoint"}"#.into()),
        _ => (404, "", "{}".into()),
    });
    let seerr = Seerr::connect(&base).await.unwrap();
    let settings = seerr.public_settings().await.unwrap();
    assert!(settings.uses_plex() && settings.series_4k_enabled && !settings.movie_4k_enabled);

    assert!(matches!(seerr.me().await, Err(Error::SignedOut)));
    let me = seerr.sign_in_plex("plex-token").await.unwrap();
    assert_eq!((me.id, me.display_name.as_str()), (7, "Sam"));
    assert_eq!(seerr.session(), "connect.sid=s%3Aabc.def");

    // A kept session works in a new connection.
    let again = Seerr::resume(seerr.address(), &seerr.session());
    assert_eq!(again.me().await.unwrap().id, 7);

    let seen = seen.lock().unwrap();
    let sign_in = seen.iter().find(|s| s.line == "POST /api/v1/auth/plex").unwrap();
    assert_eq!(sign_in.body, r#"{"authToken":"plex-token"}"#);
    assert!(seen.last().unwrap().cookie.contains("connect.sid=s%3Aabc.def"));
}

#[tokio::test]
async fn not_a_seerr() {
    let (base, _) = mock(|line, _| match line {
        "GET /api/v1/settings/public" => (200, "", r#"{"MediaContainer":{}}"#.into()),
        _ => (404, "", "".into()),
    });
    assert!(matches!(Seerr::connect(&base).await, Err(Error::NotSeerr)));
    let (base, _) = mock(|_, _| (404, "", "<html>nope</html>".into()));
    assert!(matches!(Seerr::connect(&base).await, Err(Error::NotSeerr)));
    assert!(matches!(Seerr::connect("  ").await, Err(Error::Address)));
}

#[tokio::test]
async fn wrong_password_is_a_refusal() {
    let (base, _) = mock(|line, _| match line {
        "POST /api/v1/auth/jellyfin" => (403, "", r#"{"message":"INVALID_CREDENTIALS"}"#.into()),
        "POST /api/v1/auth/local" => (403, "", r#"{"status":403,"error":"You do not have permission to access this endpoint"}"#.into()),
        _ => (404, "", "{}".into()),
    });
    let seerr = Seerr::resume(&base, "");
    match seerr.sign_in_jellyfin("sam", "wrong").await {
        Err(Error::Refused { status: 403, message }) => assert_eq!(message, "INVALID_CREDENTIALS"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(seerr.sign_in_local("a@b.c", "x").await, Err(Error::Refused { .. })));
}

#[tokio::test]
async fn search_keeps_films_and_shows() {
    let (base, seen) = mock(|line, _| {
        if line.starts_with("GET /api/v1/search?") {
            (200, "", r#"{"page":1,"totalPages":1,"totalResults":3,"results":[
                {"id":438631,"mediaType":"movie","title":"Dune","releaseDate":"2021-09-15","posterPath":"/d5NXSklXo0qyIYkgV94XAgMIckC.jpg","overview":"Paul…",
                 "mediaInfo":{"id":3,"tmdbId":438631,"status":5,"status4k":1,"requests":[],"downloadStatus":[],"downloadStatus4k":[]}},
                {"id":1,"mediaType":"person","name":"Zendaya"},
                {"id":90228,"mediaType":"tv","name":"Dune: Prophecy","firstAirDate":"2024-11-17","posterPath":null}
            ]}"#.into())
        } else {
            (404, "", "{}".into())
        }
    });
    let seerr = Seerr::resume(&base, "connect.sid=x");
    let found = seerr.search("dune: part two & more", 1).await.unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!((found[0].kind, found[0].tmdb_id, found[0].year), (Kind::Movie, 438631, Some(2021)));
    assert_eq!(found[0].state(false), State::Available);
    assert_eq!(found[0].state(true), State::NotRequested);
    assert_eq!((found[1].kind, found[1].title.as_str(), found[1].poster_path.as_deref()), (Kind::Tv, "Dune: Prophecy", None));
    assert_eq!(found[1].state(false), State::NotRequested);
    assert_eq!(seen.lock().unwrap()[0].line, "GET /api/v1/search?query=dune%3A%20part%20two%20%26%20more&page=1");
}

const SHOW: &str = r#"{"id":1399,"name":"Severance","firstAirDate":"2022-02-17","posterPath":"/p.jpg",
  "seasons":[{"seasonNumber":0,"name":"Specials","episodeCount":3},{"seasonNumber":1,"name":"Season 1","episodeCount":9},
             {"seasonNumber":2,"name":"Season 2","episodeCount":10},{"seasonNumber":3,"name":"Season 3","episodeCount":10}],
  "mediaInfo":{"status":4,"status4k":1,
    "seasons":[{"seasonNumber":1,"status":5,"status4k":1},{"seasonNumber":2,"status":3,"status4k":1}],
    "requests":[{"id":11,"status":2,"is4k":false,"seasons":[{"seasonNumber":2,"status":3}]},
                {"id":12,"status":3,"is4k":false,"seasons":[{"seasonNumber":3,"status":1}]}],
    "downloadStatus":[
      {"externalId":1,"size":2000,"sizeLeft":500,"status":"downloading","timeLeft":"00:04:10","estimatedCompletionTime":"2026-11-20T10:04:10.000Z","title":"Severance.S02E01","episode":{"seasonNumber":2,"episodeNumber":1}},
      {"externalId":1,"size":2000,"sizeLeft":1500,"status":"downloading","timeLeft":"00:21:43.123","estimatedCompletionTime":"2026-11-20T10:21:43.000Z","title":"Severance.S02E02","episode":{"seasonNumber":2,"episodeNumber":2}}],
    "downloadStatus4k":[]}}"#;

#[tokio::test]
async fn a_show_with_seasons_and_downloads() {
    let (base, _) = mock(|line, _| match line {
        "GET /api/v1/tv/1399" => (200, "", SHOW.into()),
        _ => (404, "", "{}".into()),
    });
    let show = Seerr::resume(&base, "connect.sid=x").title(Kind::Tv, 1399).await.unwrap();
    assert_eq!(show.seasons.iter().map(|s| s.number).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(show.season_state(1, false), SeasonState::Available);
    assert_eq!(show.season_state(2, false), SeasonState::Requested);
    // Season 3's request was declined: it can be asked for again.
    assert_eq!(show.season_state(3, false), SeasonState::Missing);
    assert_eq!(show.missing_seasons(false), [3]);
    assert_eq!(show.missing_seasons(true), [1, 2, 3]);

    let State::PartlyAvailable(Some(p)) = show.state(false) else { panic!("{:?}", show.state(false)) };
    assert_eq!((p.files, p.size, p.size_left), (2, 4000, 2000));
    assert_eq!(p.fraction(), Some(0.5));
    assert_eq!(p.time_left, Some(Duration::from_secs(21 * 60 + 43)));
    assert_eq!(p.estimated_completion.as_deref(), Some("2026-11-20T10:21:43.000Z"));
    assert!(!p.stalled);
}

#[test]
fn states() {
    let movie = |media: serde_json::Value| RawTitle { media_info: serde_json::from_value(media).ok(), ..Default::default() }.into_title(Kind::Movie);
    assert_eq!(movie(serde_json::Value::Null).state(false), State::NotRequested);
    assert_eq!(movie(serde_json::json!({"status": 2})).state(false), State::Requested);
    assert_eq!(movie(serde_json::json!({"status": 3})).state(false), State::Processing);
    assert_eq!(movie(serde_json::json!({"status": 6})).state(false), State::Blocked);
    assert_eq!(movie(serde_json::json!({"status": 1, "requests": [{"status": 1, "is4k": false}]})).state(false), State::Requested);
    assert_eq!(movie(serde_json::json!({"status": 1, "requests": [{"status": 2, "is4k": false}]})).state(false), State::Processing);
    assert_eq!(movie(serde_json::json!({"status": 1, "requests": [{"status": 3, "is4k": false}]})).state(false), State::NotRequested);
    assert_eq!(movie(serde_json::json!({"status": 1, "requests": [{"status": 1, "is4k": true}]})).state(false), State::NotRequested);
    let paused = movie(serde_json::json!({"status": 3, "downloadStatus": [{"size": 100, "sizeLeft": 100, "status": "paused"}]}));
    let State::Downloading(p) = paused.state(false) else { panic!() };
    assert!(p.stalled);
    assert_eq!(p.fraction(), Some(0.0));
    assert_eq!(p.time_left, None);
}

#[test]
fn times_left() {
    assert_eq!(parse_time_left("00:21:43"), Some(Duration::from_secs(1303)));
    assert_eq!(parse_time_left("00:21:43.5120000"), Some(Duration::from_secs(1303)));
    assert_eq!(parse_time_left("1.02:03:04"), Some(Duration::from_secs(93784)));
    assert_eq!(parse_time_left("2.00:00:00.25"), Some(Duration::from_secs(172_800)));
    assert_eq!(parse_time_left(""), None);
    assert_eq!(parse_time_left("soon"), None);
    assert_eq!(parse_time_left("1:2:3:4"), None);
}

#[test]
fn permissions() {
    let user = |permissions| User { permissions, ..Default::default() };
    assert!(user(2).can_request(Kind::Tv, true)); // admin
    assert!(user(32).can_request(Kind::Movie, false) && !user(32).can_request(Kind::Movie, true));
    assert!(user(262_144).can_request(Kind::Movie, false) && !user(262_144).can_request(Kind::Tv, false));
    assert!(user(1024).can_request(Kind::Tv, true));
    assert!(!user(0).can_request(Kind::Movie, false));
    assert!(user(128).auto_approved(Kind::Tv) && user(256).auto_approved(Kind::Movie) && !user(256).auto_approved(Kind::Tv));
}

#[tokio::test]
async fn requests() {
    let (base, seen) = mock(|line, cookie| match line {
        "POST /api/v1/request" if !cookie.is_empty() => (201, "", r#"{"id":31,"status":1,"is4k":false,"seasons":[{"seasonNumber":3}]}"#.into()),
        "POST /api/v1/request" => (401, "", r#"{"message":"You must be logged in to request media."}"#.into()),
        _ => (404, "", "{}".into()),
    });
    let seerr = Seerr::resume(&base, "connect.sid=x; XSRF-TOKEN=tok");
    let r = seerr.request(Kind::Tv, 1399, Seasons::Only(vec![3]), false).await.unwrap();
    assert_eq!((r.id, r.status), (31, RequestStatus::Pending));
    seerr.request(Kind::Movie, 438631, Seasons::All, true).await.unwrap();
    seerr.request(Kind::Tv, 1399, Seasons::All, false).await.unwrap();
    assert!(matches!(Seerr::resume(&base, "").request(Kind::Movie, 1, Seasons::All, false).await, Err(Error::SignedOut)));

    let seen = seen.lock().unwrap();
    let body = |i: usize| serde_json::from_str::<serde_json::Value>(&seen[i].body).unwrap();
    assert_eq!(body(0), serde_json::json!({"mediaType": "tv", "mediaId": 1399, "is4k": false, "seasons": [3]}));
    assert_eq!(body(1), serde_json::json!({"mediaType": "movie", "mediaId": 438631, "is4k": true}));
    assert_eq!(body(2)["seasons"], "all");
    assert_eq!(seen[0].xsrf, "tok");
}

#[tokio::test]
async fn refusals_carry_seerrs_reason() {
    let (base, _) = mock(|line, _| match line {
        "POST /api/v1/request" => (409, "", r#"{"message":"Request for this media already exists."}"#.into()),
        _ => (404, "", "{}".into()),
    });
    let seerr = Seerr::resume(&base, "connect.sid=x");
    match seerr.request(Kind::Movie, 1, Seasons::All, false).await {
        Err(Error::Refused { status: 409, message }) => assert_eq!(message, "Request for this media already exists."),
        other => panic!("{other:?}"),
    }
    let (base, _) = mock(|_, _| (202, "", r#"{"message":"No seasons available to request"}"#.into()));
    match Seerr::resume(&base, "connect.sid=x").request(Kind::Tv, 1, Seasons::All, false).await {
        Err(Error::Refused { status: 202, message }) => assert_eq!(message, "No seasons available to request"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn cookies_stay_cookies() {
    let s = Seerr::resume("http://h", "connect.sid=a; bad cookie=1; x=y\r\nInjected: 1; ; =v; empty=");
    assert_eq!(s.session(), "connect.sid=a");
}
