# Handy-ytget 桌面版

使用 Tauri 2 和 Rust 构建的 Windows 视频下载工具。参考 [Chlience/yt-dlp-tauri](https://github.com/Chlience/yt-dlp-tauri) 的操作流程和界面布局，界面与业务代码独立实现，下载内核复用本项目的 Rust 命令行版。

[下载最新 Windows 便携版](https://github.com/JunCxio/Handy-ytget/releases/latest) · [源码仓库](https://github.com/JunCxio/Handy-ytget)

## 使用

1. 解压 Windows 便携包，双击 `windows/Handy-ytget.exe`。
2. 粘贴 YouTube 视频链接，点击“解析视频”，或直接按回车。
3. 查看封面、标题、发布者、时长及可选画质。
4. 选择“视频”“视频＋字幕”“音频”“音频＋字幕”或“字幕”。视频可选画质及 MKV / MP4 封装，音频保存为 MP3，字幕保存为独立 SRT。
5. 含字幕模式可选择语言，列表区分人工字幕和自动字幕；解析后没有字幕会提示，并禁用含字幕模式和字幕下载。
6. 点击“选择目录”指定保存文件夹，再点击“开始下载”，完成后点击“打开下载目录”。视频和音频下载期间可点击“暂停下载”，再点击“继续下载”。

Windows 10/11 x64 需要 Microsoft Edge WebView2 运行时。大多数当前 Windows 系统已自带；如果系统缺失，可从 [微软官方网站](https://developer.microsoft.com/microsoft-edge/webview2/) 安装。便携包包含 yt-dlp、FFmpeg、FFprobe 和 Deno，无需安装 Rust、Node.js 或 Python。

## 组件安装与更新

- 启动时统一检查并一次补齐全部必要组件，不区分解析和下载阶段。界面显示当前组件及进度，全部就绪后即可使用；失败后点击“重试安装”。
- 首次安装使用随程序核对过的固定发行版本。更新时进入右上角设置，点击“检查更新”，查看当前版本与最新版本，再点击“更新组件”。
- 来源固定为 yt-dlp、Deno 官方 GitHub 发布页及 Gyan 的 FFmpeg 构建。通过 HTTPS 下载并校验 SHA256，校验通过后才安装；不会执行未校验的下载文件。
- 仅下载缺失或有更新的组件，不主动降级已安装的较新版本。已有系统 Node.js 且没有 Deno 时会继续使用 Node.js，不修改系统 Node.js。
- 下载或校验失败时保留该组件已有文件，可继续使用已就绪的旧组件。有任务运行或暂停时不能安装和更新组件，防止替换任务使用的文件。
- 自动安装和更新的组件保存在 `%LOCALAPPDATA%\local.ytget.desktop\tools`，优先使用该目录，其次使用便携包和 PATH 中的组件。不要求程序目录可写。
- 当前更新入口更新的是下载组件。程序本身的更新仍通过替换便携包完成。

组件下载需要网络能够访问上述来源；断网时会显示失败原因，不会反复在后台尝试。

## 行为

- 支持普通视频、短链接、Shorts、音乐版页面和已结束的直播；每次运行一个任务。
- 默认“视频＋字幕”、最佳画质、MKV 封装，保存到程序旁的 `downloads`。字幕优先选择人工中文字幕，否则选择列表首项；没有可用字幕时需切换为视频或音频才能下载。
- 画质选项设定当前视频可用分辨率的上限。MKV / MP4 只改变封装，不进行重新编码；播放器需要支持原始音视频编码。
- 视频和音频可能分别下载，进度显示**当前音视频流**的百分比、速度和剩余时间。只有合并并保存完成后，才显示“下载完成”。
- 可取消解析和下载。在 Windows 上取消会结束下载器及其子进程；关闭窗口也会取消当前任务。
- 视频和音频下载支持暂停：停止当前下载进程并保留临时文件，继续时使用同一请求尝试断点续传；字幕单独下载不提供暂停。
- 合并、封装和音频／字幕转换期间不能暂停。暂停状态不跨程序重启保存；重启后用相同链接、目录和参数重新下载，可尝试复用未完成的数据。
- 字幕只提取平台已有的人工或自动字幕，不提供语音识别，也不能直接提取画面内压制字幕。字幕模式没有可用字幕时不能下载；下载时所选字幕已失效会报错，不会生成空字幕并显示成功。
- 字幕模式不下载媒体；音频模式不下载视频。含字幕模式的临时文件保存在输出目录下 `.ytget/manual` 或 `.ytget/auto`，最终文件保存在所选目录。
- 取消后保留临时文件，再次下载相同视频并选择相同目录、画质和格式，可尝试原生断点续传。
- 已有的同名目标文件不会被覆盖。标题变化、文件移动和参数变化可能生成新的文件。
- 更改输入链接后，必须重新解析，避免使用上一个视频的信息下载。
- 错误原因直接显示在界面上；运行日志最多显示最近 250 行，不在后台持续写日志。
- 标题、说明和错误均作为纯文本显示；链接校验在 Rust 入口再次执行，不经 Shell 拼接执行命令。

## 保存位置与状态

首次默认保存位置是可执行文件旁的 `downloads`。通过系统文件夹对话框选择的新目录会保存到：

```text
%APPDATA%\local.ytget.desktop\output-directory.txt
```

软件改名后仍使用原应用标识和上述配置目录，沿用已有保存位置和组件缓存。

WebView2 会管理自己的用户数据目录。当前任务和视频预览保存在内存中，关闭程序后不保留任务历史。目标视频、`.part` 和下载状态文件由 yt-dlp 管理。

长目录会省略显示，将鼠标移到目录文字上可查看完整路径；目录选择按钮始终保留。窄窗口中，保存位置显示为独立一行。

## 源码结构

```text
Handy-ytget/
  ui/                         HTML、CSS、JavaScript 界面，无前端构建依赖
  src-tauri/src/main.rs        窗口入口和命令注册
  src-tauri/src/backend.rs     预览、进度、进程取消、目录设置和单元测试
  src-tauri/src/components.rs  组件下载、校验、安装及更新检查
  src-tauri/tauri.conf.json    窗口、内容安全策略和权限配置
youtube-downloader/
  src/lib.rs                  共享链接校验、依赖查找和下载命令
  src/main.rs                 原命令行入口
```

源码包包含这两个相邻目录，请保持目录关系。共享库重构后，命令行版仍可单独构建和使用。

在已配置 Tauri Windows 开发环境的终端中执行：

```powershell
cargo test --manifest-path Handy-ytget/src-tauri/Cargo.toml
cargo clippy --manifest-path Handy-ytget/src-tauri/Cargo.toml --all-targets -- -D warnings
cargo build --release --manifest-path Handy-ytget/src-tauri/Cargo.toml --features tauri/custom-protocol
cargo test --manifest-path youtube-downloader/Cargo.toml
```

发布构建须启用 `tauri/custom-protocol`，以加载嵌入的本地界面。Rust 构建所需环境见 [Tauri 官方文档](https://v2.tauri.app/start/prerequisites/)。可随包提供 `tools`，也可让程序自动安装缺失组件。GNU 构建另需随附 `WebView2Loader.dll`；常规 MSVC 构建使用静态加载器。

## 范围与许可

当前版本面向无需登录即可访问的单个 YouTube 视频，不提供账户登录、Cookie 导入、批量任务、播放列表下载或后台服务。无法访问的网络、私有视频、地区限制等会通过原始错误说明。

程序源码为 MIT 许可；没有复制参考项目的源码或素材。参考项目为 GPL-3.0。随附下载组件按各自许可分发，见 `windows/THIRD_PARTY_NOTICES.md`。实际验证记录见 `VALIDATION.md`。
