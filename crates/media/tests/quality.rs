use mohona_media::hls::{parse, Playlist};
use mohona_media::quality::*;
use mohona_media::MediaKind;
use url::Url;

fn master() -> (Vec<mohona_media::MediaVariant>, Vec<mohona_media::Track>) {
    let text = r#"#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="a",NAME="English",LANGUAGE="en",DEFAULT=YES,URI="en.m3u8"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="a",NAME="Bangla",LANGUAGE="bn",URI="bn.m3u8"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="other",NAME="Other",LANGUAGE="fr",URI="fr.m3u8"
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS="avc1.42c01e,mp4a.40.2",AUDIO="a"
v360.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,CODECS="avc1.640028,mp4a.40.2",AUDIO="a"
v1080.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1280x720,CODECS="avc1.4d401f,mp4a.40.2",AUDIO="a"
v720.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=64000,CODECS="mp4a.40.5"
audio.m3u8
"#;
    match parse(text, &Url::parse("https://e.com/m.m3u8").unwrap()).unwrap() {
        Playlist::Master { variants, tracks } => (variants, tracks),
        _ => panic!(),
    }
}

#[test]
fn quality_words_are_parsed() {
    assert_eq!(Quality::parse("best"), Ok(Quality::Best));
    assert_eq!(Quality::parse("WORST"), Ok(Quality::Worst));
    assert_eq!(Quality::parse("audio"), Ok(Quality::AudioOnly));
    assert_eq!(Quality::parse("720"), Ok(Quality::Height(720)));
    assert_eq!(Quality::parse("1080p"), Ok(Quality::Height(1080)));
    assert!(Quality::parse("0").is_err());
    assert!(Quality::parse("hd").unwrap_err().contains("not a quality"));
}

#[test]
fn variants_are_picked_by_quality() {
    let (v, _) = master();
    let name = |q| v[pick_variant(&v, q).unwrap()].url.path().to_string();
    assert_eq!(name(Quality::Best), "/v1080.m3u8");
    assert_eq!(name(Quality::Worst), "/v360.m3u8"); // not the audio-only one
    assert_eq!(name(Quality::Height(720)), "/v720.m3u8");
    assert_eq!(name(Quality::Height(900)), "/v720.m3u8"); // not taller than asked
    assert_eq!(name(Quality::Height(144)), "/v360.m3u8"); // all taller: the smallest
    assert_eq!(name(Quality::AudioOnly), "/audio.m3u8");
}

#[test]
fn tracks_follow_group_then_language_then_default() {
    let (_, t) = master();
    let url = |l: Option<&str>| {
        pick_track(&t, MediaKind::Audio, "a", l).map(|t| t.url.as_ref().unwrap().path().to_string())
    };
    assert_eq!(url(None).as_deref(), Some("/en.m3u8")); // the default
    assert_eq!(url(Some("bn")).as_deref(), Some("/bn.m3u8"));
    assert_eq!(url(Some("Bangla")).as_deref(), Some("/bn.m3u8"));
    assert_eq!(url(Some("en-US")).as_deref(), Some("/en.m3u8"));
    assert_eq!(url(Some("fr")), None); // French is in another group
    assert!(pick_track(&t, MediaKind::Audio, "nope", None).is_none());
}
