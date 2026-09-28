#!/usr/bin/env python3
"""Build native Windows/macOS release packages. Python 3.11+, no pip dependencies."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import struct
import subprocess
import sys
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
APP_NAME = "ZM-LINUX"
APP_ID = "io.github.gcd-fj.zm-linux"
WINDOWS_TARGET = "x86_64-pc-windows-msvc"
MAC_TARGET = "aarch64-apple-darwin"


def run(*args: str | Path, env: dict[str, str] | None = None, capture: bool = False) -> str:
    result = subprocess.run(
        [str(arg) for arg in args], cwd=ROOT, env=env, check=True,
        stdout=subprocess.PIPE if capture else None,
        text=True, encoding="utf-8", errors="replace",
    )
    return result.stdout or ""


def version() -> str:
    with (ROOT / "Cargo.toml").open("rb") as file:
        return tomllib.load(file)["workspace"]["package"]["version"]


def checksum(path: Path) -> None:
    with path.open("rb") as file:
        digest = hashlib.file_digest(file, "sha256").hexdigest()
    path.with_name(path.name + ".sha256").write_text(
        f"{digest}  {path.name}\n", encoding="ascii", newline="\n"
    )


def copy_notices(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    for source, name in [
        (ROOT / "LICENSE", "LICENSE"),
        (ROOT / "THIRD_PARTY_LICENSES.md", "THIRD_PARTY_LICENSES.md"),
        (ROOT / "vendor/ruffle/LICENSE.md", "RUFFLE-LICENSE.md"),
        (ROOT / "packaging/README.txt", "README.txt"),
    ]:
        shutil.copy2(source, directory / name)


def sdk_tool(name: str) -> Path:
    if found := shutil.which(name):
        return Path(found)
    kits = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Windows Kits/10/bin"
    candidates = list(kits.glob(f"10.*/x64/{name}"))
    if candidates:
        return max(candidates, key=lambda p: tuple(map(int, p.parent.parent.name.split("."))))
    raise RuntimeError(f"{name} not found; install the Windows SDK with Visual Studio C++ Build Tools")


def dumpbin_tool() -> Path:
    if found := shutil.which("dumpbin.exe"):
        return Path(found)
    vswhere = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Microsoft Visual Studio/Installer/vswhere.exe"
    installation = run(vswhere, "-latest", "-products", "*", "-requires",
                       "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                       "-property", "installationPath", capture=True).strip()
    if not installation:
        raise RuntimeError("Visual Studio C++ Build Tools not found")
    candidates = list((Path(installation) / "VC/Tools/MSVC").glob("*/bin/Hostx64/x64/dumpbin.exe"))
    if not candidates:
        raise RuntimeError("dumpbin.exe not found in Visual Studio")
    return max(candidates, key=lambda p: tuple(map(int, p.parents[3].name.split("."))))


def windows_resource(directory: Path) -> Path:
    app_version = version()
    numeric = app_version.split("-", 1)[0].split("+", 1)[0]
    if not re.fullmatch(r"\d+\.\d+\.\d+", numeric):
        raise RuntimeError("Windows packaging requires a major.minor.patch version")
    icon = (ROOT / "assets/zm.ico").as_posix()
    resource = directory / "app.rc"
    resource.write_text(f'''#pragma code_page(65001)
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numeric.replace('.', ',')},0
PRODUCTVERSION {numeric.replace('.', ',')},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "ZM-LINUX Desktop Client\\0"
      VALUE "FileVersion", "{app_version}\\0"
      VALUE "ProductName", "ZM-LINUX\\0"
      VALUE "ProductVersion", "{app_version}\\0"
      VALUE "OriginalFilename", "zm-linux.exe\\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
''', encoding="utf-8")
    output = directory / "app.res"
    run(sdk_tool("rc.exe"), "/nologo", f"/fo{output}", resource)
    return output


def verify_windows_executable(path: Path) -> None:
    with path.open("rb") as file:
        header = file.read(64)
        if len(header) != 64 or header[:2] != b"MZ":
            raise RuntimeError("Not a Windows executable")
        file.seek(struct.unpack_from("<I", header, 60)[0])
        pe = file.read(96)
    if len(pe) < 96 or pe[:4] != b"PE\0\0" or struct.unpack_from("<H", pe, 4)[0] != 0x8664:
        raise RuntimeError("Expected a Windows x86_64 PE executable")
    if struct.unpack_from("<H", pe, 24 + 68)[0] != 2:
        raise RuntimeError("Expected a GUI executable without a console window")


def build_windows(output: Path, skip_build: bool) -> None:
    work = ROOT / "target/packaging/windows-x86_64"
    work.mkdir(parents=True, exist_ok=True)
    binary = ROOT / "target" / WINDOWS_TARGET / "release/zm-linux.exe"
    if not skip_build:
        env = os.environ.copy()
        if env.get("CARGO_ENCODED_RUSTFLAGS"):
            raise RuntimeError("Unset CARGO_ENCODED_RUSTFLAGS so the static CRT flags can be applied")
        if "+crt-static" not in env.get("RUSTFLAGS", ""):
            env["RUSTFLAGS"] = env.get("RUSTFLAGS", "") + " -C target-feature=+crt-static"
        env["ZM_WINDOWS_RESOURCE"] = str(windows_resource(work))
        run("cargo", "build", "--release", "--locked", "--bin", "zm-linux",
            "--target", WINDOWS_TARGET, "--target-dir", ROOT / "target", env=env)
    verify_windows_executable(binary)
    dependencies = run(dumpbin_tool(), "/DEPENDENTS", binary, capture=True)
    if re.search(r"(?:VCRUNTIME\w*|MSVCP\w*|UCRTBASE)\.dll", dependencies, re.IGNORECASE):
        raise RuntimeError("Executable still depends on the VC runtime; rebuild with static CRT")
    print(dependencies)
    package = work / APP_NAME
    if package.exists():
        shutil.rmtree(package)
    copy_notices(package)
    shutil.copy2(binary, package / "zm-linux.exe")
    archive = output / "ZM-LINUX-windows-x86_64.zip"
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as file:
        for path in sorted(package.rglob("*")):
            if path.is_file():
                file.write(path, path.relative_to(work))
    checksum(archive)
    print(f"Created {archive}")


def mac_plist(app_version: str, minimum: str) -> dict:
    numeric = app_version.split("-", 1)[0].split("+", 1)[0]
    return {
        "CFBundleIdentifier": APP_ID, "CFBundleName": APP_NAME,
        "CFBundleDisplayName": APP_NAME, "CFBundleExecutable": "zm-linux",
        "CFBundlePackageType": "APPL", "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleShortVersionString": numeric, "CFBundleVersion": numeric,
        "CFBundleIconFile": "zm.icns", "LSMinimumSystemVersion": minimum,
        "LSApplicationCategoryType": "public.app-category.games",
        "NSHighResolutionCapable": True, "NSPrincipalClass": "NSApplication",
    }


def verify_macos_dependencies(binary: Path) -> None:
    dependencies = run("otool", "-L", binary, capture=True)
    for line in dependencies.splitlines()[1:]:
        library = line.strip().split(" (", 1)[0]
        if not library.startswith(("/System/Library/", "/usr/lib/")):
            raise RuntimeError(f"Non-system library must be bundled before shipping: {library}")
    print(dependencies)


def build_macos(output: Path, skip_build: bool) -> None:
    architecture = platform.machine()
    if architecture != "arm64":
        raise RuntimeError("macOS packages require a native Apple Silicon environment (arm64)")
    target = MAC_TARGET
    env = os.environ.copy()
    minimum = env.setdefault("MACOSX_DEPLOYMENT_TARGET", "13.0")
    identity = env.get("MACOS_SIGNING_IDENTITY", "-")
    notary_profile = env.get("MACOS_NOTARY_PROFILE")
    if notary_profile and identity == "-":
        raise RuntimeError("Notarization requires a Developer ID signing identity")
    if not skip_build:
        run("cargo", "build", "--release", "--locked", "--bin", "zm-linux",
            "--target", target, "--target-dir", ROOT / "target", env=env)
    binary = ROOT / "target" / target / "release/zm-linux"
    run("lipo", "-verify_arch", architecture, binary)
    verify_macos_dependencies(binary)
    work = ROOT / "target/packaging" / f"macos-{architecture}"
    stage = work / "dmg"
    if stage.exists():
        shutil.rmtree(stage)
    app = stage / f"{APP_NAME}.app"
    contents = app / "Contents"
    (contents / "MacOS").mkdir(parents=True)
    resources = contents / "Resources"
    copy_notices(resources)
    shutil.copy2(binary, contents / "MacOS/zm-linux")
    (contents / "MacOS/zm-linux").chmod(0o755)
    shutil.copy2(ROOT / "assets/zm.icns", resources / "zm.icns")
    with (contents / "Info.plist").open("wb") as file:
        plistlib.dump(mac_plist(version(), minimum), file)
    run("plutil", "-lint", contents / "Info.plist")
    signing = ["--timestamp", "--options", "runtime"] if identity != "-" else []
    run("codesign", "--force", "--sign", identity, *signing, app)
    run("codesign", "--verify", "--strict", "--verbose=2", app)
    (stage / "Applications").symlink_to("/Applications", target_is_directory=True)
    shutil.copy2(ROOT / "packaging/README.txt", stage / "README.txt")
    image = output / f"ZM-LINUX-macos-{architecture}.dmg"
    run("hdiutil", "create", "-ov", "-volname", APP_NAME, "-srcfolder", stage,
        "-format", "UDZO", image)
    if identity != "-":
        run("codesign", "--force", "--timestamp", "--sign", identity, image)
    if notary_profile:
        status = json.loads(run("xcrun", "notarytool", "submit", image,
                                "--keychain-profile", notary_profile, "--wait",
                                "--output-format", "json", capture=True))
        if status.get("status") != "Accepted":
            raise RuntimeError(f"Notarization failed: submission {status.get('id')} ({status.get('status')})")
        run("xcrun", "stapler", "staple", image)
        run("xcrun", "stapler", "validate", image)
    run("hdiutil", "verify", image)
    checksum(image)
    print(f"Created {image}; signing: {'ad-hoc (test build)' if identity == '-' else 'Developer ID'}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=("windows", "macos"), required=True)
    parser.add_argument("--skip-build", action="store_true", help="Reuse a binary built by this script for the same target")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    expected = "win32" if args.platform == "windows" else "darwin"
    if sys.platform != expected:
        parser.error(f"{args.platform} packages must be built on {expected}; use GitHub Actions or that OS")
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if args.platform == "windows":
        build_windows(output, args.skip_build)
    else:
        build_macos(output, args.skip_build)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Packaging failed: {error}", file=sys.stderr)
        sys.exit(1)
