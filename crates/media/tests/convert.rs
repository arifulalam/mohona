use std::path::Path;
use std::process::Command;

use mohona_media::convert::{run, Preset, PRESETS};
use mohona_media::mux::find_ffmpeg;

fn ffmpeg(args: &[&str], ff: &Path) {
    let out = Command::new(ff)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn info(ff: &Path, file: &Path) -> String {
    let out = Command::new(ff)
        .args(["-hide_banner", "-i"])
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn presets_have_names_and_parse_back() {
    for p in PRESETS {
        assert_eq!(Preset::parse(p.name()).unwrap(), p);
        assert!(!p.describe().is_empty());
    }
    assert_eq!(Preset::parse("SMALL").unwrap(), Preset::Mp4Small);
    assert!(Preset::parse("gif").unwrap_err().contains("not a preset"));
}

#[test]
fn converts_with_every_preset() {
    let Some(ff) = find_ffmpeg(None) else {
        eprintln!("skipped: ffmpeg is not installed");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mkv");
    // 1280x1000 video (taller than 720) with a one-second tone.
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=1280x1000:rate=5:duration=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "-shortest",
            source.to_str().unwrap(),
        ],
        &ff,
    );
    for preset in PRESETS {
        let out = dir
            .path()
            .join(format!("out-{}.{}", preset.name(), preset.extension()));
        run(&ff, &source, &out, preset).unwrap();
        let text = info(&ff, &out);
        match preset {
            Preset::Mp3 => {
                assert!(text.contains("Audio: mp3"), "{text}");
                assert!(!text.contains("Video:"), "{text}");
            }
            Preset::M4a => {
                assert!(text.contains("Audio: aac"), "{text}");
                assert!(!text.contains("Video:"), "{text}");
            }
            Preset::Mp4 => {
                assert!(
                    text.contains("Video: h264") && text.contains("Audio: aac"),
                    "{text}"
                );
                assert!(text.contains("1280x1000"), "{text}");
            }
            Preset::Mp4Small => {
                assert!(
                    text.contains("Video: h264") && text.contains("Audio: aac"),
                    "{text}"
                );
                assert!(text.contains("x720"), "{text}");
            }
        }
    }
    // No temporary files are left.
    let leftovers = std::fs::read_dir(dir.path())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("converting")
        })
        .count();
    assert_eq!(leftovers, 0);
}

#[test]
fn a_file_without_sound_gives_a_clear_failure_for_audio_presets() {
    let Some(ff) = find_ffmpeg(None) else {
        eprintln!("skipped: ffmpeg is not installed");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let silent = dir.path().join("silent.mp4");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=64x48:rate=5:duration=1",
            "-c:v",
            "mpeg4",
            silent.to_str().unwrap(),
        ],
        &ff,
    );
    let out = dir.path().join("x.mp3");
    let error = run(&ff, &silent, &out, Preset::Mp3).unwrap_err();
    assert!(error.contains("ffmpeg could not convert"), "{error}");
    assert!(!out.exists());
    // A video preset still works on a silent video.
    run(&ff, &silent, &dir.path().join("y.mp4"), Preset::Mp4).unwrap();
}
