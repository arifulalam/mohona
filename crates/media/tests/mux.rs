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
