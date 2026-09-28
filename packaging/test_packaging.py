#!/usr/bin/env python3
"""Offline packaging contract checks; native SDK/signing checks run in CI."""
import hashlib
import importlib.util
import os
from pathlib import Path
import plistlib
import struct
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location("zm_packaging", Path(__file__).with_name("build.py"))
packaging = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packaging)


def write_pe(path, machine=0x8664, subsystem=2):
    data = bytearray(256)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 60, 128)
    data[128:132] = b"PE\0\0"
    struct.pack_into("<H", data, 132, machine)
    struct.pack_into("<H", data, 128 + 24 + 68, subsystem)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.output = self.root / "dist"
        self.output.mkdir()
        for name in ["LICENSE", "THIRD_PARTY_LICENSES.md", "vendor/ruffle/LICENSE.md",
                     "packaging/README.txt", "assets/zm.icns"]:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture", encoding="utf-8")
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.2.3"\n', encoding="utf-8")
        self.root_patch = patch.object(packaging, "ROOT", self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)

    def test_checksum_uses_portable_sha256sum_format(self):
        path = self.output / "sample.zip"
        path.write_bytes(b"fixture")
        packaging.checksum(path)
        expected = hashlib.sha256(b"fixture").hexdigest() + "  sample.zip\n"
        self.assertEqual(path.with_suffix(".zip.sha256").read_bytes(), expected.encode("ascii"))

    def test_windows_rejects_wrong_architecture_console_and_non_pe_binaries(self):
        path = self.output / "app.exe"
        write_pe(path)
        packaging.verify_windows_executable(path)
        for machine, subsystem in [(0xAA64, 2), (0x8664, 3)]:
            write_pe(path, machine, subsystem)
            with self.assertRaises(RuntimeError):
                packaging.verify_windows_executable(path)
        path.write_bytes(b"\x7fELF")
        with self.assertRaises(RuntimeError):
            packaging.verify_windows_executable(path)

    def test_windows_zip_contains_executable_and_notices_only(self):
        binary = self.root / "target" / packaging.WINDOWS_TARGET / "release/zm-linux.exe"
        write_pe(binary)
        (self.root / "credentials.toml").write_text("must-not-ship", encoding="utf-8")
        with patch.object(packaging, "dumpbin_tool", return_value=Path("dumpbin.exe")), \
                patch.object(packaging, "run", return_value="KERNEL32.dll\nUSER32.dll"):
            packaging.build_windows(self.output, skip_build=True)
        archive = self.output / "ZM-LINUX-windows-x86_64.zip"
        with zipfile.ZipFile(archive) as file:
            self.assertEqual(set(file.namelist()), {
                "ZM-LINUX/zm-linux.exe", "ZM-LINUX/LICENSE", "ZM-LINUX/THIRD_PARTY_LICENSES.md",
                "ZM-LINUX/RUFFLE-LICENSE.md", "ZM-LINUX/README.txt",
            })
            self.assertEqual(file.read("ZM-LINUX/zm-linux.exe"), binary.read_bytes())

    def test_windows_refuses_to_ship_unbundled_vc_runtime_dependency(self):
        write_pe(self.root / "target" / packaging.WINDOWS_TARGET / "release/zm-linux.exe")
        with patch.object(packaging, "dumpbin_tool", return_value=Path("dumpbin.exe")), \
                patch.object(packaging, "run", return_value="VCRUNTIME140.dll"):
            with self.assertRaisesRegex(RuntimeError, "VC runtime"):
                packaging.build_windows(self.output, skip_build=True)
        self.assertFalse(list(self.output.glob("*.zip")))

    def test_macos_rejects_non_system_dynamic_libraries(self):
        for library in ["/opt/homebrew/lib/libexample.dylib", "@rpath/libexample.dylib"]:
            with patch.object(packaging, "run", return_value=f"app:\n {library} (compatibility version 1.0)\n"):
                with self.assertRaisesRegex(RuntimeError, "Non-system library"):
                    packaging.verify_macos_dependencies(Path("app"))

    def test_mac_plist_is_serializable_with_numeric_prerelease_version(self):
        data = packaging.mac_plist("0.2.3-rc.1", "13.0")
        restored = plistlib.loads(plistlib.dumps(data))
        self.assertEqual(restored["CFBundleExecutable"], "zm-linux")
        self.assertEqual(restored["CFBundleVersion"], "0.2.3")
        self.assertEqual(restored["CFBundlePackageType"], "APPL")

    def test_macos_is_apple_silicon_only(self):
        with patch.object(packaging.platform, "machine", return_value="x86_64"):
            with self.assertRaisesRegex(RuntimeError, "Apple Silicon"):
                packaging.build_macos(self.output, skip_build=True)

    def test_notarization_cannot_run_with_ad_hoc_identity(self):
        with patch.object(packaging.platform, "machine", return_value="arm64"), \
                patch.dict(os.environ, {"MACOS_NOTARY_PROFILE": "test-profile"}, clear=True):
            with self.assertRaisesRegex(RuntimeError, "Developer ID"):
                packaging.build_macos(self.output, skip_build=True)

    @unittest.skipIf(os.name == "nt", "Creating a macOS Applications symlink needs Windows privileges")
    def test_macos_bundle_layout_and_signing_precede_dmg_creation(self):
        binary = self.root / "target" / packaging.MAC_TARGET / "release/zm-linux"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"fixture-binary")
        calls = []

        def fake_run(*args, **kwargs):
            calls.append(tuple(map(str, args)))
            if args[:2] == ("hdiutil", "create"):
                Path(args[-1]).write_bytes(b"fixture-dmg")
            return ""

        with patch.object(packaging.platform, "machine", return_value="arm64"), \
                patch.dict(os.environ, {}, clear=True), patch.object(packaging, "run", side_effect=fake_run):
            packaging.build_macos(self.output, skip_build=True)
        stage = self.root / "target/packaging/macos-arm64/dmg"
        app = stage / "ZM-LINUX.app/Contents"
        with (app / "Info.plist").open("rb") as file:
            self.assertEqual(plistlib.load(file)["CFBundleIdentifier"], packaging.APP_ID)
        self.assertEqual((app / "MacOS/zm-linux").read_bytes(), binary.read_bytes())
        self.assertTrue((stage / "Applications").is_symlink())
        sign_index = next(i for i, call in enumerate(calls) if call[:2] == ("codesign", "--force"))
        dmg_index = next(i for i, call in enumerate(calls) if call[:2] == ("hdiutil", "create"))
        self.assertLess(sign_index, dmg_index)
        self.assertTrue((self.output / "ZM-LINUX-macos-arm64.dmg.sha256").is_file())


if __name__ == "__main__":
    unittest.main()
