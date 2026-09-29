# ZM-Container

使用 Rust 编写的造梦西游 4 / 5 桌面客户端，采用 egui + 内嵌 Ruffle。游戏资源在启动时从官方地址获取，不随程序分发。

项目原名 ZM-LINUX，现名 **ZM-Container**，仓库地址为 [gcd-fj/ZM-Container](https://github.com/gcd-fj/ZM-Container)。新构建的可执行程序名为 `zm-container`，Windows 为 `zm-container.exe`；已发布历史版本的附件仍保留原文件名。

项目参考 [zmBox](https://gitee.com/duskeye/zmBox) 的游戏宿主交互流程，重新设计 Rust 应用结构。目标平台为 Linux x86_64、Windows x86_64 和 macOS Apple 芯片；跨平台构建由 CI 验证，实际游戏兼容性仍需逐项测试。

## 重构后的结构

| 模块 | 职责 |
| --- | --- |
| zm-app | 游戏库首页、账号界面、窗口与事件接入 |
| zm-launcher | 可取消的启动工作流、状态机、会话隔离 |
| zm-core | 公共模型、错误、两款游戏的配置 |
| zm-storage | TOML 配置、本地密码文件、内存凭据、平台目录 |
| zm-auth | 4399 登录、验证码与令牌协议 |
| zm-assets | 版本发现、下载、完整性校验、缓存发布、SWF 桥接补丁 |
| zm-player | Ruffle 宿主、共享 GPU 渲染、输入、音频与诊断 |

完整设计、数据流及边界见 [架构说明](docs/ARCHITECTURE.md)。Ruffle 固定于 `a4f5b5256e245693bc9077ef6c6b6abc95490e7f`，与 egui 使用匹配的 wgpu 版本。

## 构建运行

需要 Rust 1.95 或更高版本。Ubuntu 开发依赖：

```bash
sudo apt install build-essential pkg-config libasound2-dev libudev-dev libfontconfig-dev fonts-noto-cjk
cargo run --locked --bin zm-container
```

调试版适合复现功能问题，性能评估请使用优化版：

```bash
cargo build --release --locked --bin zm-container
./target/release/zm-container
```

分段耗时、空闲 CPU/RSS 采样及两款游戏的性能验收步骤见 [性能采样与验收](docs/PERFORMANCE.md)。诊断中的 `tick_hz` 表示宿主调用播放器的频率，不等同于实际游戏呈现帧率。

Windows 使用 MSVC Rust 工具链及 Visual Studio C++ 构建工具，执行同样的 Cargo 命令，程序为 `target/release/zm-container.exe`。

macOS 使用 Apple 芯片 Mac、Xcode Command Line Tools 和原生 Rust 工具链。Windows 便携 EXE 压缩包、macOS APP/DMG 的生成命令与签名说明见 [跨平台打包](docs/PACKAGING.md)。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## 使用

1. 在游戏库中选择造四或造五，填写或选择 4399 账号。
2. 登录需要验证码时填写图片内容；图片失败可单独刷新。准备阶段可取消启动。
3. 游戏工具栏支持音量、诊断、账号切换、全屏及退出。F11 切换全屏，Esc 退出全屏。
4. 设置中可清理资源缓存、查看上次游戏诊断。Linux 另提供桌面入口安装和卸载。

游戏加载期间，窗口底部会显示待完成的资源请求数及本次累计接收的数据量。它反映下载活动，不代表游戏整体加载百分比；游戏原有的进度条仍由游戏控制。

“宿主就绪”与“会话已注入”是不同阶段。会话注入成功不代表全部游戏功能已验证，游戏后续仍可能加载资源或遇到播放器兼容问题。游戏启动连续 90 秒没有资源完成或初始化进展才会停止播放器并保留诊断；不按总下载时间判定超时。

## 数据与缓存

Linux 配置默认位于 `~/.config/zm/`；设置了有效的 `XDG_CONFIG_HOME` 时使用 `$XDG_CONFIG_HOME/zm/`。Windows 配置位于 `%APPDATA%\zm\`，macOS 位于 `~/Library/Application Support/zm/`。各平台使用相同的文件格式，不依赖系统密钥环。

| 文件 | 内容 |
| --- | --- |
| `config.toml` | 账号列表、账号标识、记住密码偏好、上次选择和音量 |
| `credentials.toml` | 勾选“记住密码”的账号及明文密码 |
| `credentials.lock` | 多进程读写密码文件时使用的锁，无密码内容 |

账号数量少，沿用现有 TOML 格式即可，无需额外数据库。密码文件不是加密保险箱：Linux 目录权限为 `0700`、文件为 `0600`；Windows 使用用户配置目录的访问权限。取消记住密码会删除对应的本地密码记录，并保留本次运行已读取的密码；删除账号同时删除对应凭据。Cookie 和 token 仅用于当前会话，不写入这些文件。

首次启动新版时，若新配置不存在，会自动迁移旧 `zm-linux/config.toml` 中的账号列表和设置，并保留旧文件。旧系统密钥环的密码需要重新输入一次，成功登录后按“记住密码”选项保存到新文件；新版不访问或清除旧密钥环。关闭客户端后，复制 `config.toml` 和 `credentials.toml` 到另一平台的配置目录即可迁移已保存账号，请勿公开密码文件。

文件采用同目录临时文件原子替换；密码文件读写加进程间锁，后台执行，正常退出前等待队列完成。配置或密码文件损坏时会报错并保留原内容，需备份修复后重试；密码解析错误不显示源文件内容。

缓存、游戏存档和日志继续使用原目录：Linux 缓存默认在 `~/.cache/zm-linux`，数据与日志在 `~/.local/share/zm-linux`；Windows 使用原系统缓存及本地数据目录。

改名保留应用标识 `io.github.gcd-fj.zm-linux`、原数据目录和游戏桥接标识，确保桌面入口更新、账号、缓存与存档继续兼容。历史性能记录中的旧程序名表示当时实际测量的产物。

主 SWF 按内容哈希发布，清单最后切换。更新失败时可以使用校验通过且匹配当前桥接版本的旧缓存；补丁更新会触发重新下载。运行时资源按版本隔离、合并相同请求。清理资源不会删除新架构数据目录中的游戏 SharedObject，后者按游戏和 UID 分开。

## 桥接开发

仓库包含桥接源文件与编译后的 ABC。修改源文件后必须重新生成 ABC：

```bash
RUFFLE_ASC_JAR=/path/to/pinned-ruffle/tools/asc/asc.jar \
RUFFLE_PLAYERGLOBAL=/path/to/target/debug/build/ruffle_core-.../out/playerglobal_import.abc \
bash tools/build-bridges.sh
```

需要 Java，Ruffle 的 playerglobal 文件由正常 Cargo 构建产生。不要混用另一版本的编译输入。

## AppImage

安装 `linuxdeploy` 后执行 `./packaging/appimage/build.sh`。产物位于 `dist/`，包含程序及校验文件，不捆绑游戏资源。

推送与工作区版本一致的 `v` 标签后，CI 会在 Linux / Windows / macOS Apple Silicon 检查通过后，将 AppImage、Windows EXE 压缩包、macOS DMG 和 SHA256 校验文件统一发布到 [GitHub Releases](https://github.com/gcd-fj/ZM-Container/releases)。也可在 Actions 中手动运行 CI，仅生成测试下载产物。未配置发行签名时 Windows、macOS 安装包可能出现系统信任提示，详情见 [打包说明](docs/PACKAGING.md)。

## 许可

项目使用 MIT，第三方来源见 [第三方说明](THIRD_PARTY_LICENSES.md)。游戏程序、资源、商标及服务属于各自权利人。
