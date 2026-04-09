#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../../../.." && pwd)"

INSTALLER="${QQPY_INSTALLER_PATH:-${ROOT_DIR}/QQPinyin_Setup_6.6.6304.400.exe}"
PREFIX="${QQPY_WINEPREFIX:-${ROOT_DIR}/.wine32}"
USE_XVFB="${QQPY_USE_XVFB:-1}"
WINE_TIMEOUT_SEC="${QQPY_WINE_TIMEOUT_SEC:-240}"
TRY_INNER_SETUP="${QQPY_TRY_INNER_SETUP:-0}"

TS="$(date +%Y%m%d_%H%M%S)"
UNPACK_DIR="${ROOT_DIR}/.cache/qqpinyin_unpack_${TS}"

run_wine() {
  if [[ "${USE_XVFB}" == "1" ]]; then
    timeout "${WINE_TIMEOUT_SEC}" xvfb-run -a env WINEPREFIX="${PREFIX}" WINEDEBUG=-all wine "$@"
  else
    timeout "${WINE_TIMEOUT_SEC}" env WINEPREFIX="${PREFIX}" WINEDEBUG=-all wine "$@"
  fi
}

echo "[qqpy-install] root=${ROOT_DIR}"
echo "[qqpy-install] installer=${INSTALLER}"
echo "[qqpy-install] prefix=${PREFIX}"
echo "[qqpy-install] try_inner_setup=${TRY_INNER_SETUP}"

if [[ ! -f "${INSTALLER}" ]]; then
  echo "[qqpy-install] ERROR: installer not found"
  exit 2
fi

mkdir -p "${ROOT_DIR}/.cache"
mkdir -p "${PREFIX}"

if [[ ! -f "${PREFIX}/system.reg" ]]; then
  echo "[qqpy-install] init win32 prefix"
  WINEARCH=win32 WINEPREFIX="${PREFIX}" WINEDEBUG=-all wineboot -u
fi

echo "[qqpy-install] set win7"
WINEPREFIX="${PREFIX}" WINEDEBUG=-all wine reg add "HKCU\\Software\\Wine" /v Version /t REG_SZ /d win7 /f

echo "[qqpy-install] extract NSIS package"
mkdir -p "${UNPACK_DIR}"
7z x -y "${INSTALLER}" -o"${UNPACK_DIR}"

PAYLOAD_DIR="$(find "${UNPACK_DIR}" -type d | rg '/\$_35_$' | sed -n '1p')"
if [[ -z "${PAYLOAD_DIR}" ]]; then
  echo "[qqpy-install] ERROR: cannot locate payload directory (\$_35_)"
  exit 3
fi

SETUP_X86="$(find "${UNPACK_DIR}" -type f -name 'QQPYSetup_x86.exe' | sed -n '1p')"
echo "[qqpy-install] payload_dir=${PAYLOAD_DIR}"
echo "[qqpy-install] inner_setup=${SETUP_X86:-<missing>}"

if [[ "${TRY_INNER_SETUP}" == "1" ]]; then
  if [[ -n "${SETUP_X86}" ]]; then
    echo "[qqpy-install] try inner setup (optional)"
    set +e
    run_wine "${SETUP_X86}" /S
    INNER_RC=$?
    set -e
    echo "[qqpy-install] inner_setup_rc=${INNER_RC}"
  else
    echo "[qqpy-install] WARN: inner setup missing, skip"
  fi
else
  echo "[qqpy-install] skip inner setup (manual deploy mode)"
fi

DEST_DIR="${PREFIX}/drive_c/Program Files/QQPinyin"
SYS32_DIR="${PREFIX}/drive_c/windows/system32"

echo "[qqpy-install] manual payload deploy"
mkdir -p "${DEST_DIR}"
cp -a "${PAYLOAD_DIR}"/. "${DEST_DIR}"/

for f in QQPinyin.ime QQPinyinTsf.dll QQImeUtil.dll; do
  if [[ -f "${DEST_DIR}/${f}" ]]; then
    cp -f "${DEST_DIR}/${f}" "${SYS32_DIR}/${f}"
  fi
done

echo "[qqpy-install] optional regsvr32"
set +e
run_wine regsvr32 /s "C:\\windows\\system32\\QQPinyinTsf.dll"
TSF_RC=$?
run_wine regsvr32 /s "C:\\windows\\system32\\QQPinyin.ime"
IME_RC=$?
set -e
echo "[qqpy-install] regsvr_tsf_rc=${TSF_RC}"
echo "[qqpy-install] regsvr_ime_rc=${IME_RC}"

echo "[qqpy-install] verify files"
ls -l \
  "${DEST_DIR}/QQPinyin.ime" \
  "${DEST_DIR}/QQPinyinTsf.dll" \
  "${DEST_DIR}/QQImeUtil.dll" \
  "${SYS32_DIR}/QQPinyin.ime" \
  "${SYS32_DIR}/QQPinyinTsf.dll" \
  "${SYS32_DIR}/QQImeUtil.dll"

if [[ -f "${ROOT_DIR}/src/reverse/probe/ime_probe.exe" ]]; then
  echo "[qqpy-install] probe load check"
  run_wine "${ROOT_DIR}/src/reverse/probe/ime_probe.exe" "C:\\windows\\system32\\QQPinyin.ime"
fi

echo "[qqpy-install] done"
echo "[qqpy-install] unpack_dir=${UNPACK_DIR}"
