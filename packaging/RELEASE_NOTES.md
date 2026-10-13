造梦西游 4 / 5 桌面客户端，使用 Rust、egui 和内嵌 Ruffle。

项目现名 **ZM-Container**（原 ZM-LINUX）。以下下载名称适用于改名后的新构建，历史版本附件保持原文件名。

## v0.1.3 更新

- 优化造梦西游5的怪物出生点查询，减少死亡与刷怪条件检查中的重复遍历。
- 修复隐藏调试日志框逐条重排历史文本的开销；展开日志时恢复完整历史，保持原始记录和错误处理。
- 优化时间轴回退时的对象层级处理，减少大量对象替换时的重复扫描；ByteArray 压缩、解压直接采用完成后的缓冲区，减少额外复制。
- 修复打开诊断窗口后游戏键盘输入被屏蔽的问题，并补充慢回调调用栈、原生调用和累计文本排版诊断。
- 修复充值跳转丢失当前账号的问题，造四、造五均打开对应的官方充值页并预填当前游戏账号。
- 新版发布统一使用 ZM-Container 名称，提供 Linux AppImage、Windows 便携 EXE 和 macOS Apple 芯片应用。

性能优化针对已经复现的具体开销，藏宝阁实战中的全部卡顿尚未完成验证。测量、回归和修复范围见仓库 `docs/PERFORMANCE-2026-09-30.md`。

升级后账号、设置与游戏存档保持兼容。资源补丁版本升至 7，首次启动会自动重新准备游戏资源，无需手动清空缓存。

## 下载

| 系统 | 文件 | 使用方式 |
| --- | --- | --- |
| Linux x86_64 | `ZM-Container-x86_64.AppImage` | 添加执行权限后启动 |
| Windows x86_64 | `ZM-Container-windows-x86_64.zip` | 解压后双击 `zm-container.exe` |
| macOS Apple 芯片 | `ZM-Container-macos-arm64.dmg` | 将 `.app` 拖到 Applications |

每个文件附带 SHA256 校验文件。Windows 包未进行发行者代码签名；CI 生成的 macOS 包仅有 ad-hoc 签名，未经过 Apple 公证，系统可能阻止直接启动。这些包需要在对应系统上验证登录、画面、音频及游戏运行后再确认兼容性。

## 配置与升级

- Linux 默认配置在 `~/.config/zm/`，Windows 在 `%APPDATA%\zm\`，macOS 在 `~/Library/Application Support/zm/`。
- 首次启动会迁移旧账号列表和设置；原系统密钥环中的密码需重新输入一次。
- 勾选“记住密码”后，密码以明文保存在 `credentials.toml`；该文件不应公开。
- 游戏资源在启动时从官方地址获取，不包含在安装包中。

构建与签名说明见仓库 `docs/PACKAGING.md`。
