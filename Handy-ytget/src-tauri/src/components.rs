use crate::backend::{component_path, tools_directory};
use reqwest::Client;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    future::Future,
    io::{Read, Write},
    path::Path,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};

struct Package {
    name: &'static str,
    version: String,
    url: String,
    sha256: String,
    executables: &'static [&'static str],
}

fn packages() -> Vec<Package> {
    vec![
        Package { name: "yt-dlp", version: "2026.10.21".into(), url: "https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.21/yt-dlp.exe".into(), sha256: "33121d192d706e62f684a06f80918e3f3a8ba52ac3a65213b6951c39c7d9f555".into(), executables: &["yt-dlp"] },
        Package { name: "Deno", version: "2.9.8".into(), url: "https://github.com/denoland/deno/releases/download/v2.9.8/deno-x86_64-pc-windows-msvc.zip".into(), sha256: "cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf".into(), executables: &["deno"] },
        Package { name: "FFmpeg / FFprobe", version: "9.0.2".into(), url: "https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-9.0.2-essentials_build.zip".into(), sha256: "60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba".into(), executables: &["ffmpeg", "ffprobe"] },
    ]
}

fn client() -> Result<Client, String> {
    Client::builder()
        .https_only(true)
        .user_agent("Handy-ytget/0.3.1")
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(900))
        .build()
        .map_err(|e| format!("无法初始化组件下载：{e}"))
}

fn notify(app: &AppHandle, message: impl AsRef<str>, percent: Option<f64>) {
    let _ = app.emit(
        "component-event",
        json!({ "message":message.as_ref(), "percent":percent }),
    );
}

fn network_error(error: reqwest::Error) -> String {
    use std::error::Error;
    let mut message = error.to_string();
    let mut cause = error.source();
    while let Some(source) = cause {
        message.push_str(&format!("；{source}"));
        cause = source.source();
    }
    message
}

async fn request<T>(
    closing: &AtomicBool,
    future: impl Future<Output = Result<T, reqwest::Error>>,
) -> Result<T, String> {
    tokio::select! {
        result = future => result.map_err(network_error),
        _ = async {
            while !closing.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(100)).await; }
        } => Err("窗口正在关闭".into()),
    }
}

fn version_numbers(value: &str) -> Option<Vec<u32>> {
    let mut values = value
        .split('.')
        .map(str::parse)
        .collect::<Result<Vec<u32>, _>>()
        .ok()?;
    if !(2..=4).contains(&values.len()) {
        return None;
    }
    values.resize(4, 0);
    Some(values)
}

fn parse_version(name: &str, output: &str) -> Option<String> {
    let first = output.lines().next()?;
    let raw = match name {
        "yt-dlp" => first.trim(),
        "deno" => first.strip_prefix("deno ")?.split_whitespace().next()?,
        _ => first
            .split_whitespace()
            .nth(2)?
            .trim_start_matches('n')
            .split('-')
            .next()?,
    };
    version_numbers(raw).map(|_| raw.into())
}

fn current_versions(app: &AppHandle, package: &Package) -> Result<Vec<Option<String>>, String> {
    package
        .executables
        .iter()
        .map(|name| {
            let Some(path) = component_path(app, name)? else {
                return Ok(None);
            };
            let mut command = Command::new(path);
            command.arg(if name.starts_with("ff") {
                "-version"
            } else {
                "--version"
            });
            crate::backend::background(&mut command);
            Ok(command
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| parse_version(name, &String::from_utf8_lossy(&o.stdout))))
        })
        .collect()
}

fn newer(installed: Option<&str>, latest: &str) -> bool {
    match installed.and_then(version_numbers) {
        Some(current) => version_numbers(latest).is_some_and(|remote| remote > current),
        None => true,
    }
}

fn checksum(text: &str, filename: Option<&str>) -> Result<String, String> {
    text.lines()
        .find_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            let hash = if filename.is_none() && fields.first() == Some(&"Hash") {
                line.split_once(':')?.1.trim()
            } else {
                *fields.first()?
            };
            if filename
                .is_some_and(|name| fields.get(1).map(|s| s.trim_start_matches('*')) != Some(name))
            {
                return None;
            }
            (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| hash.to_ascii_lowercase())
        })
        .ok_or_else(|| "官方发布页未返回有效的 SHA256，已停止安装".into())
}

async fn text(client: &Client, url: &str, closing: &AtomicBool) -> Result<String, String> {
    let mut response = request(
        closing,
        client.get(url).timeout(Duration::from_secs(30)).send(),
    )
    .await?
    .error_for_status()
    .map_err(network_error)?;
    let mut output = Vec::new();
    while let Some(chunk) = request(closing, response.chunk()).await? {
        if output.len() + chunk.len() > 256 * 1024 {
            return Err("官方版本信息超过大小限制".into());
        }
        output.extend_from_slice(&chunk);
    }
    String::from_utf8(output).map_err(|e| e.to_string())
}

async fn release(
    client: &Client,
    repository: &str,
    closing: &AtomicBool,
) -> Result<String, String> {
    let prefix = format!("https://github.com/{repository}/releases/tag/");
    let response = request(
        closing,
        client
            .get(format!("https://github.com/{repository}/releases/latest"))
            .timeout(Duration::from_secs(30))
            .send(),
    )
    .await?
    .error_for_status()
    .map_err(network_error)?;
    let version = response
        .url()
        .as_str()
        .strip_prefix(&prefix)
        .ok_or("无法识别官方发布版本")?
        .trim_start_matches('v');
    if version_numbers(version).is_none() {
        return Err("官方版本号格式无效".into());
    }
    Ok(version.into())
}

async fn latest_packages(client: &Client, closing: &AtomicBool) -> Result<Vec<Package>, String> {
    let mut result = packages();
    for package in &mut result {
        let (url, sums, filename) = match package.name {
            "yt-dlp" => {
                package.version = release(client, "yt-dlp/yt-dlp", closing).await?;
                let base = format!(
                    "https://github.com/yt-dlp/yt-dlp/releases/download/{}/",
                    package.version
                );
                (
                    format!("{base}yt-dlp.exe"),
                    format!("{base}SHA2-256SUMS"),
                    Some("yt-dlp.exe"),
                )
            }
            "Deno" => {
                package.version = release(client, "denoland/deno", closing).await?;
                let url = format!("https://github.com/denoland/deno/releases/download/v{}/deno-x86_64-pc-windows-msvc.zip", package.version);
                (url.clone(), format!("{url}.sha256sum"), None)
            }
            _ => {
                package.version = text(
                    client,
                    "https://www.gyan.dev/ffmpeg/builds/release-version",
                    closing,
                )
                .await?
                .trim()
                .into();
                if version_numbers(&package.version).is_none() {
                    return Err("FFmpeg 官方版本号格式无效".into());
                }
                let url = format!(
                    "https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-{}-essentials_build.zip",
                    package.version
                );
                (url.clone(), format!("{url}.sha256"), None)
            }
        };
        package.sha256 = checksum(&text(client, &sums, closing).await?, filename)
            .map_err(|e| format!("{}：{e}", package.name))?;
        package.url = url;
    }
    Ok(result)
}

#[derive(Serialize)]
pub struct Update {
    name: &'static str,
    installed: String,
    latest: String,
    available: bool,
}

pub async fn check_updates(app: &AppHandle, closing: &AtomicBool) -> Result<Vec<Update>, String> {
    notify(app, "正在查询官方组件版本…", None);
    latest_packages(&client()?, closing)
        .await?
        .into_iter()
        .map(|package| {
            let current = current_versions(app, &package)?;
            let using_node = package.name == "Deno"
                && current[0].is_none()
                && component_path(app, "node")?.is_some();
            let installed = if using_node {
                "使用系统 Node.js".into()
            } else {
                current
                    .iter()
                    .map(|v| v.as_deref().unwrap_or("未安装或无法读取版本"))
                    .collect::<Vec<_>>()
                    .join(" / ")
            };
            let available = !using_node
                && current
                    .iter()
                    .any(|v| newer(v.as_deref(), &package.version));
            Ok(Update {
                name: package.name,
                installed,
                latest: package.version,
                available,
            })
        })
        .collect()
}

// shortcut: 单个组件包和可执行文件限制为 512 MiB，官方包超过此大小时再调整。
const MAX_SIZE: u64 = 512 * 1024 * 1024;

async fn download(
    app: &AppHandle,
    client: &Client,
    package: &Package,
    path: &Path,
    closing: &AtomicBool,
) -> Result<(), String> {
    let mut response = request(closing, client.get(&package.url).send())
        .await?
        .error_for_status()
        .map_err(network_error)?;
    let total = response.content_length();
    if total.is_some_and(|n| n > MAX_SIZE) {
        return Err("组件包超过大小限制".into());
    }
    let mut file = File::create(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut last = Instant::now();
    while let Some(chunk) = request(closing, response.chunk()).await? {
        size += chunk.len() as u64;
        if size > MAX_SIZE {
            return Err("组件包超过大小限制".into());
        }
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        hash.update(&chunk);
        if last.elapsed() > Duration::from_millis(200) {
            let percent = total
                .filter(|n| *n > 0)
                .map(|n| size as f64 / n as f64 * 100.0);
            notify(
                app,
                format!(
                    "正在下载 {} {} · {:.1} MB",
                    package.name,
                    package.version,
                    size as f64 / 1048576.0
                ),
                percent,
            );
            last = Instant::now();
        }
    }
    let actual = format!("{:x}", hash.finalize());
    verify_hash(&actual, &package.sha256)?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

fn verify_hash(actual: &str, expected: &str) -> Result<(), String> {
    if actual != expected {
        return Err("组件 SHA256 校验失败，未替换现有文件，请重试".into());
    }
    Ok(())
}

fn extract(archive: &Path, executable: &str, target: &Path) -> Result<(), String> {
    let mut package = zip::ZipArchive::new(File::open(archive).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let name = format!("{executable}.exe");
    let index = package
        .file_names()
        .position(|p| p == name || p.ends_with(&format!("/bin/{name}")))
        .ok_or("组件包缺少可执行文件")?;
    let entry = package.by_index(index).map_err(|e| e.to_string())?;
    if entry.enclosed_name().is_none() || !entry.is_file() || entry.size() > MAX_SIZE {
        return Err("组件包中的文件无效".into());
    }
    let copied = std::io::copy(
        &mut entry.take(MAX_SIZE + 1),
        &mut File::create(target).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if copied > MAX_SIZE {
        return Err("组件文件超过大小限制".into());
    }
    Ok(())
}

pub async fn install(app: &AppHandle, update: bool, closing: &AtomicBool) -> Result<(), String> {
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return Err("组件自动安装目前仅支持 Windows x64".into());
    }
    let client = client()?;
    notify(
        app,
        if update {
            "正在检查组件更新…"
        } else {
            "正在检查缺失组件…"
        },
        None,
    );
    let packages = if update {
        latest_packages(&client, closing).await?
    } else {
        packages()
    };
    let directory = tools_directory(app)?;
    fs::create_dir_all(&directory).map_err(|e| format!("无法创建组件目录：{e}"))?;
    for package in packages {
        if closing.load(Ordering::SeqCst) {
            return Err("窗口正在关闭".into());
        }
        if package.name == "Deno"
            && component_path(app, "deno")?.is_none()
            && component_path(app, "node")?.is_some()
        {
            continue;
        }
        let required = if update {
            let versions = current_versions(app, &package)?;
            package
                .executables
                .iter()
                .enumerate()
                .filter_map(|(index, name)| {
                    newer(versions[index].as_deref(), &package.version).then_some(*name)
                })
                .collect::<Vec<_>>()
        } else {
            let mut missing = Vec::new();
            for name in package.executables {
                if component_path(app, name)?.is_none() {
                    missing.push(*name);
                }
            }
            missing
        };
        if required.is_empty() {
            continue;
        }
        let staging = tempfile::Builder::new()
            .prefix(".install-")
            .tempdir_in(&directory)
            .map_err(|e| e.to_string())?;
        let archive = staging.path().join("download");
        notify(
            app,
            format!("正在下载 {} {}…", package.name, package.version),
            Some(0.0),
        );
        download(app, &client, &package, &archive, closing)
            .await
            .map_err(|e| format!("{} 安装失败：{e}", package.name))?;
        notify(app, format!("正在校验并安装 {}…", package.name), None);
        for name in &required {
            let target = staging.path().join(format!("{name}.exe"));
            if package.url.ends_with(".zip") {
                extract(&archive, name, &target)?;
            } else {
                fs::copy(&archive, &target).map_err(|e| e.to_string())?;
            }
            let mut header = [0; 2];
            File::open(&target)
                .and_then(|mut f| f.read_exact(&mut header))
                .map_err(|e| e.to_string())?;
            if header != *b"MZ" {
                return Err("组件不是有效的 Windows 可执行文件".into());
            }
        }
        if closing.load(Ordering::SeqCst) {
            return Err("窗口正在关闭".into());
        }
        for name in required {
            fs::rename(
                staging.path().join(format!("{name}.exe")),
                directory.join(format!("{name}.exe")),
            )
            .map_err(|e| format!("无法替换 {name}，请关闭其他下载程序后重试：{e}"))?;
        }
    }
    notify(app, "下载组件已就绪", Some(100.0));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions_compare_without_downgrading_or_misreading_dates() {
        assert!(newer(Some("2026.10.21"), "2026.11.01"));
        assert!(!newer(Some("2.10.0"), "2.9.8"));
        assert!(!newer(Some("9.0.2"), "9.0.2"));
        assert!(newer(None, "9.0.2"));
        assert_eq!(
            parse_version(
                "ffmpeg",
                "ffmpeg version 9.0.2-essentials_build-www.gyan.dev Copyright"
            ),
            Some("9.0.2".into())
        );
        assert_eq!(
            parse_version("deno", "deno 2.9.8 (stable, release)"),
            Some("2.9.8".into())
        );
        assert!(version_numbers("../../file").is_none());
    }
    #[test]
    fn checksums_match_the_requested_asset_and_reject_tampering() {
        let hash = "a".repeat(64);
        assert_eq!(
            checksum(
                &format!("{}  wrong.exe\n{hash} *yt-dlp.exe", "b".repeat(64)),
                Some("yt-dlp.exe")
            )
            .unwrap(),
            hash
        );
        assert!(checksum("bad checksum", None).is_err());
        assert_eq!(
            checksum(
                &format!(
                    "\r\nAlgorithm : SHA256\r\nHash      : {}\r\nPath      : C:\\a\\deno.zip\r\n",
                    hash.to_ascii_uppercase()
                ),
                None
            )
            .unwrap(),
            hash
        );
        assert!(verify_hash(&"b".repeat(64), &hash).is_err());
    }
    #[test]
    fn extraction_rejects_traversal_and_does_not_create_the_destination() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("bad.zip");
        let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
        writer
            .start_file(
                "../../bin/ffmpeg.exe",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"MZbad").unwrap();
        writer.finish().unwrap();
        let target = directory.path().join("ffmpeg.exe");
        assert!(extract(&archive, "ffmpeg", &target).is_err());
        assert!(!target.exists());
    }
}
