use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    env,
    ffi::OsString,
    fs,
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
use ytget::{canonical_video_url, download_command, find_tool, media_command};

#[derive(Default)]
pub struct DownloadState {
    active: Mutex<Option<Arc<Task>>>,
    closing: AtomicBool,
    maintaining: AtomicBool,
}

struct Task {
    child: Mutex<Child>,
    cancelled: AtomicBool,
    paused: AtomicBool,
    processing: AtomicBool,
    download: bool,
}

pub(crate) fn background(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
}

impl Task {
    fn stop(&self, pause: bool) -> Result<(), String> {
        let mut child = self.child.lock().map_err(|_| "无法访问下载进程")?;
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return if pause {
                Err("任务已结束，无法暂停".into())
            } else {
                Ok(())
            };
        }
        if pause && (!self.download || self.processing.load(Ordering::SeqCst)) {
            return Err("当前正在准备或处理文件，请稍候再暂停".into());
        }
        #[cfg(windows)]
        {
            let root = env::var_os("SystemRoot").ok_or("无法定位 Windows 系统目录")?;
            let mut command = Command::new(PathBuf::from(root).join("System32/taskkill.exe"));
            command.args(["/PID", &child.id().to_string(), "/T", "/F"]);
            background(&mut command);
            let result = command.output().map_err(|e| format!("无法取消任务：{e}"))?;
            if !result.status.success() && child.try_wait().map_err(|e| e.to_string())?.is_none() {
                return Err("无法结束下载进程，请重试取消操作".into());
            }
        }
        #[cfg(not(windows))]
        child.kill().map_err(|e| format!("无法取消任务：{e}"))?;
        self.paused.store(pause, Ordering::SeqCst);
        self.cancelled.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct TaskLease<'a>(&'a DownloadState);
impl Drop for TaskLease<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.active.lock() {
            if let Some(task) = active.take() {
                let _ = task.stop(false);
            }
        }
    }
}

struct Tools {
    downloader: PathBuf,
    ffmpeg: PathBuf,
    runtime: OsString,
}

fn program_dir() -> Result<PathBuf, String> {
    env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位程序目录".into())
}

impl Tools {
    fn resolve(app: &AppHandle) -> Result<Self, String> {
        let missing = |name: &str| format!("缺少 {name}，将自动下载；安装失败后可点击“重试安装”。");
        let downloader = component_path(app, "yt-dlp")?.ok_or_else(|| missing("yt-dlp"))?;
        let ffmpeg = component_path(app, "ffmpeg")?.ok_or_else(|| missing("FFmpeg"))?;
        component_path(app, "ffprobe")?.ok_or_else(|| missing("FFprobe"))?;
        let (name, path) = component_path(app, "deno")?
            .map(|p| ("deno:", p))
            .or(component_path(app, "node")?.map(|p| ("node:", p)))
            .ok_or_else(|| missing("Deno / Node.js"))?;
        let mut runtime = OsString::from(name);
        runtime.push(path);
        Ok(Self {
            downloader,
            ffmpeg,
            runtime,
        })
    }
}

pub(crate) fn tools_directory(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|p| p.join("tools"))
        .map_err(|e| e.to_string())
}

pub(crate) fn component_path(app: &AppHandle, name: &str) -> Result<Option<PathBuf>, String> {
    let cached = tools_directory(app)?.join(format!("{name}{}", env::consts::EXE_SUFFIX));
    Ok(if cached.is_file() {
        Some(cached)
    } else {
        find_tool(&program_dir()?, name)
    })
}

fn settings_file(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join("output-directory.txt"))
        .map_err(|e| e.to_string())
}

fn output_directory(app: &AppHandle) -> Result<PathBuf, String> {
    let file = settings_file(app)?;
    match fs::read_to_string(&file) {
        Ok(value) => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err("保存目录设置无效，请重新选择文件夹".into());
            }
            Ok(path)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(program_dir()?.join("downloads")),
        Err(e) => Err(format!("无法读取保存目录设置：{e}")),
    }
}

fn ensure_idle(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<DownloadState>();
    if state.maintaining.load(Ordering::SeqCst)
        || state
            .active
            .lock()
            .map_err(|_| "无法读取任务状态")?
            .is_some()
    {
        return Err("已有任务正在运行，请先等待或取消".into());
    }
    Ok(())
}

#[derive(Serialize)]
pub struct Settings {
    output: String,
    ready: bool,
    message: String,
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Result<Settings, String> {
    let output = output_directory(&app)?.to_string_lossy().into_owned();
    let (ready, message) = match Tools::resolve(&app) {
        Ok(_) => (true, "yt-dlp、FFmpeg 和 JavaScript 运行时已就绪。".into()),
        Err(e) => (false, e),
    };
    Ok(Settings {
        output,
        ready,
        message,
    })
}

struct MaintenanceLease<'a>(&'a AtomicBool);
impl Drop for MaintenanceLease<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn start_maintenance(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<DownloadState>();
    let active = state.active.lock().map_err(|_| "无法读取任务状态")?;
    if state.closing.load(Ordering::SeqCst) {
        return Err("窗口正在关闭".into());
    }
    if active.is_some() || state.maintaining.swap(true, Ordering::SeqCst) {
        return Err("已有任务正在运行，请稍候".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn install_tools(app: AppHandle, update: bool) -> Result<Settings, String> {
    tauri::async_runtime::spawn_blocking(move || {
        start_maintenance(&app)?;
        let state = app.state::<DownloadState>();
        let _lease = MaintenanceLease(&state.maintaining);
        tauri::async_runtime::block_on(crate::components::install(&app, update, &state.closing))?;
        get_settings(app.clone())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn check_tools_updates(app: AppHandle) -> Result<Vec<crate::components::Update>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        start_maintenance(&app)?;
        let state = app.state::<DownloadState>();
        let _lease = MaintenanceLease(&state.maintaining);
        tauri::async_runtime::block_on(crate::components::check_updates(&app, &state.closing))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn choose_directory(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        ensure_idle(&app)?;
        let window = app.get_webview_window("main").ok_or("无法定位主窗口")?;
        let mut picker = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("选择视频保存位置");
        if let Ok(path) = output_directory(&app) {
            picker = picker.set_directory(path);
        }
        let Some(selected) = picker.blocking_pick_folder() else {
            return Ok(None);
        };
        let path = selected.into_path().map_err(|e| e.to_string())?;
        if !path.is_absolute() || !path.is_dir() {
            return Err("请选择有效的本地文件夹".into());
        }
        ensure_idle(&app)?;
        let text = path.to_str().ok_or("保存目录包含不支持的字符")?.to_owned();
        let file = settings_file(&app)?;
        fs::create_dir_all(file.parent().ok_or("无法定位设置目录")?).map_err(|e| e.to_string())?;
        let temporary = file.with_extension("tmp");
        fs::write(&temporary, &text).map_err(|e| format!("无法保存目录设置：{e}"))?;
        fs::rename(&temporary, &file).map_err(|e| format!("无法更新目录设置：{e}"))?;
        Ok(Some(text))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn open_directory(app: AppHandle) -> Result<(), String> {
    let path = output_directory(&app)?;
    fs::create_dir_all(&path).map_err(|e| format!("无法创建下载目录：{e}"))?;
    #[cfg(windows)]
    let opener = "explorer.exe";
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let opener = "xdg-open";
    Command::new(opener)
        .arg(path)
        .spawn()
        .map_err(|e| format!("无法打开文件夹：{e}"))?;
    Ok(())
}

#[tauri::command]
pub fn cancel_task(app: AppHandle) -> Result<(), String> {
    cancel(&app)
}

pub fn cancel(app: &AppHandle) -> Result<(), String> {
    let task = app
        .state::<DownloadState>()
        .active
        .lock()
        .map_err(|_| "无法读取任务状态")?
        .clone();
    if let Some(task) = task {
        task.stop(false)?;
    }
    Ok(())
}

#[tauri::command]
pub fn pause_task(app: AppHandle) -> Result<(), String> {
    let task = app
        .state::<DownloadState>()
        .active
        .lock()
        .map_err(|_| "无法读取任务状态")?
        .clone()
        .ok_or("当前没有可暂停的下载")?;
    task.stop(true)
}

pub fn shutdown(app: &AppHandle) {
    app.state::<DownloadState>()
        .closing
        .store(true, Ordering::SeqCst);
    let _ = cancel(app);
}

fn progress_payload(line: &str) -> Option<Value> {
    let data: Value = serde_json::from_str(line.strip_prefix("YTGET_PROGRESS:")?).ok()?;
    let downloaded = data["downloaded"].as_f64()?;
    let total = data["total"].as_f64().or_else(|| data["estimate"].as_f64());
    let percent = total
        .filter(|n| *n > 0.0)
        .map(|n| (downloaded / n * 100.0).clamp(0.0, 100.0));
    Some(
        json!({ "kind": "progress", "percent": percent, "speed": data["speed"].as_f64(), "eta": data["eta"].as_f64(), "status": data["status"].as_str() }),
    )
}

fn processing_label(line: &str) -> Option<&'static str> {
    if line.starts_with("[Merger]") || line.starts_with("[VideoRemuxer]") {
        Some("正在合并音视频")
    } else if line.starts_with("[ExtractAudio]") {
        Some("正在生成音频")
    } else if line.starts_with("[SubtitlesConvertor]") {
        Some("正在转换字幕")
    } else if line.starts_with("[Fixup") {
        Some("正在整理媒体文件")
    } else {
        None
    }
}

fn read_pipe(
    pipe: impl Read + Send + 'static,
    app: AppHandle,
    task: Arc<Task>,
    stdout: bool,
) -> thread::JoinHandle<Result<String, String>> {
    thread::spawn(move || {
        let mut output = String::new();
        for line in BufReader::new(pipe).lines() {
            let line = line.map_err(|e| format!("无法读取下载器输出：{e}"))?;
            if task.download && !line.starts_with("YTGET_SUBTITLE:") {
                let payload = if let Some(progress) = progress_payload(&line) {
                    task.processing.store(false, Ordering::SeqCst);
                    progress
                } else if let Some(label) = processing_label(&line) {
                    task.processing.store(true, Ordering::SeqCst);
                    json!({ "kind": "processing", "message": label })
                } else {
                    json!({ "kind": "log", "message": line })
                };
                let _ = app.emit("download-event", payload);
            } else if !task.download && !stdout {
                let _ = app.emit("download-event", json!({ "kind":"log", "message":line }));
            }
            output.push_str(&line);
            output.push('\n');
        }
        Ok(output)
    })
}

fn run_process(app: &AppHandle, mut command: Command, download: bool) -> Result<String, String> {
    let state = app.state::<DownloadState>();
    let mut active = state.active.lock().map_err(|_| "无法读取任务状态")?;
    if state.closing.load(Ordering::SeqCst) {
        return Err("窗口正在关闭".into());
    }
    if active.is_some() {
        return Err("已有任务正在运行，请先等待或取消".into());
    }
    if state.maintaining.load(Ordering::SeqCst) {
        return Err("正在准备下载组件，请稍候".into());
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    background(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法启动下载器：{e}"))?;
    let stdout = child.stdout.take().ok_or("无法读取下载器标准输出")?;
    let stderr = child.stderr.take().ok_or("无法读取下载器错误输出")?;
    let task = Arc::new(Task {
        child: Mutex::new(child),
        cancelled: AtomicBool::new(false),
        paused: AtomicBool::new(false),
        processing: AtomicBool::new(false),
        download,
    });
    *active = Some(task.clone());
    drop(active);
    let _ = app.emit("download-event", json!({"kind":"started"}));
    let _lease = TaskLease(&state);
    let stdout = read_pipe(stdout, app.clone(), task.clone(), true);
    let stderr = read_pipe(stderr, app.clone(), task.clone(), false);
    let status = loop {
        if let Some(status) = task
            .child
            .lock()
            .map_err(|_| "无法读取进程状态")?
            .try_wait()
            .map_err(|e| e.to_string())?
        {
            break status;
        }
        thread::sleep(Duration::from_millis(80));
    };
    let stdout = stdout.join().map_err(|_| "下载器输出读取线程异常")??;
    let stderr = stderr.join().map_err(|_| "下载器错误读取线程异常")??;
    if task.paused.load(Ordering::SeqCst) {
        return Err("任务已暂停".into());
    }
    if task.cancelled.load(Ordering::SeqCst) {
        return Err("任务已取消".into());
    }
    if !status.success() {
        let detail: Vec<_> = stderr.lines().rev().take(5).collect();
        return Err(format!(
            "处理失败（{status}）\n{}",
            detail.into_iter().rev().collect::<Vec<_>>().join("\n")
        ));
    }
    Ok(stdout)
}

#[derive(Serialize)]
pub struct Subtitle {
    language: String,
    name: String,
    automatic: bool,
}

#[derive(Deserialize)]
pub struct SubtitleChoice {
    language: String,
    automatic: bool,
}

fn valid_subtitle_language(language: &str) -> bool {
    !language.is_empty()
        && language.len() <= 64
        && language
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn subtitles(value: &Value) -> Vec<Subtitle> {
    let mut tracks = Vec::new();
    for (key, automatic) in [("subtitles", false), ("automatic_captions", true)] {
        if let Some(languages) = value[key].as_object() {
            for (language, formats) in languages {
                if language == "live_chat" || !valid_subtitle_language(language) {
                    continue;
                }
                let Some(format) = formats.as_array().and_then(|formats| {
                    formats
                        .iter()
                        .find(|format| matches!(format["ext"].as_str(), Some("vtt" | "srt")))
                }) else {
                    continue;
                };
                tracks.push(Subtitle {
                    language: language.clone(),
                    name: format["name"].as_str().unwrap_or(language).into(),
                    automatic,
                });
            }
        }
    }
    tracks
}

#[derive(Serialize)]
pub struct Video {
    url: String,
    title: String,
    channel: String,
    duration: Option<f64>,
    description: String,
    thumbnail: String,
    heights: Vec<u32>,
    subtitles: Vec<Subtitle>,
}

fn parse_video(url: String, output: &str) -> Result<Video, String> {
    let value: Value =
        serde_json::from_str(output).map_err(|e| format!("无法读取视频信息：{e}"))?;
    if value["is_live"].as_bool() == Some(true)
        || value["live_status"].as_str() == Some("is_upcoming")
    {
        return Err("当前视频正在直播或尚未开始，请在直播结束后下载".into());
    }
    let title = value["title"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("解析器未返回视频标题")?
        .to_owned();
    let heights: BTreeSet<_> = value["formats"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["vcodec"].as_str().is_some_and(|c| c != "none"))
        .filter_map(|f| f["height"].as_u64())
        .filter(|h| (144..=8640).contains(h))
        .map(|h| h as u32)
        .collect();
    let id = url.rsplit('=').next().ok_or("视频链接缺少 ID")?;
    Ok(Video {
        thumbnail: format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"),
        url,
        title,
        channel: value["channel"]
            .as_str()
            .or_else(|| value["uploader"].as_str())
            .unwrap_or("YouTube")
            .into(),
        duration: value["duration"].as_f64(),
        description: value["description"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(1000)
            .collect(),
        heights: heights.into_iter().rev().collect(),
        subtitles: subtitles(&value),
    })
}

#[tauri::command]
pub async fn inspect_video(app: AppHandle, url: String) -> Result<Video, String> {
    let url = canonical_video_url(&url)?;
    tauri::async_runtime::spawn_blocking(move || {
        let tools = Tools::resolve(&app)?;
        let mut command = Command::new(tools.downloader);
        command
            .args([
                "--ignore-config",
                "--no-plugin-dirs",
                "--no-cache-dir",
                "--encoding",
                "utf-8",
                "--no-playlist",
                "--skip-download",
                "--dump-single-json",
                "--no-warnings",
                "--socket-timeout",
                "20",
                "--no-js-runtimes",
                "--js-runtimes",
            ])
            .arg(tools.runtime)
            .args(["--", &url]);
        parse_video(url, &run_process(&app, command, false)?)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    Mkv,
    Mp4,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum DownloadMode {
    Video,
    VideoSubtitles,
    Audio,
    AudioSubtitles,
    Subtitles,
}

fn format_selector(height: Option<u32>) -> Result<String, String> {
    match height {
        None => Ok("bv*+ba/b".into()),
        Some(h) if (144..=8640).contains(&h) => Ok(format!("bv*[height<={h}]+ba/b[height<={h}]")),
        _ => Err("画质参数无效".into()),
    }
}

#[derive(Serialize)]
pub struct DownloadResult {
    file: Option<String>,
    subtitle_files: Vec<String>,
}

fn media_download_command(
    tools: &Tools,
    output: &std::path::Path,
    mode: DownloadMode,
    height: Option<u32>,
    container: Container,
    subtitle: Option<&SubtitleChoice>,
) -> Result<Command, String> {
    let needs_subtitle = matches!(
        mode,
        DownloadMode::VideoSubtitles | DownloadMode::AudioSubtitles | DownloadMode::Subtitles
    );
    if needs_subtitle != subtitle.is_some() {
        return Err("下载模式与字幕选择不一致，请选择字幕语言或切换下载模式".into());
    }
    let mut command = match mode {
        DownloadMode::Video | DownloadMode::VideoSubtitles => {
            let format = format_selector(height)?;
            let extension = match container {
                Container::Mkv => "mkv",
                Container::Mp4 => "mp4",
            };
            download_command(
                &tools.downloader,
                &tools.ffmpeg,
                tools.runtime.clone(),
                output,
                &format,
                extension,
            )
        }
        DownloadMode::Audio | DownloadMode::AudioSubtitles | DownloadMode::Subtitles => {
            if height.is_some() {
                return Err("当前模式不接受画质参数".into());
            }
            let mut command = media_command(
                &tools.downloader,
                &tools.ffmpeg,
                tools.runtime.clone(),
                output,
                "bestaudio",
            );
            if matches!(mode, DownloadMode::Subtitles) {
                command.args(["--skip-download", "--ignore-no-formats-error"]);
            } else {
                command.args([
                    "--extract-audio",
                    "--audio-format",
                    "mp3",
                    "--audio-quality",
                    "0",
                ]);
            }
            command
        }
    };
    if let Some(subtitle) = subtitle {
        if !valid_subtitle_language(&subtitle.language) {
            return Err("字幕语言参数无效".into());
        }
        let language = format!("^{}$", subtitle.language.replace('.', "\\."));
        command.args(if subtitle.automatic {
            ["--write-auto-subs", "--no-write-subs"]
        } else {
            ["--write-subs", "--no-write-auto-subs"]
        });
        command.args([
            "--print",
            "before_dl:YTGET_SUBTITLE:%(requested_subtitles)j",
            "--sub-langs",
            &language,
            "--sub-format",
            "srt/vtt",
            "--convert-subs",
            "srt",
        ]);
        let source = if subtitle.automatic { "auto" } else { "manual" };
        command.arg("--paths").arg(format!(
            "temp:{}",
            output.join(".ytget").join(source).display()
        ));
        command.args([
            "--output",
            if subtitle.automatic {
                "subtitle:%(title).120B [%(id)s].auto.%(ext)s"
            } else {
                "subtitle:%(title).120B [%(id)s].manual.%(ext)s"
            },
        ]);
    }
    Ok(command)
}

fn reported_file(stdout: &str, prefix: &str) -> Option<String> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix(prefix))
        .filter_map(|value| serde_json::from_str::<String>(value).ok())
        .next_back()
}

fn subtitle_file(stdout: &str, choice: &SubtitleChoice) -> Result<String, String> {
    let selected = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("YTGET_SUBTITLE:"))
        .filter_map(|value| serde_json::from_str::<Value>(value).ok())
        .next_back();
    if !selected
        .as_ref()
        .is_some_and(|tracks| tracks.get(&choice.language).is_some_and(Value::is_object))
    {
        return Err("当前视频没有所选字幕，请重新解析并选择可用语言".into());
    }
    let base = reported_file(stdout, "YTGET_BASE:").ok_or("下载器未返回保存文件名")?;
    let source = if choice.automatic { "auto" } else { "manual" };
    let file = PathBuf::from(base).with_extension(format!("{source}.{}.srt", choice.language));
    if !file.is_file() {
        return Err("所选字幕文件未生成，请重新解析并检查字幕是否仍可用".into());
    }
    Ok(file.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn download_video(
    app: AppHandle,
    url: String,
    height: Option<u32>,
    container: Container,
    mode: DownloadMode,
    subtitle: Option<SubtitleChoice>,
) -> Result<DownloadResult, String> {
    let url = canonical_video_url(&url)?;
    tauri::async_runtime::spawn_blocking(move || {
        let tools = Tools::resolve(&app)?;
        let output = output_directory(&app)?;
        let mut command = media_download_command(&tools, &output, mode, height, container, subtitle.as_ref())?;
        fs::create_dir_all(&output).map_err(|e| format!("无法创建保存目录：{e}"))?;
        command.args(["--no-simulate", "--no-quiet", "--print", "after_move:YTGET_FILE:%(filepath)j", "--print", "after_video:YTGET_BASE:%(filename)j", "--progress-template", "download:YTGET_PROGRESS:{\"downloaded\":%(progress.downloaded_bytes|0)j,\"total\":%(progress.total_bytes|null)j,\"estimate\":%(progress.total_bytes_estimate|null)j,\"speed\":%(progress.speed|null)j,\"eta\":%(progress.eta|null)j,\"status\":%(progress.status)j}", "--", &url]);
        let stdout = run_process(&app, command, true)?;
        let file = if matches!(mode, DownloadMode::Subtitles) { None } else { reported_file(&stdout, "YTGET_FILE:") };
        let subtitle_files = subtitle.as_ref().map(|choice| subtitle_file(&stdout, choice)).transpose()?.into_iter().collect();
        Ok(DownloadResult { file, subtitle_files })
    }).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_handles_estimates_unknown_totals_and_stream_completion() {
        let data = progress_payload(r#"YTGET_PROGRESS:{"downloaded":50,"total":100,"speed":2048,"eta":2,"status":"downloading"}"#).unwrap();
        assert_eq!(data["percent"], 50.0);
        assert_eq!(data["speed"], 2048.0);
        assert_eq!(
            progress_payload(r#"YTGET_PROGRESS:{"downloaded":50,"estimate":200}"#).unwrap()
                ["percent"],
            25.0
        );
        assert!(
            progress_payload(r#"YTGET_PROGRESS:{"downloaded":50,"total":"NA"}"#).unwrap()
                ["percent"]
                .is_null()
        );
        assert!(progress_payload("a normal log line").is_none());
        assert!(progress_payload("YTGET_PROGRESS:bad json").is_none());
    }
    #[test]
    fn metadata_filters_audio_formats_and_refuses_active_streams() {
        let url = "https://www.youtube.com/watch?v=jNQXAC9IVRw".to_string();
        let video = parse_video(url.clone(), r#"{"title":"标题","channel":"频道","duration":19,"formats":[{"vcodec":"av1","height":1080},{"vcodec":"h264","height":720},{"vcodec":"av1","height":1080},{"vcodec":"none","height":2160}]}"#).unwrap();
        assert_eq!(video.heights, [1080, 720]);
        assert_eq!(video.title, "标题");
        assert!(video.thumbnail.ends_with("/jNQXAC9IVRw/hqdefault.jpg"));
        assert!(parse_video(url.clone(), r#"{"title":"live","is_live":true}"#).is_err());
        assert!(parse_video(url, r#"{"formats":[]}"#).is_err());
    }
    #[test]
    fn quality_and_container_are_validated_at_the_command_boundary() {
        assert_eq!(format_selector(None).unwrap(), "bv*+ba/b");
        assert_eq!(
            format_selector(Some(1080)).unwrap(),
            "bv*[height<=1080]+ba/b[height<=1080]"
        );
        assert!(format_selector(Some(0)).is_err());
        assert!(format_selector(Some(u32::MAX)).is_err());
        assert!(serde_json::from_str::<Container>("\"exe\"").is_err());
    }
    #[test]
    fn subtitles_keep_language_and_source_and_exclude_chat_or_unsupported_formats() {
        let tracks = subtitles(&json!({
            "subtitles": {"en": [{"ext":"vtt", "name":"English"}], "live_chat":[{"ext":"vtt"}], "bad.*":[{"ext":"vtt"}], "fr":[{"ext":"json3"}]},
            "automatic_captions": {"en": [{"ext":"vtt", "name":"English (auto)"}], "zh-Hans":[{"ext":"srt"}]}
        }));
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].language, "en");
        assert!(!tracks[0].automatic);
        assert!(tracks[1].automatic);
        assert_eq!(tracks[2].name, "zh-Hans");
        assert!(subtitles(&json!({})).is_empty());
    }
    #[test]
    fn audio_command_downloads_no_video_and_selects_one_subtitle_source() {
        let tools = Tools {
            downloader: "yt-dlp.exe".into(),
            ffmpeg: "ffmpeg.exe".into(),
            runtime: "deno:deno.exe".into(),
        };
        let output = std::path::Path::new("中文 & 目录");
        for automatic in [false, true] {
            let choice = SubtitleChoice {
                language: "en.US".into(),
                automatic,
            };
            let command = media_download_command(
                &tools,
                output,
                DownloadMode::AudioSubtitles,
                None,
                Container::Mkv,
                Some(&choice),
            )
            .unwrap();
            let args: Vec<_> = command
                .get_args()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
            assert!(args.windows(2).any(|a| a == ["--format", "bestaudio"]));
            assert!(args.windows(2).any(|a| a == ["--audio-format", "mp3"]));
            assert!(args.windows(2).any(|a| a == ["--sub-langs", "^en\\.US$"]));
            assert!(args.iter().any(|a| a
                == if automatic {
                    "--write-auto-subs"
                } else {
                    "--write-subs"
                }));
            assert!(!args
                .iter()
                .any(|a| a == "--remux-video" || a == "--merge-output-format"));
            assert!(args.iter().any(|a| a == "--continue"));
            assert!(args.iter().any(|a| a.contains(if automatic {
                "].auto."
            } else {
                "].manual."
            })));
        }
        let bad = SubtitleChoice {
            language: "en,.*".into(),
            automatic: false,
        };
        assert!(media_download_command(
            &tools,
            output,
            DownloadMode::AudioSubtitles,
            None,
            Container::Mkv,
            Some(&bad)
        )
        .is_err());
        assert!(media_download_command(
            &tools,
            output,
            DownloadMode::Audio,
            Some(1080),
            Container::Mkv,
            None
        )
        .is_err());
        assert!(serde_json::from_str::<DownloadMode>("\"exe\"").is_err());
        assert_eq!(
            processing_label("[ExtractAudio] Destination: file.mp3"),
            Some("正在生成音频")
        );
        assert_eq!(processing_label("[download] 10%"), None);
    }
    #[test]
    fn all_five_modes_enforce_their_media_and_subtitle_contract() {
        let tools = Tools {
            downloader: "yt-dlp.exe".into(),
            ffmpeg: "ffmpeg.exe".into(),
            runtime: "deno:deno.exe".into(),
        };
        let output = std::path::Path::new("目录");
        let choice = SubtitleChoice {
            language: "en".into(),
            automatic: false,
        };
        for (name, needs_subtitle, video, audio) in [
            ("video", false, true, false),
            ("video_subtitles", true, true, false),
            ("audio", false, false, true),
            ("audio_subtitles", true, false, true),
            ("subtitles", true, false, false),
        ] {
            let mode = serde_json::from_value::<DownloadMode>(json!(name)).unwrap();
            let subtitle = needs_subtitle.then_some(&choice);
            let command =
                media_download_command(&tools, output, mode, None, Container::Mkv, subtitle)
                    .unwrap();
            let args: Vec<_> = command.get_args().collect();
            assert_eq!(args.iter().any(|a| *a == "--remux-video"), video);
            assert_eq!(args.iter().any(|a| *a == "--extract-audio"), audio);
            assert_eq!(
                args.iter().any(|a| *a == "--skip-download"),
                !video && !audio
            );
            assert_eq!(
                args.iter().any(|a| *a == "--ignore-no-formats-error"),
                !video && !audio
            );
            assert_eq!(args.iter().any(|a| *a == "--write-subs"), needs_subtitle);
            assert!(media_download_command(
                &tools,
                output,
                mode,
                None,
                Container::Mkv,
                if needs_subtitle { None } else { Some(&choice) }
            )
            .is_err());
        }
    }
    #[test]
    fn subtitle_result_uses_the_final_filename_and_rejects_a_missing_file() {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().join("带点.的标题 [id].webm");
        let output = format!(
            "YTGET_SUBTITLE:{{\"en.US\":{{\"ext\":\"vtt\"}}}}\nYTGET_BASE:{}\n",
            serde_json::to_string(&base).unwrap()
        );
        let choice = SubtitleChoice {
            language: "en.US".into(),
            automatic: true,
        };
        assert!(subtitle_file(&output, &choice).is_err());
        let expected = base.with_extension("auto.en.US.srt");
        fs::write(&expected, "1\n00:00:00,000 --> 00:00:01,000\n字幕\n").unwrap();
        assert_eq!(
            PathBuf::from(subtitle_file(&output, &choice).unwrap()),
            expected
        );
        let missing = format!(
            "YTGET_SUBTITLE:{{}}\nYTGET_BASE:{}\n",
            serde_json::to_string(&base).unwrap()
        );
        assert!(subtitle_file(&missing, &choice)
            .unwrap_err()
            .contains("没有所选字幕"));
    }
}
