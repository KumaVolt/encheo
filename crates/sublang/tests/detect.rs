use encheo_sublang::{Budget, detect, untagged_subtitle_text};

#[test]
fn finds_and_identifies_untagged_tracks() {
    let file = std::fs::File::open("tests/fixtures/untagged.mkv").unwrap();
    let budget = Budget { start_fraction: 0.0, chars_per_track: 100, ..Budget::default() };
    let texts = untagged_subtitle_text(file, &budget).unwrap();
    // Stream 3 is tagged French and must be left alone.
    assert_eq!(texts.keys().copied().collect::<Vec<_>>(), [1, 2]);
    assert!(texts[&1].starts_with("As noites são longas"), "{:?}", texts[&1]);
    assert_eq!(detect(&texts[&1]).unwrap().code, "por");
    assert_eq!(detect(&texts[&2]).unwrap().code, "eng");
}

#[test]
fn byte_budget_stops_early() {
    let file = std::fs::File::open("tests/fixtures/untagged.mkv").unwrap();
    let budget = Budget { start_fraction: 0.0, chars_per_track: 100, max_bytes: 1 };
    let texts = untagged_subtitle_text(file, &budget).unwrap();
    assert!(texts.values().map(|t| t.len()).sum::<usize>() < 200, "{texts:?}");
}

#[test]
fn rejects_non_matroska() {
    let file = std::io::Cursor::new(b"not a video".to_vec());
    assert!(untagged_subtitle_text(file, &Budget::default()).is_err());
}
