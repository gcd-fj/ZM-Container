#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APPDIR="${PROJECT_ROOT}/target/appimage/AppDir"
DIST="${PROJECT_ROOT}/dist"
LINUXDEPLOY="${LINUXDEPLOY:-linuxdeploy}"
APPIMAGETOOL="${APPIMAGETOOL:-}"
APPIMAGE_RUNTIME_FILE="${APPIMAGE_RUNTIME_FILE:-}"

command -v "${LINUXDEPLOY}" >/dev/null || { echo "linuxdeploy not found" >&2; exit 1; }
if [[ "${SKIP_CARGO_BUILD:-0}" != "1" ]]; then
    cargo build --locked --release --bin zm-container --manifest-path "${PROJECT_ROOT}/Cargo.toml"
fi
[[ -x "${PROJECT_ROOT}/target/release/zm-container" ]] || { echo "release binary not found" >&2; exit 1; }

rm -rf "${APPDIR}"
mkdir -p "${APPDIR}/usr/bin" "${APPDIR}/usr/share/applications" "${APPDIR}/usr/share/icons/hicolor/512x512/apps" "${DIST}"
install -m 0755 "${PROJECT_ROOT}/target/release/zm-container" "${APPDIR}/usr/bin/zm-container"
install -m 0644 "${PROJECT_ROOT}/packaging/appimage/io.github.gcd-fj.zm-linux.desktop" "${APPDIR}/usr/share/applications/"
# Keep AppStream metadata out of local packages to avoid appimagetool
# performing an online homepage check during an otherwise local build.
install -m 0644 "${PROJECT_ROOT}/assets/io.github.gcd-fj.zm-linux.png" "${APPDIR}/usr/share/icons/hicolor/512x512/apps/"

if [[ -n "${APPIMAGETOOL}" ]]; then
    command -v "${APPIMAGETOOL}" >/dev/null || { echo "appimagetool not found" >&2; exit 1; }
    "${LINUXDEPLOY}" --appdir "${APPDIR}" --desktop-file "${APPDIR}/usr/share/applications/io.github.gcd-fj.zm-linux.desktop" --icon-file "${APPDIR}/usr/share/icons/hicolor/512x512/apps/io.github.gcd-fj.zm-linux.png"
    APPIMAGETOOL_ARGS=(--no-appstream)
    if [[ -n "${APPIMAGE_RUNTIME_FILE}" ]]; then
        APPIMAGETOOL_ARGS+=(--runtime-file "${APPIMAGE_RUNTIME_FILE}")
    fi
    ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "${APPIMAGETOOL}" "${APPIMAGETOOL_ARGS[@]}" "${APPDIR}" "${DIST}/ZM-Container-x86_64.AppImage"
else
    OUTPUT="${DIST}/ZM-Container-x86_64.AppImage" "${LINUXDEPLOY}" --appdir "${APPDIR}" --desktop-file "${APPDIR}/usr/share/applications/io.github.gcd-fj.zm-linux.desktop" --icon-file "${APPDIR}/usr/share/icons/hicolor/512x512/apps/io.github.gcd-fj.zm-linux.png" --output appimage
fi
(cd "${DIST}" && sha256sum ZM-Container-x86_64.AppImage > ZM-Container-x86_64.AppImage.sha256)
