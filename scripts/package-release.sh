#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"
IMAGE="kronkeeper:${VERSION}"
OUT_DIR="${1:-dist}"
PKG_NAME="kronkeeper-${VERSION}"
STAGING="${OUT_DIR}/${PKG_NAME}"

echo "==> Building Docker image ${IMAGE}"
docker build -t "${IMAGE}" -t kronkeeper:latest .

echo "==> Staging deploy bundle at ${STAGING}"
rm -rf "${STAGING}"
mkdir -p "${STAGING}/scripts"

cp deploy/docker-compose.yml "${STAGING}/"
cp deploy/up.sh "${STAGING}/"
cp deploy/README.md "${STAGING}/"
cp deploy/scripts/hello.sh "${STAGING}/scripts/"
chmod +x "${STAGING}/up.sh" "${STAGING}/scripts/hello.sh"

# Pin the image tag for this release.
sed "s/KRONKEEPER_IMAGE=kronkeeper:latest/KRONKEEPER_IMAGE=${IMAGE}/" \
  deploy/.env.example > "${STAGING}/.env.example"

echo "==> Saving Docker image to ${STAGING}/kronkeeper-image.tar"
docker save "${IMAGE}" -o "${STAGING}/kronkeeper-image.tar"

ARCHIVE="${OUT_DIR}/${PKG_NAME}.tar.gz"
echo "==> Creating ${ARCHIVE}"
mkdir -p "${OUT_DIR}"
tar -czf "${ARCHIVE}" -C "${OUT_DIR}" "${PKG_NAME}"

SIZE="$(du -h "${ARCHIVE}" | cut -f1)"
echo ""
echo "Release package ready:"
echo "  ${ARCHIVE} (${SIZE})"
echo ""
echo "Ship this archive. Recipients run:"
echo "  tar -xzf ${PKG_NAME}.tar.gz && cd ${PKG_NAME} && ./up.sh"
