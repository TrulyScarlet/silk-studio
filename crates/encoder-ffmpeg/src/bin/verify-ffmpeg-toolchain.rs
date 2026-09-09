use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use encoder_ffmpeg::FfmpegToolchain;

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn run(arguments: Vec<OsString>) -> Result<(), String> {
    let Some((ffmpeg_path, ffprobe_path)) = parse_arguments(arguments)? else {
        println!("{}", usage());
        return Ok(());
    };
    let toolchain = FfmpegToolchain::new(ffmpeg_path, ffprobe_path);
    let verified = toolchain
        .verify()
        .map_err(|error| format!("[{}] {error}", error.code()))?;

    println!("ffmpeg_path={}", verified.ffmpeg_path().display());
    println!("ffprobe_path={}", verified.ffprobe_path().display());
    println!("ffmpeg_version={}", verified.ffmpeg_version());
    println!("ffprobe_version={}", verified.ffprobe_version());
    println!("ffmpeg_sha256={}", verified.ffmpeg_sha256());
    println!("ffprobe_sha256={}", verified.ffprobe_sha256());
    println!("ffmpeg_build_configuration=");
    print!("{}", verified.build_configuration());
    println!("ffprobe_build_configuration=");
    print!("{}", verified.ffprobe_build_configuration());
    println!("verification=passed");
    Ok(())
}

fn parse_arguments(arguments: Vec<OsString>) -> Result<Option<(PathBuf, PathBuf)>, String> {
    let mut ffmpeg_path = None;
    let mut ffprobe_path = None;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.to_string_lossy().as_ref() {
            "--ffmpeg" => ffmpeg_path = Some(next_path(&mut arguments, "--ffmpeg")?),
            "--ffprobe" => ffprobe_path = Some(next_path(&mut arguments, "--ffprobe")?),
            "--help" | "-h" => return Ok(None),
            unknown => return Err(format!("unknown argument {unknown}\n\n{}", usage())),
        }
    }

    let ffmpeg_path = ffmpeg_path.ok_or_else(usage)?;
    let ffprobe_path = ffprobe_path.ok_or_else(usage)?;
    Ok(Some((ffmpeg_path, ffprobe_path)))
}

fn next_path(
    arguments: &mut impl Iterator<Item = OsString>,
    option: &str,
) -> Result<PathBuf, String> {
    arguments
        .next()
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| format!("{option} requires a path\n\n{}", usage()))
}

fn usage() -> String {
    "usage: verify-ffmpeg-toolchain --ffmpeg <path> --ffprobe <path>".to_string()
}
