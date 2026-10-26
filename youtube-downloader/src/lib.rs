use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn canonical_video_url(input: &str) -> Result<String, String> {
    let input = input.trim();
    let input = input
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| input.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(input);
    if input.is_empty() || input.chars().any(char::is_whitespace) {
        return Err("请输入一个完整的 YouTube 视频链接".into());
    }
    let input = input
        .strip_prefix("https://")
        .or_else(|| input.strip_prefix("http://"))
        .unwrap_or(input);
    let (host, path) = input.split_once('/').ok_or("链接缺少视频路径")?;
    let host = host.to_ascii_lowercase();
    let path = path.split('#').next().unwrap_or("");
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let id = match host.as_str() {
        "youtu.be" | "www.youtu.be" => Some(path),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com" => {
            if path == "watch" {
                let mut values = query.split('&').filter_map(|pair| pair.strip_prefix("v="));
                let id = values.next();
                if values.next().is_some() {
                    return Err("链接包含多个视频 ID，请使用单个视频的分享链接".into());
                }
                id
            } else {
                path.strip_prefix("shorts/")
                    .or_else(|| path.strip_prefix("live/"))
                    .or_else(|| path.strip_prefix("embed/"))
                    .map(|id| id.strip_suffix('/').unwrap_or(id))
            }
        }
        _ => return Err("仅接受 youtube.com 或 youtu.be 的视频链接".into()),
    };
    let id = id.filter(|id| {
        id.len() == 11
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    });
    match id {
        Some(id) => Ok(format!("https://www.youtube.com/watch?v={id}")),
        None => Err("未找到有效的视频 ID；请使用视频的分享链接，播放列表和频道链接不受支持".into()),
    }
}

pub fn find_tool(base: &Path, name: &str) -> Option<PathBuf> {
    let filename = format!("{name}{}", env::consts::EXE_SUFFIX);
    let bundled = [base.join("tools").join(&filename), base.join(&filename)];
    let search_path = env::var_os("PATH").unwrap_or_default();
    bundled
        .into_iter()
        .chain(env::split_paths(&search_path).map(|dir| dir.join(&filename)))
        .find(|path| path.is_file())
}

pub fn download_command(
    downloader: &Path,
    ffmpeg: &Path,
    runtime: OsString,
    output: &Path,
    format: &str,
    container: &str,
) -> Command {
    let mut command = media_command(downloader, ffmpeg, runtime, output, format);
    command.args([
        "--merge-output-format",
        container,
        "--remux-video",
        container,
    ]);
    command
}

pub fn media_command(
    downloader: &Path,
    ffmpeg: &Path,
    runtime: OsString,
    output: &Path,
    format: &str,
) -> Command {
    let mut command = Command::new(downloader);
    command.args([
        "--ignore-config",
        "--no-plugin-dirs",
        "--no-cache-dir",
        "--encoding",
        "utf-8",
        "--no-playlist",
        "--no-overwrites",
        "--continue",
        "--windows-filenames",
        "--format",
        format,
        "--retries",
        "3",
        "--fragment-retries",
        "3",
        "--socket-timeout",
        "20",
        "--progress",
        "--newline",
        "--progress-delta",
        "1",
        "--no-colors",
        "--no-js-runtimes",
    ]);
    command
        .arg("--js-runtimes")
        .arg(runtime)
        .arg("--ffmpeg-location")
        .arg(ffmpeg)
        .arg("--paths")
        .arg(output)
        .args(["--output", "%(title).120B [%(id)s].%(ext)s"]);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_common_video_links_and_removes_tracking() {
        for input in [
            "https://www.youtube.com/watch?v=BaW_jenozKc",
            "http://youtube.com/watch?list=PLtest&v=BaW_jenozKc&t=12#details",
            "https://youtu.be/BaW_jenozKc?si=tracking",
            "https://m.youtube.com/watch?v=BaW_jenozKc",
            "https://music.youtube.com/watch?v=BaW_jenozKc&list=test",
            "https://www.youtube.com/shorts/BaW_jenozKc/",
            "https://www.youtube.com/live/BaW_jenozKc?feature=share",
            "https://www.youtube.com/embed/BaW_jenozKc",
            "youtu.be/BaW_jenozKc",
            "  \"https://YOUTUBE.COM/watch?v=BaW_jenozKc\"  ",
            "'https://youtu.be/BaW_jenozKc'",
        ] {
            assert_eq!(
                canonical_video_url(input).unwrap(),
                "https://www.youtube.com/watch?v=BaW_jenozKc",
                "{input}"
            );
        }
        assert!(canonical_video_url("https://youtu.be/aA0_-123456").is_ok());
    }

    #[test]
    fn rejects_wrong_hosts_non_video_links_and_ambiguous_ids() {
        for input in [
            "",
            "  ",
            "--exec=calc.exe",
            "file:///C:/secret",
            "https://youtube.com.evil.example/watch?v=BaW_jenozKc",
            "https://youtube.com@evil.example/watch?v=BaW_jenozKc",
            "https://evil.example/youtube.com/watch?v=BaW_jenozKc",
            "https://youtube.com:443/watch?v=BaW_jenozKc",
            "https://youtube.com/playlist?list=PLtest",
            "https://youtube.com/@channel",
            "https://youtube.com/watch?other=v=BaW_jenozKc",
            "https://youtube.com/watch?v=BaW_jenozKc&v=BaW_jenozKc",
            "https://youtu.be/short",
            "https://youtu.be/BaW_jenozKc/extra",
            "https://youtu.be/BaW_jenozK%",
            "https://youtu.be/中文中文中文中文中文中",
            "https://youtu.be/BaW_jenozKc\nhttps://youtu.be/BaW_jenozKc",
        ] {
            assert!(canonical_video_url(input).is_err(), "{input}");
        }
    }

    #[test]
    fn passes_paths_and_url_as_separate_process_arguments() {
        let url = "https://www.youtube.com/watch?v=BaW_jenozKc";
        let command = download_command(
            Path::new("tools/yt-dlp"),
            Path::new("工具 目录/ffmpeg"),
            "deno:工具 目录/deno".into(),
            Path::new("视频 & 目录"),
            "bv*+ba/b",
            "mkv",
        );
        let mut command = command;
        command.args(["--", url]);
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(command.get_program(), "tools/yt-dlp");
        for expected in [
            "--ignore-config",
            "--no-plugin-dirs",
            "--no-playlist",
            "--no-overwrites",
            "--continue",
        ] {
            assert!(args.iter().any(|arg| arg == expected));
        }
        assert!(args
            .windows(2)
            .any(|args| args == ["--ffmpeg-location", "工具 目录/ffmpeg"]));
        assert!(args
            .windows(2)
            .any(|args| args == ["--paths", "视频 & 目录"]));
        assert!(args
            .windows(2)
            .any(|args| args == ["--js-runtimes", "deno:工具 目录/deno"]));
        assert!(args.windows(2).any(|args| args == ["--encoding", "utf-8"]));
        assert_eq!(&args[args.len() - 2..], &["--", url]);
    }
}
