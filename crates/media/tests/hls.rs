use mohona_media::hls::{parse, Playlist};
use mohona_media::{ByteRange, Encryption, MediaError, MediaKind};
use url::Url;

fn base() -> Url {
    Url::parse("https://cdn.example.com/video/master.m3u8").unwrap()
}

const MASTER: &str = r#"#EXTM3U
#EXT-X-VERSION:6
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="aud",NAME="English",LANGUAGE="en",DEFAULT=YES,URI="audio/en.m3u8"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="aud",NAME="Bangla",LANGUAGE="bn",URI="audio/bn.m3u8"
#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID="subs",NAME="English",LANGUAGE="en",URI="subs/en.m3u8"
#EXT-X-MEDIA:TYPE=CLOSED-CAPTIONS,GROUP-ID="cc",NAME="CC",INSTREAM-ID="CC1"
#EXT-X-STREAM-INF:BANDWIDTH=2500000,AVERAGE-BANDWIDTH=2000000,RESOLUTION=1280x720,FRAME-RATE=29.97,CODECS="avc1.4d401f,mp4a.40.2",AUDIO="aud",SUBTITLES="subs"
720p/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS="avc1.42c01e,mp4a.40.2",AUDIO="aud"
360p/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS="mp4a.40.2"
https://other.example.org/audio-only.m3u8
"#;

#[test]
fn master_lists_variants_and_tracks() {
    let Playlist::Master { variants, tracks } = parse(MASTER, &base()).unwrap() else {
        panic!("expected a master playlist");
    };
    assert_eq!(variants.len(), 3);
    let hd = &variants[0];
    assert_eq!(
        hd.url.as_str(),
        "https://cdn.example.com/video/720p/index.m3u8"
    );
    assert_eq!(hd.bandwidth, 2_500_000);
    assert_eq!(hd.average_bandwidth, Some(2_000_000));
    assert_eq!(hd.resolution.unwrap().height, 720);
    assert_eq!(hd.frame_rate, Some(29.97));
    // The comma inside the quoted codec list must not split it.
    assert_eq!(hd.codecs.as_deref(), Some("avc1.4d401f,mp4a.40.2"));
    assert_eq!(hd.audio_group.as_deref(), Some("aud"));
    assert_eq!(hd.subtitle_group.as_deref(), Some("subs"));
    assert_eq!(hd.label(), "720p 2.5 Mbit/s");
    assert!(!hd.is_audio_only());
    assert!(variants[2].is_audio_only());
    assert_eq!(variants[2].url.host_str(), Some("other.example.org"));

    // Closed captions are inside the video, so they are not tracks.
    assert_eq!(tracks.len(), 3);
    assert_eq!(tracks[0].kind, MediaKind::Audio);
    assert!(tracks[0].default);
    assert_eq!(tracks[1].language.as_deref(), Some("bn"));
    assert_eq!(
        tracks[1].url.as_ref().unwrap().as_str(),
        "https://cdn.example.com/video/audio/bn.m3u8"
    );
    assert_eq!(tracks[2].kind, MediaKind::Subtitles);
}

#[test]
fn media_playlist_lists_segments() {
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXT-X-MEDIA-SEQUENCE:7\n\
        #EXTINF:9.009,first\nseg7.ts\n#EXTINF:9.009,\nseg8.ts\n\
        #EXT-X-DISCONTINUITY\n#EXTINF:4.5,\nhttps://ads.example.net/a.ts\n#EXT-X-ENDLIST\n";
    let Playlist::Media(list) = parse(text, &base()).unwrap() else {
        panic!("expected a media playlist");
    };
    assert!(list.ended);
    assert_eq!(list.target_duration, 10.0);
    assert_eq!(list.segments.len(), 3);
    assert_eq!(list.segments[0].sequence, 7);
    assert_eq!(
        list.segments[0].url.as_str(),
        "https://cdn.example.com/video/seg7.ts"
    );
    assert!(!list.segments[1].discontinuity);
    assert!(list.segments[2].discontinuity);
    assert_eq!(list.segments[2].sequence, 9);
    assert!((list.total_duration() - 22.518).abs() < 1e-9);
}

#[test]
fn live_playlist_is_not_ended() {
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6,\na.ts\n";
    let Playlist::Media(list) = parse(text, &base()).unwrap() else {
        panic!()
    };
    assert!(!list.ended);
}

#[test]
fn byte_ranges_continue_from_the_previous_piece() {
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n\
        #EXTINF:4,\n#EXT-X-BYTERANGE:1000@200\nmovie.ts\n\
        #EXTINF:4,\n#EXT-X-BYTERANGE:500\nmovie.ts\n\
        #EXTINF:4,\n#EXT-X-BYTERANGE:300\nother.ts\n#EXT-X-ENDLIST\n";
    let Playlist::Media(list) = parse(text, &base()).unwrap() else {
        panic!()
    };
    assert_eq!(
        list.segments[0].range,
        Some(ByteRange {
            offset: 200,
            length: 1000
        })
    );
    assert_eq!(
        list.segments[1].range,
        Some(ByteRange {
            offset: 1200,
            length: 500
        })
    );
    // A different file starts at 0 when no offset is given.
    assert_eq!(
        list.segments[2].range,
        Some(ByteRange {
            offset: 0,
            length: 300
        })
    );
}

#[test]
fn init_segment_applies_until_replaced() {
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n\
        #EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"720@0\"\n#EXTINF:4,\ns1.m4s\n#EXTINF:4,\ns2.m4s\n\
        #EXT-X-MAP:URI=\"init2.mp4\"\n#EXTINF:4,\ns3.m4s\n#EXT-X-ENDLIST\n";
    let Playlist::Media(list) = parse(text, &base()).unwrap() else {
        panic!()
    };
    let first = list.segments[0].init.as_ref().unwrap();
    assert_eq!(first.url.path(), "/video/init.mp4");
    assert_eq!(
        first.range,
        Some(ByteRange {
            offset: 0,
            length: 720
        })
    );
    assert_eq!(list.segments[1].init, list.segments[0].init);
    assert_eq!(
        list.segments[2].init.as_ref().unwrap().url.path(),
        "/video/init2.mp4"
    );
    assert_eq!(list.segments[2].init.as_ref().unwrap().range, None);
}

#[test]
fn aes_128_key_and_iv() {
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:258\n\
        #EXT-X-KEY:METHOD=AES-128,URI=\"keys/k1.bin\"\n#EXTINF:4,\na.ts\n\
        #EXT-X-KEY:METHOD=AES-128,URI=\"keys/k2.bin\",IV=0x000102030405060708090a0b0c0d0e0f\n#EXTINF:4,\nb.ts\n\
        #EXT-X-KEY:METHOD=NONE\n#EXTINF:4,\nc.ts\n#EXT-X-ENDLIST\n";
    let Playlist::Media(list) = parse(text, &base()).unwrap() else {
        panic!()
    };
    let Some(Encryption::Aes128 { key_url, iv }) = &list.segments[0].encryption else {
        panic!("first segment should be encrypted")
    };
    assert_eq!(key_url.path(), "/video/keys/k1.bin");
    assert_eq!(*iv, None);
    // Without an IV the segment number (258 = 0x102) is the IV.
    let derived = list.segments[0].iv().unwrap();
    assert_eq!(&derived[..8], &[0u8; 8]);
    assert_eq!(&derived[8..], &[0, 0, 0, 0, 0, 0, 1, 2]);
    let explicit = list.segments[1].iv().unwrap();
    assert_eq!(explicit[0], 0x00);
    assert_eq!(explicit[15], 0x0f);
    assert!(list.segments[2].encryption.is_none());
    assert!(list.segments[2].iv().is_none());
}

#[test]
fn drm_is_refused_with_a_clear_message() {
    for key in [
        "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\",KEYFORMAT=\"com.apple.streamingkeydelivery\"",
        "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"data:text/plain;base64,AAAA\"",
        "#EXT-X-KEY:METHOD=AES-128,URI=\"k\",KEYFORMAT=\"urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed\"",
    ] {
        let text = format!("#EXTM3U\n#EXT-X-TARGETDURATION:4\n{key}\n#EXTINF:4,\na.ts\n");
        let err = parse(&text, &base()).unwrap_err();
        assert!(matches!(err, MediaError::Protected(_)), "{key}: {err:?}");
        assert!(err.to_string().contains("DRM"));
    }
}

#[test]
fn bad_input_gets_specific_errors() {
    assert_eq!(
        parse("<html></html>", &base()).unwrap_err(),
        MediaError::NotAPlaylist
    );
    assert_eq!(
        parse("#EXTM3U\n#EXT-X-ENDLIST\n", &base()).unwrap_err(),
        MediaError::Empty
    );
    let err = parse("#EXTM3U\n#EXTINF:abc,\na.ts\n", &base()).unwrap_err();
    assert!(matches!(err, MediaError::Invalid(_)));
    let err = parse(
        "#EXTM3U\n#EXT-X-STREAM-INF:RESOLUTION=1x1\na.m3u8\n",
        &base(),
    )
    .unwrap_err();
    assert!(matches!(err, MediaError::Invalid(m) if m.contains("BANDWIDTH")));
    // A byte-order mark and Windows line endings are fine.
    let text = "\u{feff}#EXTM3U\r\n#EXT-X-TARGETDURATION:4\r\n#EXTINF:4,\r\na.ts\r\n";
    assert!(parse(text, &base()).is_ok());
}
