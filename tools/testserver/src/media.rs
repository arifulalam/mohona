//! Playlists and manifests for tests. Segments are `/file?size=N` pieces, so
//! every piece has its own length and the joined result shows the order.

fn segment_lines(sizes: &[u64]) -> String {
    let mut text = String::from("#EXTM3U\n#EXT-X-TARGETDURATION:4\n");
    for size in sizes {
        text.push_str(&format!("#EXTINF:4,\n/file?size={size}\n"));
    }
    text.push_str("#EXT-X-ENDLIST\n");
    text
}

/// The text served at `/hls/<name>` and `/dash/<name>`, if there is any.
pub fn body(path: &str) -> Option<String> {
    Some(match path {
        "/hls/master.m3u8" => "#EXTM3U\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,URI=\"/hls/audio-en.m3u8\"\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Bangla\",LANGUAGE=\"bn\",URI=\"/hls/audio-bn.m3u8\"\n\
            #EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,URI=\"/hls/subs-en.m3u8\"\n\
            #EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"Bangla\",LANGUAGE=\"bn\",URI=\"/hls/subs-bn.m3u8\"\n\
            #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS=\"avc1.42c01e,mp4a.40.2\",AUDIO=\"aud\",SUBTITLES=\"subs\"\n\
            /hls/v360.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1280x720,CODECS=\"avc1.4d401f,mp4a.40.2\",AUDIO=\"aud\",SUBTITLES=\"subs\"\n\
            /hls/v720.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=64000,CODECS=\"mp4a.40.5\"\n\
            /hls/sound.m3u8\n"
            .to_string(),
        "/hls/v720.m3u8" => segment_lines(&[1000, 1100, 1200]),
        "/hls/v360.m3u8" => segment_lines(&[500, 600, 700]),
        "/hls/audio-en.m3u8" => segment_lines(&[300, 310]),
        "/hls/audio-bn.m3u8" => segment_lines(&[400, 410]),
        "/hls/sound.m3u8" | "/hls/plain.m3u8" => segment_lines(&[100, 200]),
        "/hls/subs-en.m3u8" => "#EXTM3U\n#EXT-X-TARGETDURATION:10\n\
            #EXTINF:10,\n/hls/sub-en-1.vtt\n#EXTINF:10,\n/hls/sub-en-2.vtt\n#EXT-X-ENDLIST\n"
            .to_string(),
        "/hls/subs-bn.m3u8" => "#EXTM3U\n#EXT-X-TARGETDURATION:10\n\
            #EXTINF:10,\n/hls/sub-bn-1.vtt\n#EXT-X-ENDLIST\n"
            .to_string(),
        "/hls/sub-en-1.vtt" => "WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:900000,LOCAL:00:00:00.000\n\n\
            00:00:01.000 --> 00:00:03.000\nHello <i>world</i>\n\n\
            00:00:08.000 --> 00:00:12.000\nAcross the cut\n"
            .to_string(),
        "/hls/sub-en-2.vtt" => "WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:1800000,LOCAL:00:00:00.000\n\n\
            00:00:08.000 --> 00:00:12.000\nAcross the cut\n\n\
            00:00:15.000 --> 00:00:16.500\nThe end &amp; more\n"
            .to_string(),
        "/hls/sub-bn-1.vtt" => "WEBVTT\n\n00:00:02.000 --> 00:00:04.000\nহ্যালো\n".to_string(),
        "/dash/sub-bn.vtt" => "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nদাশ\n".to_string(),
        "/dash/sub-fr.ttml" => r#"<?xml version="1.0"?>
<tt xmlns="http://www.w3.org/ns/ttml"><body><div>
 <p begin="00:00:03.000" end="00:00:04.000">Bonjour<br/>le monde</p>
</div></body></tt>
"#
        .to_string(),
        "/hls/protected.m3u8" => "#EXTM3U\n#EXT-X-TARGETDURATION:4\n\
            #EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n\
            #EXTINF:4,\n/file?size=100\n#EXT-X-ENDLIST\n"
            .to_string(),
        "/dash/manifest.mpd" => r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT15S">
 <Period>
  <AdaptationSet contentType="video" mimeType="video/mp4">
   <SegmentTemplate timescale="1" duration="5" startNumber="1" media="/file?size=$Number$00"/>
   <Representation id="v1" bandwidth="2500000" width="1280" height="720" codecs="avc1.4d401f"/>
   <Representation id="v0" bandwidth="800000" width="640" height="360" codecs="avc1.42c01e">
     <SegmentTemplate timescale="1" duration="5" startNumber="1" media="/file?size=$Number$0"/>
   </Representation>
  </AdaptationSet>
  <AdaptationSet contentType="text" lang="bn" mimeType="text/vtt">
   <Representation id="s1" bandwidth="1000"><BaseURL>/dash/sub-bn.vtt</BaseURL></Representation>
  </AdaptationSet>
  <AdaptationSet contentType="text" lang="fr" mimeType="application/ttml+xml">
   <Representation id="s2" bandwidth="1000"><BaseURL>/dash/sub-fr.ttml</BaseURL></Representation>
  </AdaptationSet>
  <AdaptationSet contentType="audio" lang="en" mimeType="audio/mp4">
   <SegmentTemplate timescale="1" duration="5" startNumber="1" media="/file?size=$Number$5"/>
   <Representation id="a1" bandwidth="128000" codecs="mp4a.40.2"/>
  </AdaptationSet>
 </Period>
</MPD>
"#
        .to_string(),
        _ => return None,
    })
}
