use mohona_media::dash::parse;
use mohona_media::{ByteRange, MediaError, MediaKind};
use url::Url;

fn base() -> Url {
    Url::parse("https://cdn.example.com/show/manifest.mpd").unwrap()
}

fn mpd(attrs: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" {attrs}>{body}</MPD>"#
    )
}

#[test]
fn template_by_number_with_padding_and_ids() {
    let text = mpd(
        r#"type="static" mediaPresentationDuration="PT25S""#,
        r#"<Period>
          <AdaptationSet contentType="video" mimeType="video/mp4" codecs="avc1.4d401f" width="1280" height="720">
            <SegmentTemplate timescale="1000" duration="10000" startNumber="1"
               initialization="$RepresentationID$/init.mp4" media="$RepresentationID$/seg-$Number%05d$.m4s"/>
            <Representation id="v720" bandwidth="2500000"/>
            <Representation id="v360" bandwidth="800000" width="640" height="360"/>
          </AdaptationSet>
          <AdaptationSet contentType="audio" lang="en" mimeType="audio/mp4" codecs="mp4a.40.2">
            <SegmentTemplate timescale="48000" duration="480000" initialization="a/init.mp4" media="a/$Number$.m4s"/>
            <Representation id="a1" bandwidth="128000"/>
          </AdaptationSet>
        </Period>"#,
    );
    let r = parse(&text, &base()).unwrap();
    assert_eq!(r.len(), 3);
    let v = &r[0];
    assert_eq!(v.kind, MediaKind::Video);
    assert_eq!(v.resolution.unwrap().height, 720);
    assert_eq!(v.codecs.as_deref(), Some("avc1.4d401f"));
    // 25 s in 10 s pieces: 3 segments (the last one is short).
    assert_eq!(v.segments.segments.len(), 3);
    assert_eq!(
        v.segments.segments[0].url.as_str(),
        "https://cdn.example.com/show/v720/seg-00001.m4s"
    );
    assert_eq!(
        v.segments.segments[2].url.path(),
        "/show/v720/seg-00003.m4s"
    );
    assert_eq!(
        v.segments.segments[0].init.as_ref().unwrap().url.path(),
        "/show/v720/init.mp4"
    );
    assert_eq!(v.segments.segments[0].duration, 10.0);
    assert_eq!(r[1].resolution.unwrap().width, 640); // own size beats the set's
    let a = &r[2];
    assert_eq!(a.kind, MediaKind::Audio);
    assert_eq!(a.language.as_deref(), Some("en"));
    assert_eq!(a.segments.segments.len(), 3);
    assert_eq!(a.segments.segments[0].duration, 10.0);
    assert_eq!(a.segments.segments[2].sequence, 2);
}

#[test]
fn timeline_with_repeats_and_time_names() {
    let text = mpd(
        r#"type="static" mediaPresentationDuration="PT30S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4">
          <SegmentTemplate timescale="90000" media="v_$Time$.m4s" initialization="init.mp4">
            <SegmentTimeline>
              <S t="0" d="450000" r="1"/>
              <S d="90000"/>
              <S d="450000" r="-1"/>
            </SegmentTimeline>
          </SegmentTemplate>
          <Representation id="x" bandwidth="1"/>
        </AdaptationSet></Period>"#,
    );
    let r = parse(&text, &base()).unwrap();
    let s = &r[0].segments.segments;
    // 5 s, 5 s, 1 s, then 5 s pieces until 30 s: (30-11)/5 -> 4 more.
    assert_eq!(s.len(), 3 + 4);
    assert_eq!(s[0].url.path(), "/show/v_0.m4s");
    assert_eq!(s[1].url.path(), "/show/v_450000.m4s");
    assert_eq!(s[2].url.path(), "/show/v_900000.m4s");
    assert_eq!(s[2].duration, 1.0);
    assert_eq!(s[3].url.path(), "/show/v_990000.m4s");
}

#[test]
fn periods_are_joined_and_base_urls_nest() {
    let text = mpd(
        r#"type="static" mediaPresentationDuration="PT20S""#,
        r#"<BaseURL>media/</BaseURL>
        <Period id="1" duration="PT10S"><AdaptationSet mimeType="video/mp4">
          <SegmentTemplate duration="5" media="p1-$Number$.m4s"/>
          <Representation id="r" bandwidth="1"><BaseURL>hd/</BaseURL></Representation>
        </AdaptationSet></Period>
        <Period id="2" start="PT10S"><AdaptationSet mimeType="video/mp4">
          <SegmentTemplate duration="5" media="p2-$Number$.m4s"/>
          <Representation id="r" bandwidth="1"><BaseURL>hd/</BaseURL></Representation>
        </AdaptationSet></Period>"#,
    );
    let r = parse(&text, &base()).unwrap();
    assert_eq!(r.len(), 1);
    let s = &r[0].segments.segments;
    assert_eq!(s.len(), 4);
    assert_eq!(
        s[0].url.as_str(),
        "https://cdn.example.com/show/media/hd/p1-1.m4s"
    );
    assert_eq!(s[2].url.path(), "/show/media/hd/p2-1.m4s");
    assert!(s[2].discontinuity && !s[1].discontinuity);
    assert_eq!(s[3].sequence, 3);
}

#[test]
fn segment_base_is_one_file_and_segment_list_has_ranges() {
    let text = mpd(
        r#"type="static" mediaPresentationDuration="PT1M""#,
        r#"<Period>
          <AdaptationSet mimeType="video/mp4">
            <Representation id="whole" bandwidth="5"><BaseURL>movie.mp4</BaseURL>
              <SegmentBase indexRange="800-1500"><Initialization range="0-799"/></SegmentBase>
            </Representation>
          </AdaptationSet>
          <AdaptationSet mimeType="audio/mp4">
            <Representation id="listed" bandwidth="5"><BaseURL>audio.mp4</BaseURL>
              <SegmentList duration="30">
                <Initialization range="0-99"/>
                <SegmentURL mediaRange="100-599"/>
                <SegmentURL mediaRange="600-999"/>
              </SegmentList>
            </Representation>
          </AdaptationSet>
        </Period>"#,
    );
    let r = parse(&text, &base()).unwrap();
    let whole = &r[0].segments.segments;
    assert_eq!(whole.len(), 1);
    assert_eq!(whole[0].url.path(), "/show/movie.mp4");
    assert_eq!(whole[0].duration, 60.0);
    let listed = &r[1].segments.segments;
    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed[1].range,
        Some(ByteRange {
            offset: 600,
            length: 400
        })
    );
    assert_eq!(
        listed[0].init.as_ref().unwrap().range,
        Some(ByteRange {
            offset: 0,
            length: 100
        })
    );
    assert_eq!(listed[0].duration, 30.0);
}

#[test]
fn subtitles_are_recognised() {
    let text = mpd(
        r#"type="static" mediaPresentationDuration="PT10S""#,
        r#"<Period><AdaptationSet contentType="text" lang="bn" mimeType="text/vtt">
          <Representation id="s" bandwidth="1"><BaseURL>bn.vtt</BaseURL></Representation>
        </AdaptationSet></Period>"#,
    );
    let r = parse(&text, &base()).unwrap();
    assert_eq!(r[0].kind, MediaKind::Subtitles);
    assert_eq!(r[0].language.as_deref(), Some("bn"));
}

#[test]
fn drm_live_and_bad_input_are_reported() {
    let drm = mpd(
        r#"type="static" mediaPresentationDuration="PT10S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4">
          <ContentProtection schemeIdUri="urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed"/>
          <SegmentTemplate duration="5" media="$Number$.m4s"/><Representation id="r" bandwidth="1"/>
        </AdaptationSet></Period>"#,
    );
    let err = parse(&drm, &base()).unwrap_err();
    assert!(matches!(err, MediaError::Protected(_)));
    assert!(err.to_string().contains("DRM"));

    let live = mpd(r#"type="dynamic""#, "<Period/>");
    assert!(
        matches!(parse(&live, &base()).unwrap_err(), MediaError::Invalid(m) if m.contains("live"))
    );

    assert!(matches!(
        parse("not xml", &base()).unwrap_err(),
        MediaError::Xml(_)
    ));
    assert!(matches!(
        parse("<html/>", &base()).unwrap_err(),
        MediaError::Invalid(_)
    ));
    let no_duration = mpd(
        r#"type="static""#,
        r#"<Period><AdaptationSet mimeType="video/mp4"><SegmentTemplate duration="5" media="$Number$.m4s"/>
        <Representation id="r" bandwidth="1"/></AdaptationSet></Period>"#,
    );
    assert!(
        matches!(parse(&no_duration, &base()).unwrap_err(), MediaError::Invalid(m) if m.contains("length"))
    );
    assert_eq!(
        parse(&mpd(r#"type="static""#, ""), &base()).unwrap_err(),
        MediaError::Empty
    );
}
