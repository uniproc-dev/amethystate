use amethystate_core::event_channel;
use amethystate_core::path::StorePath;

#[derive(serde::Deserialize)]
struct Spelled {
    levels: Vec<String>,
    joined: String,
    channel: String,
}

fn spelled() -> Vec<Spelled> {
    serde_json::from_str(include_str!("fixtures/paths.json")).unwrap()
}

#[test]
fn a_path_is_joined_and_named_as_the_fixture_spells_it() {
    for one in spelled() {
        let path = StorePath::from_segments(&one.levels);

        assert_eq!(
            serde_json::to_value(&path).unwrap(),
            serde_json::json!(one.joined),
            "joined form of {:?}",
            one.levels
        );
        assert_eq!(
            event_channel(&path),
            one.channel,
            "channel of {:?}",
            one.levels
        );
    }
}

#[test]
fn no_two_paths_share_a_channel() {
    let mut channels: Vec<String> = spelled().into_iter().map(|one| one.channel).collect();
    let all = channels.len();
    channels.sort();
    channels.dedup();

    assert_eq!(channels.len(), all);
}

#[test]
fn a_channel_holds_only_what_tauri_accepts_in_an_event_name() {
    for one in spelled() {
        let name = event_channel(&StorePath::from_segments(&one.levels));

        assert!(
            name.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '/' | ':' | '_')),
            "{name}"
        );
    }
}
