use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use ytget::{canonical_video_url, download_command, find_tool};

const HELP: &str = "ytget — YouTube 视频下载工具

用法：
  ytget                          粘贴链接后下载
  ytget <链接>                   下载一个视频
  ytget <链接> -o <目录>         指定保存目录
  ytget --help                   查看帮助
  ytget --version                查看版本

默认保存到程序旁的 downloads 文件夹；选择最佳画质并封装为 MKV。
支持普通视频、youtu.be、Shorts、已结束直播和 embed 链接。
默认断点续传，不覆盖已有文件，不下载整个播放列表。
";

#[derive(Debug, PartialEq)]
enum Action {
    Help,
    Version,
    Download {
        url: Option<String>,
        output: Option<PathBuf>,
    },
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Action, String> {
    let mut args = args.into_iter();
    let mut url = None;
    let mut output = None;
    let mut positional_only = false;
    while let Some(arg) = args.next() {
        if !positional_only {
            match arg.to_str() {
                Some("-h" | "--help") => return Ok(Action::Help),
                Some("-V" | "--version") => return Ok(Action::Version),
                Some("--") => {
                    positional_only = true;
                    continue;
                }
                Some("-o" | "--output") => {
                    let value = args.next().ok_or("-o / --output 后需要填写目录")?;
                    if value.is_empty() || value.to_string_lossy().starts_with('-') {
                        return Err("保存目录不能为空或以 - 开头；相对目录可写成 ./目录".into());
                    }
                    if output.replace(PathBuf::from(value)).is_some() {
                        return Err("保存目录只能指定一次".into());
                    }
                    continue;
                }
                Some(value) if value.starts_with('-') => {
                    return Err(format!("未知参数：{value}；用 --help 查看用法"));
                }
                _ => {}
            }
        }
        let value = arg
            .into_string()
            .map_err(|_| "链接必须是有效的 Unicode 文本")?;
        if url.replace(value).is_some() {
            return Err("每次只接受一个视频链接；含 & 的链接请用引号包住".into());
        }
    }
    Ok(Action::Download { url, output })
}

fn prompt_url() -> Result<String, String> {
    print!("粘贴 YouTube 视频链接，然后按回车：");
    io::stdout()
        .flush()
        .map_err(|e| format!("无法显示输入提示：{e}"))?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| format!("无法读取链接：{e}"))?;
    Ok(input)
}

fn run() -> Result<(), String> {
    let (url, output) = match parse_args(env::args_os().skip(1))? {
        Action::Help => {
            print!("{HELP}");
            return Ok(());
        }
        Action::Version => {
            println!("ytget {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Action::Download { url, output } => (url, output),
    };
    let input = match url {
        Some(url) => url,
        None => {
            println!("ytget {} — YouTube 视频下载工具", env!("CARGO_PKG_VERSION"));
            prompt_url()?
        }
    };
    let url = canonical_video_url(&input)?;
    let executable = env::current_exe().map_err(|e| format!("无法定位程序目录：{e}"))?;
    let base = executable.parent().ok_or("无法确定程序目录")?;
    let downloader = find_tool(base, "yt-dlp")
        .ok_or("缺少 yt-dlp；请将官方可执行文件放到程序旁的 tools 目录，或加入 PATH")?;
    let ffmpeg = find_tool(base, "ffmpeg")
        .ok_or("缺少 FFmpeg；请将 ffmpeg 和 ffprobe 放到程序旁的 tools 目录，或加入 PATH")?;
    let (runtime_name, runtime_path) = find_tool(base, "deno")
        .map(|path| ("deno:", path))
        .or_else(|| find_tool(base, "node").map(|path| ("node:", path)))
        .ok_or(
            "缺少 JavaScript 运行时；请将 Deno 放到 tools 目录，或安装 Deno / Node.js 并加入 PATH",
        )?;
    let mut runtime = OsString::from(runtime_name);
    runtime.push(runtime_path);
    let output = output.unwrap_or_else(|| base.join("downloads"));
    let output = if output.is_absolute() {
        output
    } else {
        env::current_dir()
            .map_err(|e| format!("无法读取当前目录：{e}"))?
            .join(output)
    };
    std::fs::create_dir_all(&output)
        .map_err(|e| format!("无法创建保存目录 {}：{e}", output.display()))?;
    println!(
        "视频：{url}\n保存到：{}\n正在下载最佳画质并合并音视频……",
        output.display()
    );
    io::stdout()
        .flush()
        .map_err(|e| format!("无法显示下载信息：{e}"))?;
    let status = download_command(&downloader, &ffmpeg, runtime, &output, "bv*+ba/b", "mkv")
        .args(["--", &url])
        .status()
        .map_err(|e| format!("无法启动 yt-dlp（{}）：{e}", downloader.display()))?;
    if !status.success() {
        return Err(format!(
            "下载未完成（{status}），请查看上方 yt-dlp 错误。\n网络中断时可用相同链接和目录重新运行以续传；合并失败时请保留临时文件。"
        ));
    }
    println!("处理完成。视频保存在：{}", output.display());
    Ok(())
}

fn main() -> ExitCode {
    let interactive = env::args_os().len() == 1;
    let result = run();
    if let Err(error) = &result {
        eprintln!("错误：{error}");
    }
    if interactive {
        print!("按回车键退出……");
        let _ = io::stdout().flush();
        let _ = io::stdin().read_line(&mut String::new());
    }
    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cli_and_rejects_incomplete_or_duplicate_arguments() {
        let parse = |args: &[&str]| parse_args(args.iter().map(OsString::from));
        assert_eq!(
            parse(&[]).unwrap(),
            Action::Download {
                url: None,
                output: None
            }
        );
        assert_eq!(parse(&["--help"]).unwrap(), Action::Help);
        assert_eq!(parse(&["--version"]).unwrap(), Action::Version);
        assert_eq!(
            parse(&["-o", "视频 目录", "https://youtu.be/BaW_jenozKc"]).unwrap(),
            Action::Download {
                url: Some("https://youtu.be/BaW_jenozKc".into()),
                output: Some("视频 目录".into())
            }
        );
        assert_eq!(
            parse(&["--", "--help"]).unwrap(),
            Action::Download {
                url: Some("--help".into()),
                output: None
            }
        );
        for args in [
            vec!["-o"],
            vec!["-o", ""],
            vec!["-o", "--help"],
            vec!["a", "b"],
            vec!["--unknown"],
            vec!["-o", "a", "-o", "b"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }
}
