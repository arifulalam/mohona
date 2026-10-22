use mohona_media::subtitles::{detect, to_srt, Format};
use mohona_media::MediaError;

#[test]
fn webvtt_becomes_srt() {
    let vtt = "\u{feff}WEBVTT\r\nX-TIMESTAMP-MAP=MPEGTS:900000,LOCAL:00:00:00.000\r\n\r\n\
        NOTE a comment\r\n\r\n\
        intro\r\n00:00:01.500 --> 00:00:03.000 align:start position:10%\r\n<v Ana>Hello &amp; <i>welcome</i></v>\r\n<c.yellow>second line</c>\r\n\r\n\
        01:02.000 --> 01:04.250\r\nShort time form\r\n\r\n\
        01:00:00.000 --> 01:00:01.000\r\n\r\n\
        00:00:05.000 --> 00:00:06.000\r\nWith <00:00:05.500>timestamp\r\n";
    let srt = to_srt(vtt).unwrap();
    assert_eq!(
        srt,
        "1\n00:00:01,500 --> 00:00:03,000\nHello & <i>welcome</i>\nsecond line\n\n\
         2\n00:01:02,000 --> 00:01:04,250\nShort time form\n\n\
         3\n00:00:05,000 --> 00:00:06,000\nWith timestamp\n\n"
    );
}

#[test]
fn glued_segments_and_repeated_cues_are_handled() {
    let one = "WEBVTT\n\n00:00:08.000 --> 00:00:12.000\nAcross the boundary\n\n00:00:01.000 --> 00:00:02.000\nFirst\n";
    let two = "WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:1,LOCAL:00:00:00.000\n\n00:00:08.000 --> 00:00:12.000\nAcross the boundary\n\n00:00:13.000 --> 00:00:14.000\nLast\n";
    // The first segment does not end with a blank line, as in real streams.
    let srt = to_srt(&format!("{one}{two}")).unwrap();
    assert_eq!(srt.matches("Across the boundary").count(), 1);
    assert!(srt.contains("Last") && srt.contains("First"));
    assert!(srt.starts_with("1\n00:00:08,000"));
}

#[test]
fn ttml_becomes_srt() {
    let ttml = r#"<?xml version="1.0"?>
<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter" ttp:tickRate="1000">
 <body><div>
  <p begin="00:00:01.000" end="00:00:02.500">Hello<br/>there  <span>friend</span></p>
  <p begin="3000t" dur="1.5s">By ticks</p>
  <p begin="10s" end="11s">   </p>
  <p begin="bad" end="11s">skipped</p>
 </div></body></tt>"#;
    let srt = to_srt(ttml).unwrap();
    assert_eq!(
        srt,
        "1\n00:00:01,000 --> 00:00:02,500\nHello\nthere friend\n\n\
         2\n00:00:03,000 --> 00:00:04,500\nBy ticks\n\n"
    );
}

#[test]
fn detection_and_errors() {
    assert_eq!(detect("WEBVTT\n"), Some(Format::WebVtt));
    assert_eq!(
        detect("<?xml version=\"1.0\"?><tt xmlns=\"x\"/>"),
        Some(Format::Ttml)
    );
    assert_eq!(detect("\0\0\0 ftypmp4"), None);
    assert!(matches!(to_srt("\0\0\0 moof"), Err(MediaError::Invalid(m)) if m.contains("MP4")));
    assert_eq!(to_srt("WEBVTT\n\n").unwrap_err(), MediaError::Empty);
}
