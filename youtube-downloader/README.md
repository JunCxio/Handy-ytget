# ytget

输入一个 YouTube 视频链接，即可下载视频。程序使用 Rust 标准库编写，通过独立进程调用 yt-dlp 解析和下载，使用 FFmpeg 合并音视频，使用 Deno 执行 YouTube 的 JavaScript 挑战。

## Windows 直接使用

1. 解压 Windows 便携包，保留 `ytget.exe` 和 `tools` 文件夹的位置关系。
2. 双击 `ytget.exe`。
3. 粘贴视频链接，按回车开始下载。
4. 下载完成后，在程序旁的 `downloads` 文件夹中查看视频。

便携包适用于 Windows 10/11 x64，已包含运行依赖，不需要安装 Rust、Python 或单独配置环境变量。程序需要能够连接 YouTube 及其视频服务器。

```text
ytget.exe
tools/
  yt-dlp.exe
  ffmpeg.exe
  ffprobe.exe
  deno.exe
downloads/             首次下载时自动创建
```

## 命令行使用

在 PowerShell 中运行：

```powershell
.\ytget.exe "https://www.youtube.com/watch?v=视频ID"
.\ytget.exe "https://youtu.be/视频ID" -o "D:\我的视频"
.\ytget.exe --help
.\ytget.exe --version
```

`视频ID` 为示意文字，请替换为实际的视频链接。链接包含 `&` 时必须加引号；使用双击粘贴模式时可以直接粘贴完整链接。

支持 `youtube.com/watch?v=...`、`youtu.be/...`、`youtube.com/shorts/...`、`youtube.com/live/...`、`youtube.com/embed/...`，以及移动版和音乐版域名。支持带播放列表参数的视频链接，每次只下载其中的单个视频。频道和纯播放列表链接会被拒绝。

## 默认行为

- 自动选择可获取的最佳画质和音质，不设分辨率上限，文件可能较大。
- 保存为 MKV；只合并和重新封装，不进行画质有损的重新编码。可使用 VLC 等支持对应编码的播放器播放。
- 文件名为 `视频标题 [视频ID].mkv`，标题按字节限制长度，并处理 Windows 文件名中的非法字符。
- 默认目录为**可执行文件所在目录**下的 `downloads`；`-o` 指定的相对路径相对于当前工作目录。
- 下载进度、速度、剩余时间和合并状态由 yt-dlp 实时输出。
- 网络失败时进行有限次数重试；保留 `.part` 和下载状态文件，用相同链接和目录重新运行即可尝试续传。服务器是否支持续传由远端决定。
- 已存在的目标文件不会被覆盖。移动、改名或修改视频标题后，不能保证识别为相同的已有文件。
- 原样显示 yt-dlp 的具体错误，失败时返回非零退出码；双击模式会等待回车后关闭窗口。
- 只接受合法的 YouTube 视频域名和 11 位视频 ID，丢弃分享跟踪参数和播放列表参数。
- 使用进程参数直接调用依赖，不将链接交给 Shell 执行，不加载外部 yt-dlp 配置和插件，不读取浏览器 Cookie。

## 从 Rust 源码构建

需要 Rust 1.74 或更新版本及对应平台的链接工具。Rust 项目没有第三方 crate 依赖。

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

将 `target/release/ytget.exe` 和 `tools` 目录放在一起即可。也可以通过 PATH 提供依赖，查找顺序为：程序旁的 `tools` 目录、程序目录、PATH。JavaScript 运行时优先使用 Deno，没有 Deno 时使用 Node.js；其版本须满足当前 yt-dlp 的要求。

macOS/Linux 可使用相同 Rust 源码构建，并通过 PATH 安装 yt-dlp、FFmpeg 和 Deno。当前交付仅验证 Windows x64。

## 更新解析器

YouTube 的解析规则会变化。出现解析或签名错误时，退出本工具，在便携包目录运行：

```powershell
.\tools\yt-dlp.exe -U
```

该命令使用 yt-dlp 官方更新机制。也可以从 [yt-dlp 官方发布页](https://github.com/yt-dlp/yt-dlp/releases) 获取新版本替换 `tools/yt-dlp.exe`。无需重新编译 Rust 工具。更新后，随包提供的初始校验记录不再对应已更新的文件。

## 故障处理

| 提示或现象 | 处理方式 |
| --- | --- |
| 无法连接、超时或 DNS 错误 | 检查本机网络能否正常访问 YouTube；已有的系统进程代理环境变量会传递给 yt-dlp |
| 要求登录或确认身份、地区限制、私有视频 | 当前版本面向无需登录即可访问的视频，不提供账户登录和浏览器 Cookie 导入 |
| JavaScript 或签名解析错误 | 优先更新 yt-dlp，再检查 Deno 版本 |
| 合并失败、磁盘空间不足 | 保留已下载的临时文件，修复错误后以相同链接和目录重新运行 |
| 找不到依赖 | 检查 `tools` 是否和程序一起解压，是否有文件被移走 |
| 无法写入目录 | 用 `-o` 指定有写入权限且磁盘空间足够的目录 |

当前范围为单视频下载，不包含图形界面、批量任务、账户登录、音频单独导出或转码设置。持续直播会由 yt-dlp 按直播处理；如需固定的视频文件，请使用已结束的直播链接。

## 项目结构与许可

`src/main.rs` 包含命令行入口、参数解析和下载执行；`src/lib.rs` 包含与桌面版共享的链接校验、依赖定位、下载命令及单元测试。持久化由 yt-dlp 负责，包含目标视频、断点文件和合并过程中的临时文件。程序自身没有数据库、账户状态或后台服务。

Rust 源码采用 MIT 许可，见 `LICENSE`。便携包中的第三方程序分别遵循各自的许可，见 `windows/THIRD_PARTY_NOTICES.md` 和 `windows/licenses/`。

实际验证结果见 `VALIDATION.md`。
