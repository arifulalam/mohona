//! `mohona video-format`: the kind of file taken from video pages (YouTube and alike).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use mohona_daemon::{Call, Client, Response};

#[derive(Args)]
pub struct VideoFormatArgs {
    /// Quality: best, 2160, 1440, 1080, 720, 480, 360 (the tallest picture to take) or audio (sound only)
    #[arg(long)]
    quality: Option<String>,
    /// Video format: mp4, mkv, webm, mov or avi (mov and avi are converted by ffmpeg, which takes longer)
    #[arg(long)]
    format: Option<String>,
    /// Audio format for sound only: mp3, m4a, opus, ogg, flac, wav or aac
    #[arg(long)]
    audio: Option<String>,
    /// The daemon's connection file (default: your application-data folder)
    #[arg(long)]
    info: Option<PathBuf>,
}

async fn run_inner(args: VideoFormatArgs) -> Result<(), String> {
    let path = args.info.unwrap_or_else(mohona_daemon::default_info_path);
    let client = Client::from_info_file(&path)
        .map_err(|_| "the daemon is not running. Start it with: mohona daemon".to_string())?;
    let ask = |call: Call| async {
        match client.call(call).await.map_err(|e| e.to_string())? {
            Response::VideoFormat {
                quality,
                container,
                audio,
            } => Ok((quality, container, audio)),
            Response::Error { message } => Err(message),
            _ => Err("the daemon sent an unexpected answer".to_string()),
        }
    };
    let (mut quality, mut container, mut audio) = ask(Call::GetVideoFormat).await?;
    if args.quality.is_some() || args.format.is_some() || args.audio.is_some() {
        // What is not mentioned stays as it is.
        quality = args.quality.unwrap_or(quality);
        container = args.format.unwrap_or(container);
        audio = args.audio.unwrap_or(audio);
        (quality, container, audio) = ask(Call::SetVideoFormat {
            quality,
            container,
            audio,
        })
        .await?;
    }
    if quality == "audio" {
        println!("Video pages give the sound only, as {audio}.");
    } else if quality == "best" {
        println!("Video pages give the best quality, as {container}.");
    } else {
        println!("Video pages give at most {quality}p, as {container}.");
    }
    Ok(())
}

pub async fn run(args: VideoFormatArgs) -> ExitCode {
    match run_inner(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
