# Handy-ytget

使用 Rust 和 Tauri 2 构建的 YouTube 桌面下载工具。粘贴链接，解析后选择下载内容和保存目录。

[下载 Windows 便携版](https://github.com/JunCxio/Handy-ytget/releases/latest)

## 功能

- 支持视频、视频＋字幕、音频、音频＋字幕、字幕五种模式，默认“视频＋字幕”。
- 视频可选择画质及 MKV / MP4 封装；音频保存为 MP3；字幕保存为独立 SRT。
- 可选择字幕语言，区分人工和自动字幕；没有可用字幕时提示并禁止字幕下载。
- 视频和音频均支持暂停、继续与取消。
- 可选择并记住保存目录；启动时检查必要组件，缺失时自动下载和校验，设置中可检查组件更新。

## 使用

1. 从 Releases 下载 `Handy-ytget-windows-x64-v0.3.1.zip` 并完整解压。
2. 双击 `Handy-ytget/windows/Handy-ytget.exe`。
3. 粘贴 YouTube 视频链接，点击“解析视频”，选择内容和保存目录后开始下载。

便携包包含 yt-dlp、FFmpeg、FFprobe 和 Deno，无需安装 Rust、Node.js 或 Python。运行需要 Windows 10/11 x64 和 Microsoft Edge WebView2；缺失 WebView2 时可从[微软官网](https://developer.microsoft.com/microsoft-edge/webview2/)安装。

字幕只提取平台已有的人工或自动字幕，不做语音识别。合并、封装和转换阶段不能暂停；续传取决于源站与 yt-dlp。当前支持无需登录即可访问的单个视频。

## 源码与构建

仓库保留两个相邻工程：

```text
Handy-ytget/          桌面界面与 Rust 后端
youtube-downloader/  共享下载命令库与命令行入口
```

在已安装 Rust 并配置 [Tauri 开发环境](https://v2.tauri.app/start/prerequisites/)的终端中，从仓库根目录执行：

```powershell
cargo test --manifest-path Handy-ytget/src-tauri/Cargo.toml
cargo test --manifest-path youtube-downloader/Cargo.toml
cargo build --release --manifest-path Handy-ytget/src-tauri/Cargo.toml --features tauri/custom-protocol
```

编译后运行 `Handy-ytget/src-tauri/target/release/handy-ytget.exe`。发布包中的文件名为 `Handy-ytget.exe`。GNU 构建另需提供 `WebView2Loader.dll`；常规 MSVC 构建使用静态加载器。

详细说明见[桌面版文档](Handy-ytget/README.md)，验证范围见[验证记录](Handy-ytget/VALIDATION.md)。

## 许可

本项目源码采用 [MIT 许可](LICENSE)。外部下载组件按各自许可分发，相关说明和许可文本随 Windows 便携包提供。界面操作流程参考 [yt-dlp-tauri](https://github.com/Chlience/yt-dlp-tauri)，界面和业务代码独立实现。
