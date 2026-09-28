# 桌面应用打包

当前采用各平台原生构建，共用 Rust 代码和 TOML 配置格式。macOS 仅支持 Apple 芯片，不构建 Intel 或 Universal 包。

| 平台 | 构建目标 | 发布产物 |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `ZM-LINUX-x86_64.AppImage` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `ZM-LINUX-windows-x86_64.zip`，内含 `zm-linux.exe` |
| macOS Apple 芯片 | `aarch64-apple-darwin` | `ZM-LINUX-macos-arm64.dmg`，内含 `ZM-LINUX.app` |

所有发布包附带 `.sha256`。Windows 先采用解压即用的程序，后续有安装目录、快捷方式和卸载器需求时再增加安装程序。macOS 使用标准 `.app` 包装，再放入 DMG 供用户拖入 Applications。

## GitHub Actions

`.github/workflows/ci.yml` 在 Ubuntu 24.04、Windows Server 2022 和 macOS 15 Apple Silicon 上分别检查、测试和构建。固定 runner 标签和 Rust 版本，避免 `latest` 改变架构。工作区版本仍以 `Cargo.toml` 为准。

- 推送分支或创建 PR：验证三个平台，并上传 Windows、macOS 打包产物。
- Actions → CI → Run workflow：手动构建三个平台的下载产物，不创建公开 Release。
- 推送与工作区版本一致的 `v` 标签：三个平台全部通过后，统一校验并上传 GitHub Release，避免多个平台抢先创建同一个 Release。

首次跨平台发布前，先通过手动运行取得测试包，在 Windows 和 Apple 芯片 Mac 上验证登录、中文字体、音频、全屏、账号保存、两款游戏加载和退出。CI 编译通过不代表游戏兼容性全部通过。本地 Linux 环境无法验证 Windows/macOS 的原生 SDK、签名工具和图形运行。

发布标签前先更新工作区版本和 `Cargo.lock`，修改 `packaging/RELEASE_NOTES.md`，提交并推送代码，再创建对应 `v` 标签。标签版本不匹配时 CI 会停止发布。

## Windows 本地打包

需要 Rust 1.95+ 的 MSVC 工具链、Visual Studio C++ Build Tools（含 Windows SDK）和 Python 3.11+。在仓库根目录执行：

```powershell
rustup target add x86_64-pc-windows-msvc
python packaging/build.py --platform windows
```

脚本生成图标与版本资源，启用 `crt-static` 构建，并检查 PE 架构、GUI 子系统和 VC 运行库依赖。Release 程序双击时不弹出控制台窗口；开发构建保留控制台。压缩包包含程序和许可说明，不携带账号文件、缓存或游戏资源。

脚本将构建放在 `target/x86_64-pc-windows-msvc/release/`，压缩包放在 `dist/`。只有复用本脚本在同一目标上构建的程序时，才使用 `--skip-build`。

默认不进行 Windows 发行者签名。正式分发可使用 Windows SDK 的 SignTool 或受信任的签名服务，对 EXE 签名后重新打包并生成校验。代码签名与 SmartScreen 声誉是不同机制，新签名程序仍可能出现提示；签名步骤需要在最终压缩和计算 SHA256 之前完成。

## macOS Apple 芯片本地打包

在 Apple 芯片 Mac 的原生终端中执行，需要 Xcode Command Line Tools、Rust 1.95+ 和 Python 3.11+：

```bash
xcode-select --install
rustup target add aarch64-apple-darwin
python3 packaging/build.py --platform macos
```

脚本默认设置 `MACOSX_DEPLOYMENT_TARGET=13.0`；这是构建最低系统目标，实际最低版本兼容性需要对应机器验证。程序使用 wgpu 的 macOS 后端，不需要安装 Wine。打包脚本检查 Mach-O 架构和动态库依赖；若发现 Homebrew 或其他非系统动态库，会停止打包，避免生成只在构建机能运行的应用。

`.app` 包含 `Contents/Info.plist`、`Contents/MacOS/zm-linux` 和图标、许可文件。DMG 生成在 `dist/`，并提供 Applications 入口。配置目录为 `~/Library/Application Support/zm/`，游戏数据和日志仍位于 `dirs` 对应的原 `zm-linux` 数据目录。

### 签名与公证

默认使用 ad-hoc 签名，适合本机和开发测试，不代表受信任的开发者签名。GitHub Actions 默认也采用这一模式。下载的测试包可能被 Gatekeeper 阻止；发行给普通用户时应使用 Developer ID Application 证书签名并通过 Apple 公证。

如果构建机已经安装证书，并使用 `notarytool store-credentials` 配置了钥匙串凭据，可以运行：

```bash
MACOS_SIGNING_IDENTITY='Developer ID Application: Your Name (TEAMID)' \
MACOS_NOTARY_PROFILE='zm-notary' \
python3 packaging/build.py --platform macos
```

脚本会启用 Hardened Runtime，签名 APP 和 DMG，提交最终 DMG 等待公证通过，附加并验证公证票据，最后计算 SHA256。公证失败会停止，不会输出成功状态。没有证书时无需填写这两个环境变量。

CI 尚未配置证书导入或公证凭据。需要正式签名发布时，先把证书和公证凭据放入仓库的受保护 Secrets，再增加仅对受信任发布任务执行的临时钥匙串导入步骤；不要把私钥或密码写入仓库，也不要向外部 PR 暴露它们。

## 图标与本地检查

打包使用提交到仓库的 `assets/zm.ico` 和 `assets/zm.icns`，日常构建无需 Pillow。更新原始 PNG 后，可安装 Pillow 并运行 `python3 packaging/generate-icons.py` 重新生成两种格式。

```bash
python3 packaging/test_packaging.py
python3 -m py_compile packaging/build.py packaging/generate-icons.py
cargo fmt --all -- --check
```

离线测试覆盖包内容、校验文件、错误架构和运行库检测、macOS 包结构及签名顺序。真实 SDK、代码签名和 DMG 检查由相应系统上的打包脚本执行。

## 官方参考

- [GitHub 托管 runner 与架构](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
- [Rust 的静态 C 运行库链接](https://doc.rust-lang.org/reference/linkage.html#static-and-dynamic-c-runtimes)
- [Apple 自定义公证流程](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)
- [Microsoft SmartScreen 与代码签名](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)
