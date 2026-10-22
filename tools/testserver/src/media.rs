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
            #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS=\"avc1.42c01e,mp4a.40.2\",AUDIO=\"aud\"\n\
            /hls/v360.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1280x720,CODECS=\"avc1.4d401f,mp4a.40.2\",AUDIO=\"aud\"\n\
            /hls/v720.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=64000,CODECS=\"mp4a.40.5\"\n\
            /hls/sound.m3u8\n"
            .to_string(),
        "/hls/v720.m3u8" => segment_lines(&[1000, 1100, 1200]),
        "/hls/v360.m3u8" => segment_lines(&[500, 600, 700]),
        "/hls/audio-en.m3u8" => segment_lines(&[300, 310]),
        "/hls/audio-bn.m3u8" => segment_lines(&[400, 410]),
        "/hls/sound.m3u8" | "/hls/plain.m3u8" => segment_lines(&[100, 200]),
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
