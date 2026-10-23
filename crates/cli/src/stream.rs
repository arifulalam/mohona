//! `mohona stream`: download an HLS (.m3u8) or DASH (.mpd) stream.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use mohona_core::{
    download_stream, record_live, sanitize_filename, CancelToken, Error as CoreError, LiveEnd,
    LiveEvent, LiveRequest, RateLimit, StreamEvent, StreamRequest,
};
use mohona_media::dash;
use mohona_media::hls::{self, Playlist};
use mohona_media::mux::{self, Container};
use mohona_media::quality::{
    pick_audio_rendition, pick_rendition, pick_subtitles, pick_track, pick_variant, Quality,
};
use mohona_media::{MediaKind, SegmentList};
use mohona_net::{ClientConfig, HttpClient};
use tokio::sync::mpsc;
use url::Url;

use crate::{describe, format_bytes, RequestArgs};

pub(crate) struct StreamArgs {
    pub url: String,
    pub output: PathBuf,
    pub name: Option<String>,
    pub quality: String,
    pub audio: Option<String>,
    pub list: bool,
    pub no_join: bool,
    pub record_for: Option<f64>,
    pub subs: Option<String>,
    pub embed_subs: bool,
    pub container: String,
    pub ffmpeg: Option<PathBuf>,
    pub connections: usize,
    pub max_speed: Option<u64>,
    pub request: RequestArgs,
}

/// One subtitle track to download and convert.
struct SubTrack {
    label: String,
    segments: SegmentList,
}

/// Whether `--subs` asks for a track with these names.
fn wanted(subs: &str, names: &[Option<&str>]) -> bool {
    subs.split(',').map(str::trim).any(|w| {
        w.eq_ignore_ascii_case("all")
            || names.iter().flatten().any(|n| {
                n.eq_ignore_ascii_case(w)
                    || n.to_ascii_lowercase()
                        .starts_with(&format!("{}-", w.to_ascii_lowercase()))
            })
    })
}

/// One file to make.
struct Job {
    label: String,
    segments: SegmentList,
    extension: &'static str,
    /// The playlist's address when the stream is live (it has no end yet).
    live: Option<Url>,
}

enum Kind {
    Hls,
    Dash,
}

pub(crate) async fn run(args: StreamArgs) -> Result<(), String> {
    let quality = Quality::parse(&args.quality)?;
    let container = Container::parse(&args.container)?;
    let (address, from_address) = mohona_net::split_userinfo(&args.url);
    let mut context = args.request.context()?;
    if context.credentials.is_none() {
        context.credentials = from_address;
    }
    let client = HttpClient::new(&ClientConfig::default())
        .and_then(|c| c.with_context(&context))
        .map_err(|e| describe(&e))?;
    let base = Url::parse(&address).map_err(|e| format!("\"{address}\" is not an address: {e}"))?;

    let text = fetch_text(&client, &address).await?;
    let kind = if text
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with("#EXTM3U")
    {
        Kind::Hls
    } else if text.trim_start().starts_with('<') {
        Kind::Dash
    } else {
        return Err(format!(
            "{address} is not a stream: it is neither an HLS playlist nor a DASH manifest"
        ));
    };

    let jobs = match kind {
        Kind::Hls => hls_jobs(&client, &base, &text, quality, &args).await?,
        Kind::Dash => dash_jobs(&base, &text, quality, &args)?,
    };
    let (jobs, subtitle_tracks) = jobs;
    if args.list {
        return Ok(());
    }

    let name = sanitize_filename(&args.name.clone().unwrap_or_else(|| name_from(&base)));
    std::fs::create_dir_all(&args.output)
        .map_err(|e| format!("cannot use the folder {}: {e}", args.output.display()))?;
    let cancel = CancelToken::new();
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                cancel.cancel();
            }
        });
    }
    let limits = match args.max_speed.filter(|r| *r > 0) {
        Some(rate) => vec![RateLimit::new(rate)],
        None => Vec::new(),
    };

    let several = jobs.len() > 1;
    let mut saved = Vec::new();
    let live = jobs.iter().any(|j| j.live.is_some());
    if live && (args.subs.is_some() || !subtitle_tracks.is_empty()) {
        return Err("subtitles are not supported for live streams yet (leave out --subs)".into());
    }
    // A finished file of the same name is never replaced.
    let name = free_name(&args.output, &name, &jobs);
    let jobs = if live {
        saved = record_jobs(&client, &args, &name, jobs, &cancel, &limits).await?;
        Vec::new()
    } else {
        jobs
    };
    for (index, job) in jobs.into_iter().enumerate() {
        let file = if index == 0 {
            format!("{name}.{}", job.extension)
        } else {
            format!("{name}.audio.{}", job.extension)
        };
        let dest = args.output.join(file);
        let mut request = StreamRequest::new(job.segments, &dest);
        request.connections = args.connections;
        request.cancel = cancel.clone();
        request.limits = limits.clone();
        println!("Downloading {} ...", job.label);
        let (sender, mut receiver) = mpsc::channel(64);
        let client = client.clone();
        let worker =
            tokio::spawn(async move { download_stream(&client, &request, Some(&sender)).await });
        let interactive = std::io::stdout().is_terminal();
        let mut last_tenth = 0;
        while let Some(event) = receiver.recv().await {
            if let StreamEvent::Progress { done, total, bytes } = event {
                if interactive {
                    print!("\r  {done}/{total} segments, {}   ", format_bytes(bytes));
                    let _ = std::io::stdout().flush();
                } else if done * 10 / total > last_tenth {
                    last_tenth = done * 10 / total;
                    println!("  {done}/{total} segments, {}", format_bytes(bytes));
                }
            }
        }
        if interactive {
            println!();
        }
        match worker.await.map_err(|e| e.to_string())? {
            Ok(outcome) => {
                println!("Saved {} ({})", outcome.path.display(), format_bytes(outcome.bytes));
                saved.push(outcome.path);
            }
            Err(CoreError::Cancelled) => {
                return Err("stopped. Run the same command again to resume.".into())
            }
            Err(error) => {
                return Err(format!(
                    "{}\nThe segments already fetched are kept. Run the same command again to continue.",
                    describe(&error)
                ))
            }
        }
    }
    let mut main = saved.first().cloned();
    if several && saved.len() == 2 {
        main = join_files(&args, &name, container, &saved[0], &saved[1]).await;
    }
    if !subtitle_tracks.is_empty() {
        let files =
            download_subtitles(&client, &args, &name, subtitle_tracks, &cancel, &limits).await?;
        if args.embed_subs && !files.is_empty() {
            match main {
                Some(main) => embed(&args, &name, container, &main, &files).await,
                None => println!(
                    "The subtitles stay as .srt files: they can only be put into one joined video file."
                ),
            }
        }
    }
    Ok(())
}

/// A name whose result files do not exist yet: a recording never replaces an
/// earlier one.
fn free_name(dir: &std::path::Path, name: &str, jobs: &[Job]) -> String {
    let taken = |candidate: &str| {
        jobs.iter().enumerate().any(|(i, j)| {
            let file = if i == 0 {
                format!("{candidate}.{}", j.extension)
            } else {
                format!("{candidate}.audio.{}", j.extension)
            };
            dir.join(file).exists()
        }) || dir.join(format!("{candidate}.mp4")).exists()
            || dir.join(format!("{candidate}.mkv")).exists()
    };
    if !taken(name) {
        return name.to_string();
    }
    (1..)
        .map(|n| format!("{name} ({n})"))
        .find(|c| !taken(c))
        .expect("an unused name exists")
}

/// `90`, `90s`, `30m`, `2h`, `1h30m`, `1h30m15s`: seconds.
pub(crate) fn parse_duration(text: &str) -> Result<f64, String> {
    let bad = || format!("\"{text}\" is not a time (use 90s, 30m, 2h or 1h30m)");
    let t = text.trim().to_ascii_lowercase();
    if let Ok(n) = t.parse::<f64>() {
        return if n > 0.0 { Ok(n) } else { Err(bad()) };
    }
    let (mut total, mut number) = (0.0, String::new());
    for c in t.chars() {
        if c.is_ascii_digit() || c == '.' {
            number.push(c);
        } else {
            let unit = match c {
                'h' => 3600.0,
                'm' => 60.0,
                's' => 1.0,
                _ => return Err(bad()),
            };
            total += number.parse::<f64>().map_err(|_| bad())? * unit;
            number.clear();
        }
    }
    if !number.is_empty() || total <= 0.0 {
        return Err(bad());
    }
    Ok(total)
}

fn clock(seconds: f64) -> String {
    let s = seconds as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Records live jobs (video, and audio when it is separate) side by side,
/// until the streams end, the limit is reached or Ctrl-C is pressed.
async fn record_jobs(
    client: &HttpClient,
    args: &StreamArgs,
    name: &str,
    jobs: Vec<Job>,
    cancel: &CancelToken,
    limits: &[std::sync::Arc<RateLimit>],
) -> Result<Vec<PathBuf>, String> {
    println!("Live stream: recording. Press Ctrl-C to stop; what is recorded is kept.");
    let interactive = std::io::stdout().is_terminal();
    let mut workers = Vec::new();
    let mut display = None;
    for (index, job) in jobs.into_iter().enumerate() {
        let Some(playlist) = job.live else {
            return Err(
                "this stream is live for one part and finished for another; that is not supported"
                    .into(),
            );
        };
        let file = if index == 0 {
            format!("{name}.{}", job.extension)
        } else {
            format!("{name}.audio.{}", job.extension)
        };
        let mut request = LiveRequest::new(playlist, args.output.join(file));
        request.cancel = cancel.clone();
        request.limits = limits.to_vec();
        request.max_seconds = args.record_for;
        let (sender, receiver) = mpsc::channel(64);
        if index == 0 {
            display = Some(receiver);
        } else {
            // Only the first track is shown; the others still need a reader.
            tokio::spawn(async move {
                let mut receiver = receiver;
                while receiver.recv().await.is_some() {}
            });
        }
        let client = client.clone();
        workers.push(tokio::spawn(async move {
            record_live(&client, &request, Some(&sender)).await
        }));
    }
    if let Some(mut receiver) = display {
        let mut last_report = 0usize;
        while let Some(event) = receiver.recv().await {
            match event {
                LiveEvent::Segment {
                    count,
                    seconds,
                    bytes,
                } => {
                    if interactive {
                        print!(
                            "\r  Recording {} of video, {count} segments, {}   ",
                            clock(seconds),
                            format_bytes(bytes)
                        );
                        let _ = std::io::stdout().flush();
                    } else if count >= last_report + 10 {
                        last_report = count;
                        println!(
                            "  Recorded {} ({count} segments, {})",
                            clock(seconds),
                            format_bytes(bytes)
                        );
                    }
                }
                LiveEvent::Missed { count } => {
                    if interactive {
                        println!();
                    }
                    println!(
                        "  Warning: {count} segment(s) were gone before they could be fetched."
                    );
                }
                LiveEvent::Started => {}
            }
        }
        if interactive {
            println!();
        }
    }
    let mut saved = Vec::new();
    for worker in workers {
        match worker.await.map_err(|e| e.to_string())? {
            Ok(outcome) => {
                let why = match outcome.ended {
                    LiveEnd::StreamEnded => "the stream ended",
                    LiveEnd::Stopped => "stopped",
                    LiveEnd::TimeLimit => "time limit reached",
                    LiveEnd::PlaylistGone => {
                        "the stream's playlist is gone (it has probably ended)"
                    }
                };
                println!(
                    "Saved {} ({}, {}; {why})",
                    outcome.path.display(),
                    clock(outcome.seconds),
                    format_bytes(outcome.bytes)
                );
                if outcome.segments > 0 {
                    saved.push(outcome.path);
                } else {
                    let _ = std::fs::remove_file(&outcome.path);
                }
            }
            Err(error) => return Err(describe(&error)),
        }
    }
    if saved.is_empty() {
        return Err("nothing was recorded".into());
    }
    Ok(saved)
}

/// Downloads each subtitle track and saves it as `NAME.LANG.srt`. A track
/// that cannot be converted is reported and skipped.
async fn download_subtitles(
    client: &HttpClient,
    args: &StreamArgs,
    name: &str,
    tracks: Vec<SubTrack>,
    cancel: &CancelToken,
    limits: &[std::sync::Arc<RateLimit>],
) -> Result<Vec<(PathBuf, Option<String>)>, String> {
    let mut files = Vec::new();
    for track in tracks {
        let language = sanitize_filename(&track.label);
        let raw = args.output.join(format!("{name}.{language}.subtitles"));
        let srt = args.output.join(format!("{name}.{language}.srt"));
        println!("Downloading subtitles {} ...", track.label);
        let mut request = StreamRequest::new(track.segments, &raw);
        request.connections = args.connections;
        request.cancel = cancel.clone();
        request.limits = limits.to_vec();
        match download_stream(client, &request, None).await {
            Ok(_) => {}
            Err(CoreError::Cancelled) => {
                return Err("stopped. Run the same command again to resume.".into())
            }
            Err(error) => {
                println!(
                    "Could not download the subtitles {}: {}",
                    track.label,
                    describe(&error)
                );
                continue;
            }
        }
        let text = std::fs::read(&raw).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&raw);
        match mohona_media::subtitles::to_srt(&String::from_utf8_lossy(&text)) {
            Ok(converted) => {
                std::fs::write(&srt, converted)
                    .map_err(|e| format!("cannot save {}: {e}", srt.display()))?;
                println!("Saved {}", srt.display());
                files.push((srt, Some(track.label.clone())));
            }
            Err(error) => println!("Could not use the subtitles {}: {error}", track.label),
        }
    }
    Ok(files)
}

/// Puts the subtitle files into the finished video as tracks (the `.srt`
/// files stay too). Trouble is reported but never fails the command.
async fn embed(
    args: &StreamArgs,
    name: &str,
    container: Container,
    main: &std::path::Path,
    files: &[(PathBuf, Option<String>)],
) {
    let Some(ffmpeg) = mux::find_ffmpeg(args.ffmpeg.as_deref()) else {
        println!("The subtitles stay as .srt files because ffmpeg was not found (see --ffmpeg).");
        return;
    };
    let output = args
        .output
        .join(format!("{name}.{}", container.extension()));
    println!("Adding the subtitles to the video with ffmpeg ...");
    let (m, f, o) = (main.to_path_buf(), files.to_vec(), output.clone());
    let result =
        tokio::task::spawn_blocking(move || mux::embed_subtitles(&ffmpeg, &m, &f, &o, container))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
    match result {
        Ok(()) => {
            if main != output {
                let _ = std::fs::remove_file(main);
            }
            println!("Saved {} (with subtitles)", output.display());
        }
        Err(message) => println!("{message}\nThe video is unchanged and the .srt files are kept."),
    }
}

/// Joins video and audio into one file. Trouble here is reported but never
/// fails the command: the two downloaded files are still good, and are kept.
async fn join_files(
    args: &StreamArgs,
    name: &str,
    container: Container,
    video: &std::path::Path,
    audio: &std::path::Path,
) -> Option<PathBuf> {
    if args.no_join {
        println!("Video and audio are kept as two files (--no-join).");
        return None;
    }
    let Some(ffmpeg) = mux::find_ffmpeg(args.ffmpeg.as_deref()) else {
        println!(
            "Video and audio are saved as two files because ffmpeg was not found.\n\
             Install ffmpeg and run the same command again (it will not download anything twice),\n\
             or say where it is with --ffmpeg PATH (or the MOHONA_FFMPEG variable)."
        );
        return None;
    };
    let output = args
        .output
        .join(format!("{name}.{}", container.extension()));
    println!("Joining video and audio with ffmpeg ...");
    let (v, a, o) = (video.to_path_buf(), audio.to_path_buf(), output.clone());
    let result = tokio::task::spawn_blocking(move || mux::join(&ffmpeg, &v, &a, &o, container))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
    match result {
        Ok(()) => {
            // The result may have taken the video's own name (both .mp4).
            for input in [video, audio] {
                if input != output {
                    let _ = std::fs::remove_file(input);
                }
            }
            let size = std::fs::metadata(&output).map_or(0, |m| m.len());
            println!("Saved {} ({})", output.display(), format_bytes(size));
            Some(output)
        }
        Err(message) => {
            println!(
                "{message}\nThe two files are kept: {} and {}.",
                video.display(),
                audio.display()
            );
            None
        }
    }
}

async fn fetch_text(client: &HttpClient, url: &str) -> Result<String, String> {
    let mut body = client.get(url).await.map_err(|e| describe(&e))?;
    let mut data = Vec::new();
    while let Some(chunk) = body.chunk().await.map_err(|e| describe(&e))? {
        data.extend_from_slice(&chunk);
        if data.len() > 16 * 1024 * 1024 {
            return Err(format!("{url} is too big to be a playlist"));
        }
    }
    Ok(String::from_utf8_lossy(&data).into_owned())
}

/// A file name from the address: the last part, or the one before it when
/// that is a generic playlist name.
fn name_from(url: &Url) -> String {
    let parts: Vec<&str> = url
        .path_segments()
        .map(|p| p.filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let generic = [
        "index",
        "master",
        "playlist",
        "manifest",
        "stream",
        "main",
        "chunklist",
    ];
    for part in parts.iter().rev() {
        let stem = part.rsplit_once('.').map_or(*part, |(s, _)| s);
        if !stem.is_empty() && !generic.contains(&stem.to_ascii_lowercase().as_str()) {
            return stem.to_string();
        }
    }
    "stream".to_string()
}

fn describe_variant(v: &mohona_media::MediaVariant) -> String {
    let mut text = v.label();
    if let Some(codecs) = &v.codecs {
        text.push_str(&format!("  [{codecs}]"));
    }
    if v.is_audio_only() {
        text.push_str("  (sound only)");
    }
    text
}

async fn hls_jobs(
    client: &HttpClient,
    base: &Url,
    text: &str,
    quality: Quality,
    args: &StreamArgs,
) -> Result<(Vec<Job>, Vec<SubTrack>), String> {
    match hls::parse(text, base).map_err(|e| e.to_string())? {
        Playlist::Media(list) => {
            if args.list {
                println!(
                    "HLS stream (a single quality), {} segments, {:.0} s",
                    list.segments.len(),
                    list.total_duration()
                );
            }
            Ok((vec![job("the stream", list, "ts", Some(base))?], Vec::new()))
        }
        Playlist::Master { variants, tracks } => {
            if args.list {
                println!("HLS stream with {} qualities:", variants.len());
                let mut order: Vec<usize> = (0..variants.len()).collect();
                order.sort_by_key(|i| std::cmp::Reverse(variants[*i].bandwidth));
                for i in order {
                    println!("  {}", describe_variant(&variants[i]));
                }
                for (title, kind) in [
                    ("Audio", MediaKind::Audio),
                    ("Subtitles", MediaKind::Subtitles),
                ] {
                    let found: Vec<String> = tracks
                        .iter()
                        .filter(|t| t.kind == kind)
                        .map(|t| {
                            let language = t.language.clone().unwrap_or_else(|| t.name.clone());
                            if t.default {
                                format!("{language} (default)")
                            } else {
                                language
                            }
                        })
                        .collect();
                    if !found.is_empty() {
                        println!("{title}: {}", found.join(", "));
                    }
                }
                return Ok((Vec::new(), Vec::new()));
            }
            let index =
                pick_variant(&variants, quality).ok_or("the stream has no quality that fits")?;
            let variant = &variants[index];
            println!("Quality: {}", describe_variant(variant));
            let mut jobs = vec![media_job(client, &variant.url, "video", true).await?];
            if let (Some(group), false) = (&variant.audio_group, variant.is_audio_only()) {
                match pick_track(&tracks, MediaKind::Audio, group, args.audio.as_deref()) {
                    Some(track) => {
                        println!(
                            "Audio: {}",
                            track.language.clone().unwrap_or_else(|| track.name.clone())
                        );
                        let url = track.url.clone().expect("a picked track has a playlist");
                        jobs.push(media_job(client, &url, "audio", false).await?);
                    }
                    None if args.audio.is_some() => {
                        let known: Vec<String> = tracks
                            .iter()
                            .filter(|t| t.kind == MediaKind::Audio && &t.group == group)
                            .filter_map(|t| t.language.clone().or_else(|| Some(t.name.clone())))
                            .collect();
                        return Err(format!(
                            "no audio in \"{}\" (this stream has: {})",
                            args.audio.clone().unwrap_or_default(),
                            known.join(", ")
                        ));
                    }
                    None => {}
                }
            }
            let mut subs = Vec::new();
            if let Some(asked) = &args.subs {
                let group = variant.subtitle_group.as_deref();
                let offered: Vec<&mohona_media::Track> = tracks
                    .iter()
                    .filter(|t| {
                        t.kind == MediaKind::Subtitles
                            && Some(t.group.as_str()) == group
                            && t.url.is_some()
                    })
                    .collect();
                if offered.is_empty() {
                    return Err("this quality has no subtitles".into());
                }
                let mut found = false;
                for t in &offered {
                    if !wanted(asked, &[t.language.as_deref(), Some(&t.name)]) {
                        continue;
                    }
                    found = true;
                    let label = t.language.clone().unwrap_or_else(|| t.name.clone());
                    let url = t.url.clone().expect("filtered above");
                    let text = fetch_text(client, url.as_str()).await?;
                    match hls::parse(&text, &url).map_err(|e| e.to_string())? {
                        Playlist::Media(segments) => subs.push(SubTrack { label, segments }),
                        Playlist::Master { .. } => {
                            return Err(format!("{url} is another master playlist"))
                        }
                    }
                }
                if !found {
                    let known: Vec<String> = offered
                        .iter()
                        .map(|t| t.language.clone().unwrap_or_else(|| t.name.clone()))
                        .collect();
                    return Err(format!(
                        "no subtitles in \"{asked}\" (this stream has: {})",
                        known.join(", ")
                    ));
                }
            }
            Ok((jobs, subs))
        }
    }
}

async fn media_job(
    client: &HttpClient,
    url: &Url,
    label: &str,
    video: bool,
) -> Result<Job, String> {
    let text = fetch_text(client, url.as_str()).await?;
    match hls::parse(&text, url).map_err(|e| e.to_string())? {
        Playlist::Media(list) => {
            let fragmented = list.segments.iter().any(|s| s.init.is_some());
            let extension = match (fragmented, video) {
                (true, true) => "mp4",
                (true, false) => "m4a",
                _ => "ts",
            };
            job(label, list, extension, Some(url))
        }
        Playlist::Master { .. } => Err(format!(
            "{url} is another master playlist, not a list of segments"
        )),
    }
}

fn job(
    label: &str,
    list: SegmentList,
    extension: &'static str,
    address: Option<&Url>,
) -> Result<Job, String> {
    let live = if list.ended { None } else { address.cloned() };
    Ok(Job {
        label: label.to_string(),
        segments: list,
        extension,
        live,
    })
}

fn dash_jobs(
    base: &Url,
    text: &str,
    quality: Quality,
    args: &StreamArgs,
) -> Result<(Vec<Job>, Vec<SubTrack>), String> {
    let renditions = dash::parse(text, base).map_err(|e| e.to_string())?;
    if args.list {
        println!("DASH stream:");
        for (title, kind) in [
            ("Video", MediaKind::Video),
            ("Audio", MediaKind::Audio),
            ("Subtitles", MediaKind::Subtitles),
        ] {
            for r in renditions.iter().filter(|r| r.kind == kind) {
                let size = r
                    .resolution
                    .map_or(String::new(), |s| format!("{}p ", s.height));
                let language = r
                    .language
                    .as_deref()
                    .map_or(String::new(), |l| format!(" [{l}]"));
                println!(
                    "  {title}: {size}{:.1} Mbit/s{language}  ({} segments)",
                    r.bandwidth as f64 / 1_000_000.0,
                    r.segments.segments.len()
                );
            }
        }
        return Ok((Vec::new(), Vec::new()));
    }
    let index =
        pick_rendition(&renditions, quality).ok_or("the stream has no quality that fits")?;
    let main = &renditions[index];
    println!("Quality: {:.1} Mbit/s", main.bandwidth as f64 / 1_000_000.0);
    let mut jobs = vec![job("video", main.segments.clone(), "mp4", None)?];
    if main.kind == MediaKind::Video {
        match pick_audio_rendition(&renditions, args.audio.as_deref()) {
            Some(i) => {
                println!(
                    "Audio: {}",
                    renditions[i]
                        .language
                        .clone()
                        .unwrap_or_else(|| "default".into())
                );
                jobs.push(job("audio", renditions[i].segments.clone(), "m4a", None)?);
            }
            None if args.audio.is_some() => {
                return Err(format!(
                    "no audio in \"{}\" in this stream",
                    args.audio.clone().unwrap_or_default()
                ))
            }
            None => {}
        }
    }
    let mut subs = Vec::new();
    if let Some(asked) = &args.subs {
        let offered = pick_subtitles(&renditions, None);
        if offered.is_empty() {
            return Err("this stream has no subtitles".into());
        }
        for r in &offered {
            if wanted(asked, &[r.language.as_deref(), Some(&r.id)]) {
                subs.push(SubTrack {
                    label: r.language.clone().unwrap_or_else(|| r.id.clone()),
                    segments: r.segments.clone(),
                });
            }
        }
        if subs.is_empty() {
            let known: Vec<String> = offered
                .iter()
                .map(|r| r.language.clone().unwrap_or_else(|| r.id.clone()))
                .collect();
            return Err(format!(
                "no subtitles in \"{asked}\" (this stream has: {})",
                known.join(", ")
            ));
        }
    }
    Ok((jobs, subs))
}
