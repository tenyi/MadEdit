#!/usr/bin/env bash
#
# 產生 Linux AppImage（MadEdit-<版本>-linux-<架構>.AppImage）。
#
# 用法：
#   ./scripts/make_appimage.sh [輸出目錄] [架構]
#
# 架構為 x86_64 或 aarch64，省略時與 make_linux_dist.sh 相同（本機架構）。
# 會先產生同架構的 tar.gz 發行目錄，再把執行檔與拼字字典包進 AppImage。
#
# 需要：curl，以及 make_linux_dist.sh 的建置依賴（cargo-zigbuild、zig）。
# linuxdeploy 與 appimagetool 會下載到 target/appimage-tools/（已在 .gitignore 的 target 內）。
set -euo pipefail

if [[ "$(uname)" != "Linux" ]]; then
    echo "此腳本僅適用於 Linux" >&2
    exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="MadEdit"
VERSION="$(grep -m1 '^version' "${ROOT}/Cargo.toml" | sed -E 's/.*=[[:space:]]*"([^"]+)".*/\1/')"
OUT_DIR="${1:-${ROOT}/dist}"
ARCH_RAW="${2:-${MADEDIT_ARCH:-$(uname -m)}}"
case "${ARCH_RAW}" in
    x86_64|amd64|x64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *)
        echo "不支援的架構：${ARCH_RAW}（請用 x86_64 或 aarch64）" >&2
        exit 1
        ;;
esac

mkdir -p "${OUT_DIR}"
OUT_DIR="$(cd "${OUT_DIR}" && pwd)"
NAME="${APP_NAME}-${VERSION}-linux-${ARCH}"
if [[ "${MADEDIT_WITH_FORMATTER:-0}" == "1" ]]; then
    NAME="${NAME}-dprint"
fi
STAGE="${OUT_DIR}/${NAME}"

echo "準備 ${NAME} 發行目錄..."
"${ROOT}/scripts/make_linux_dist.sh" "${OUT_DIR}" "${ARCH}"

if [[ ! -x "${STAGE}/${APP_NAME}" ]]; then
    echo "找不到發行執行檔：${STAGE}/${APP_NAME}" >&2
    exit 1
fi

# --- 下載 AppImage 工具 ---------------------------------------------------
TOOL_DIR="${ROOT}/target/appimage-tools"
mkdir -p "${TOOL_DIR}"
fetch_tool() {
    local url="$1" dest="$2"
    if [[ ! -x "${dest}" ]]; then
        echo "下載 $(basename "${dest}")..."
        curl -fL --retry 3 --retry-delay 2 -o "${dest}.partial" "${url}"
        chmod +x "${dest}.partial"
        mv "${dest}.partial" "${dest}"
    fi
}
fetch_tool \
    "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${ARCH}.AppImage" \
    "${TOOL_DIR}/linuxdeploy-${ARCH}.AppImage"
fetch_tool \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${ARCH}.AppImage" \
    "${TOOL_DIR}/appimagetool-${ARCH}.AppImage"

# 無 FUSE 的 CI 環境改為解壓後執行。
export APPIMAGE_EXTRACT_AND_RUN=1
export ARCH
export VERSION

# 檔案總管開啟檔案時要把路徑傳進 AppImage；發行用的 .desktop 在安裝腳本裡才改寫 Exec。
# linuxdeploy recognizes a desktop file by its .desktop extension, so keep the original name inside a temp dir.
DESKTOP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/madedit-desktop.XXXXXX")"
DESKTOP="${DESKTOP_DIR}/madedit.desktop"
sed 's|^Exec=.*|Exec=MadEdit %F|' "${STAGE}/madedit.desktop" > "${DESKTOP}"
APPDIR="${OUT_DIR}/.${NAME}.AppDir"
cleanup() {
    rm -rf "${DESKTOP_DIR:-}"
    if [[ -n "${APPDIR:-}" ]]; then
        rm -rf "${APPDIR}"
    fi
}
trap cleanup EXIT
rm -rf "${APPDIR}"
echo "組裝 AppDir..."
"${TOOL_DIR}/linuxdeploy-${ARCH}.AppImage" \
    --appdir "${APPDIR}" \
    --executable "${STAGE}/${APP_NAME}" \
    --desktop-file "${DESKTOP}" \
    --icon-file "${STAGE}/madedit-icon.png"

# 拼字字典要跟執行檔放在同一層，spell.rs 會從 current_exe 的目錄載入。
BIN_IN_APP="$(find "${APPDIR}" -type f -name "${APP_NAME}" | head -n 1)"
if [[ -z "${BIN_IN_APP}" ]]; then
    echo "linuxdeploy 沒有把執行檔放進 AppDir" >&2
    exit 1
fi
mkdir -p "$(dirname "${BIN_IN_APP}")/dictionaries"
cp -a "${STAGE}/dictionaries/." "$(dirname "${BIN_IN_APP}")/dictionaries/"

# 「帶 DPRINT」變體：格式化工具與執行檔放同一層（core/dprint.rs 會在 current_exe 旁找 formatters/）
if [[ -d "${STAGE}/formatters" ]]; then
    mkdir -p "$(dirname "${BIN_IN_APP}")/formatters"
    cp -a "${STAGE}/formatters/." "$(dirname "${BIN_IN_APP}")/formatters/"
fi

APPIMAGE="${OUT_DIR}/${NAME}.AppImage"
rm -f "${APPIMAGE}"
echo "產生 ${NAME}.AppImage..."
"${TOOL_DIR}/appimagetool-${ARCH}.AppImage" "${APPDIR}" "${APPIMAGE}"

if command -v sha256sum >/dev/null 2>&1; then
    (cd "${OUT_DIR}" && sha256sum "${NAME}.AppImage" > "${NAME}.AppImage.sha256")
elif command -v shasum >/dev/null 2>&1; then
    (cd "${OUT_DIR}" && shasum -a 256 "${NAME}.AppImage" > "${NAME}.AppImage.sha256")
fi
echo "完成：${APPIMAGE}"
