//! cargo run -p encheo-jellyfin --example probe -- <url> <user> <password>
use encheo_jellyfin::*;

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let client = ClientInfo { device_id: "probe".into(), device_name: "probe".into(), version: "0.1".into() };
    let login = sign_in(&a[0], &a[1], &a[2], &client).await.expect("sign in");
    println!("signed in to {} ({:?}) as {}", login.server_name, login.flavor, login.user_name);
    let s = Server::new(login, client, "jf0:");
    let libs = s.libraries().await.expect("libraries");
    for l in &libs {
        let page = s.library_items(&l.key, &l.kind, 0, 50).await.expect("items");
        println!("library {} ({}): {} items", l.title, l.kind, page.items.len());
        for i in &page.items {
            println!("  {:?} {} ({:?}) thumb={}", i.kind, i.title, i.year, i.thumb.is_some());
            let full = s.metadata(&i.rating_key).await.expect("metadata").expect("item");
            println!("    media: {:?} {:?} parts={}", full.media.first().map(|m| &m.video_codec), full.media.first().map(|m| &m.video_resolution), full.media.first().map_or(0, |m| m.parts.len()));
            if full.kind == encheo_plex::ItemKind::Show {
                for season in s.children(&full.rating_key).await.expect("seasons") {
                    let eps = s.children(&season.rating_key).await.expect("episodes");
                    println!("    {} → {} episodes: {:?}", season.title, eps.len(), eps.iter().map(|e| e.episode_label()).collect::<Vec<_>>());
                    if let Some(e) = eps.iter().find(|e| e.duration.unwrap_or(0) > 300_000) {
                        let full = s.metadata(&e.rating_key).await.unwrap().unwrap();
                        let url = s.stream_url(&full);
                        let r = reqwest::Client::new().get(&url).header("Range", "bytes=0-1023").send().await.unwrap();
                        println!("    stream {} → {} ({} bytes)", e.title, r.status(), r.bytes().await.unwrap().len());
                        s.report_start(&full).await.expect("start");
                        s.report_progress(&full, encheo_plex::PlayState::Playing, 125_000).await.expect("progress");
                        s.report_progress(&full, encheo_plex::PlayState::Stopped, 125_000).await.expect("stop");
                        let after = s.metadata(&e.rating_key).await.unwrap().unwrap();
                        println!("    resume point after report: {:?} ms", after.view_offset);
                    }
                }
                println!("    all episodes: {}", s.all_episodes(&full.rating_key).await.expect("all").len());
            }
        }
        println!("  recently added: {}", s.recently_added(&l.key, 10).await.expect("latest").len());
    }
    println!("on deck: {:?}", s.on_deck().await.expect("on deck").iter().map(|i| &i.title).collect::<Vec<_>>());
    println!("search 'test': {:?}", s.search("test").await.expect("search").iter().map(|i| (&i.kind, &i.title)).collect::<Vec<_>>());
}
