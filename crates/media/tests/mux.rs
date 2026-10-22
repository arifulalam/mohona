use std::path::Path;
use std::process::Command;

use mohona_media::mux::{find_ffmpeg, join, Container};

/// Makes a one-second test video and a one-second tone with ffmpeg itself.
fn make_inputs(ffmpeg: &Path, dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let video = dir.join("v.mp4");
    let audio = dir.join("a.m4a");
    let run = |args: &[&str]| {
        let out = Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=64x48:rate=10:duration=1",
        "-c:v",
        "mpeg4",
        video.to_str().unwrap(),
    ]);
    run(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=1",
        "-c:a",
        "aac",
        audio.to_str().unwrap(),
    ]);
    (video, audio)
}

fn streams(ffmpeg: &Path, file: &Path) -> String {
    // ffmpeg -i prints the streams to stderr and exits with an error (no output given).
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn joins_video_and_audio_by_copying() {
    let Some(ffmpeg) = find_ffmpeg(None) else {
        eprintln!("skipped: ffmpeg is not installed");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (video, audio) = make_inputs(&ffmpeg, dir.path());
    for container in [Container::Mp4, Container::Mkv] {
        let output = dir.path().join(format!("joined.{}", container.extension()));
        join(&ffmpeg, &video, &audio, &output, container).unwrap();
        let info = streams(&ffmpeg, &output);
        assert!(info.contains("Video:") && info.contains("Audio:"), "{info}");
        // Copied, not re-encoded.
        assert!(info.contains("mpeg4"), "{info}");
    }
    // Only the inputs and the results are left (no temporary files).
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 4);
}

#[test]
fn the_result_may_replace_one_of_the_inputs() {
    let Some(ffmpeg) = find_ffmpeg(None) else {
        eprintln!("skipped: ffmpeg is not installed");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (video, audio) = make_inputs(&ffmpeg, dir.path());
    join(&ffmpeg, &video, &audio, &video, Container::Mp4).unwrap();
    assert!(streams(&ffmpeg, &video).contains("Audio:"));
}

#[test]
fn bad_input_gives_ffmpegs_words_and_leaves_nothing() {
    let Some(ffmpeg) = find_ffmpeg(None) else {
        eprintln!("skipped: ffmpeg is not installed");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (video, audio) = (dir.path().join("v.ts"), dir.path().join("a.ts"));
    std::fs::write(&video, b"not media").unwrap();
    std::fs::write(&audio, b"not media").unwrap();
    let output = dir.path().join("out.mp4");
    let error = join(&ffmpeg, &video, &audio, &output, Container::Mp4).unwrap_err();
    assert!(error.contains("ffmpeg could not join"), "{error}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn an_explicit_path_is_not_second_guessed() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(find_ffmpeg(Some(&dir.path().join("nope"))), None);
    assert_eq!(find_ffmpeg(Some(dir.path())), None); // a folder without ffmpeg in it
    if let Some(real) = find_ffmpeg(None) {
        assert_eq!(find_ffmpeg(Some(&real)), Some(real.clone()));
        assert_eq!(find_ffmpeg(Some(real.parent().unwrap())), Some(real));
    }
}

#[test]
fn language_tags() {
    use mohona_media::mux::language_tag;
    assert_eq!(language_tag("en").as_deref(), Some("eng"));
    assert_eq!(language_tag("bn-BD").as_deref(), Some("ben"));
    assert_eq!(language_tag("fil").as_deref(), Some("fil"));
    assert_eq!(language_tag("English"), None);
    assert_eq!(language_tag("x"), None);
}

#[test]
fn embeds_srt_files_as_tracks() {
    use mohona_media::mux::embed_subtitles;
    let Some(ffmpeg) = find_ffmpeg(None) else {
        eprintln!("skipped: ffmpeg is not installed");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (video, audio) = make_inputs(&ffmpeg, dir.path());
    let joined = dir.path().join("j.mp4");
    join(&ffmpeg, &video, &audio, &joined, Container::Mp4).unwrap();
    let en = dir.path().join("en.srt");
    let bn = dir.path().join("bn.srt");
    std::fs::write(&en, "1\n00:00:00,100 --> 00:00:00,900\nHello\n\n").unwrap();
    std::fs::write(&bn, "1\n00:00:00,100 --> 00:00:00,900\nHalo\n\n").unwrap();
    let subs = vec![(en, Some("en".to_string())), (bn, Some("bn".to_string()))];
    for container in [Container::Mp4, Container::Mkv] {
        let out = dir
            .path()
            .join(format!("with-subs.{}", container.extension()));
        embed_subtitles(&ffmpeg, &joined, &subs, &out, container).unwrap();
        let info = streams(&ffmpeg, &out);
        assert_eq!(info.matches("Subtitle:").count(), 2, "{info}");
        assert!(info.contains("(eng)") && info.contains("(ben)"), "{info}");
        assert!(info.contains("Video:") && info.contains("Audio:"), "{info}");
    }
    // Replacing the input itself works, and a bad subtitle file changes nothing.
    embed_subtitles(&ffmpeg, &joined, &subs, &joined, Container::Mp4).unwrap();
    assert_eq!(streams(&ffmpeg, &joined).matches("Subtitle:").count(), 2);
    let before = std::fs::read(&joined).unwrap();
    let bad = vec![(dir.path().join("missing.srt"), None)];
    let error = embed_subtitles(&ffmpeg, &joined, &bad, &joined, Container::Mp4).unwrap_err();
    assert!(error.contains("could not add the subtitles"), "{error}");
    assert_eq!(std::fs::read(&joined).unwrap(), before);
}
