造梦西游 4 / 5 桌面客户端，使用 Rust、egui 和内嵌 Ruffle。

## v0.1.2 更新

- 新增 Windows x86_64 便携程序和 macOS Apple Silicon 应用打包，保留 Linux AppImage；macOS 仅支持 Apple 芯片。
- 账号与设置改用跨平台 TOML 文件，记住的密码保存在独立的本地文件中，不再依赖 Linux 系统密钥环。
- 将静态资源请求放到后台工作线程，减少 UI 线程承担的下载和响应体处理；退出会话时取消未完成的后台等待。
- 优化 Ruffle 嵌套 JSON 转换，消除递归子树复制；复用渲染纹理视图，合并连续重复的调试日志。
- 增加 JSON、AMF 和 ByteArray 的慢阶段诊断，并按文件内容识别动态 SWF，便于排查游戏加载卡顿。
- 新增跨平台构建、打包检查和 SHA256 校验。此版本未加入补帧，也未修改游戏目标帧率。

## 下载

| 系统 | 文件 | 使用方式 |
| --- | --- | --- |
| Linux x86_64 | `ZM-LINUX-x86_64.AppImage` | 添加执行权限后启动 |
| Windows x86_64 | `ZM-LINUX-windows-x86_64.zip` | 解压后双击 `zm-linux.exe` |
| macOS Apple 芯片 | `ZM-LINUX-macos-arm64.dmg` | 将 `.app` 拖到 Applications |

每个文件附带 SHA256 校验文件。Windows 包未进行发行者代码签名；CI 生成的 macOS 包仅有 ad-hoc 签名，未经过 Apple 公证，系统可能阻止直接启动。这些包需要在对应系统上验证登录、画面、音频及游戏运行后再确认兼容性。

## 配置与升级

- Linux 默认配置在 `~/.config/zm/`，Windows 在 `%APPDATA%\zm\`，macOS 在 `~/Library/Application Support/zm/`。
- 首次启动会迁移旧账号列表和设置；原系统密钥环中的密码需重新输入一次。
- 勾选“记住密码”后，密码以明文保存在 `credentials.toml`；该文件不应公开。
- 游戏资源在启动时从官方地址获取，不包含在安装包中。

构建与签名说明见仓库 `docs/PACKAGING.md`。
