#!/usr/bin/env bash
# ==============================================================================
# Skoci — Ultra-lightweight Agent-Native Deployment Platform
# 1-Line Universal Installer for Linux (x86_64, aarch64/Raspberry Pi 64-bit) & macOS
# ==============================================================================

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
NC='\033[0m' # No Color

echo -e "${BLUE}${BOLD}"
cat << "EOF"
  ____  _              _ 
 / ___|| | _____   ___(_)
 \___ \| |/ / _ \ / __| |
  ___) |   < (_) | (__| |
 |____/|_|\_\___/ \___|_|
 Ultra-lightweight Agent-Native Deployment Platform
EOF
echo -e "${NC}"

# 1. Detect OS
OS="$(uname -s)"
case "${OS}" in
    Linux*)     OS_NAME="linux" ;;
    Darwin*)   OS_NAME="darwin" ;;
    *)          echo -e "${RED}Error: Sistem operasi '${OS}' tidak didukung. Skoci membutuhkan Linux atau macOS.${NC}"; exit 1 ;;
esac

# 2. Detect Architecture
ARCH="$(uname -m)"
case "${ARCH}" in
    x86_64|amd64)   ARCH_NAME="x86_64" ;;
    aarch64|arm64)  ARCH_NAME="aarch64" ;;
    armv7l|armv6l|i386|i686)
        echo -e "${RED}Error: Arsitektur '${ARCH}' (32-bit) tidak didukung.${NC}"
        echo -e "${YELLOW}Catatan untuk Raspberry Pi: Skoci membutuhkan OS 64-bit (misalnya Raspberry Pi OS 64-bit atau Ubuntu Server 64-bit).${NC}"
        exit 1
        ;;
    *)
        echo -e "${RED}Error: Arsitektur '${ARCH}' tidak didukung.${NC}"
        exit 1
        ;;
esac

echo -e "${GREEN}✓ Terdeteksi:${NC} ${OS_NAME} (${ARCH_NAME})"

# 3. Check / Install Docker
echo -e "\n${BOLD}[1/4] Memeriksa Docker Engine...${NC}"
if command -v docker >/dev/null 2>&1; then
    DOCKER_VERSION=$(docker --version | awk '{print $3}' | tr -d ',')
    echo -e "${GREEN}✓ Docker terpasang:${NC} v${DOCKER_VERSION}"
else
    echo -e "${YELLOW}Docker belum terpasang.${NC}"
    if [ "${OS_NAME}" = "linux" ]; then
        echo -e "Memasang Docker resmi via get.docker.com..."
        curl -fsSL https://get.docker.com | sh
        sudo systemctl enable --now docker || true
        echo -e "${GREEN}✓ Docker berhasil dipasang.${NC}"
    else
        echo -e "${RED}Silakan pasang Docker Desktop untuk macOS terlebih dahulu: https://docker.com${NC}"
        exit 1
    fi
fi

# 4. Binary Installation
echo -e "\n${BOLD}[2/4] Memasang binary Skoci...${NC}"
INSTALL_DIR="/usr/local/bin"
BIN_TARGET="${INSTALL_DIR}/skoci"

# Check if installing from local repo build or downloading
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd || echo "")"
if [ -f "${SCRIPT_DIR}/target/release/skoci" ]; then
    echo -e "Menyalin binary lokal dari target/release/skoci..."
    sudo cp "${SCRIPT_DIR}/target/release/skoci" "${BIN_TARGET}"
elif [ -f "${SCRIPT_DIR}/target/debug/skoci" ]; then
    echo -e "Menyalin binary lokal dari target/debug/skoci..."
    sudo cp "${SCRIPT_DIR}/target/debug/skoci" "${BIN_TARGET}"
else
    echo -e "Mengunduh binary rilis resmi untuk ${OS_NAME}-${ARCH_NAME}..."
    DOWNLOAD_URL="https://github.com/imamjabar/skoci/releases/latest/download/skoci-${OS_NAME}-${ARCH_NAME}"
    TMP_BIN="/tmp/skoci-bin"
    if curl -fSL --progress-bar -o "${TMP_BIN}" "${DOWNLOAD_URL}" 2>/dev/null; then
        sudo mv "${TMP_BIN}" "${BIN_TARGET}"
    else
        echo -e "${YELLOW}Catatan: Binary rilis remote belum tersedia, mengompilasi dari repositori GitHub...${NC}"
        if ! command -v cargo >/dev/null 2>&1; then
            echo -e "Memasang dependensi build dan compiler Rust di sistem..."
            if command -v apt-get >/dev/null 2>&1; then
                sudo apt-get update -qq && sudo apt-get install -y -qq build-essential pkg-config libssl-dev git curl
            fi
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
            export PATH="$HOME/.cargo/bin:$PATH"
        fi
        BUILD_DIR="/tmp/skoci-build"
        rm -rf "${BUILD_DIR}"
        git clone --depth 1 https://github.com/imamjabar/skoci.git "${BUILD_DIR}"
        (cd "${BUILD_DIR}" && cargo build --release -p skoci)
        sudo cp "${BUILD_DIR}/target/release/skoci" "${BIN_TARGET}"
        rm -rf "${BUILD_DIR}"
    fi
fi

sudo chmod +x "${BIN_TARGET}"
echo -e "${GREEN}✓ Skoci binary siap di ${BIN_TARGET}${NC} ($(${BIN_TARGET} --version))"

# 5. Configuration & Master Token Generation
echo -e "\n${BOLD}[3/4] Konfigurasi Token Operator...${NC}"
CONFIG_DIR="/etc/skoci"
DATA_DIR="/var/lib/skoci"
sudo mkdir -p "${CONFIG_DIR}" "${DATA_DIR}" "${DATA_DIR}/backups"

TOKEN_FILE="${CONFIG_DIR}/master_token"
if [ ! -f "${TOKEN_FILE}" ]; then
    RAND_HEX=$(openssl rand -hex 16 2>/dev/null || cat /dev/urandom | tr -dc 'a-f0-9' 2>/dev/null | fold -w 32 | head -n 1)
    MASTER_TOKEN="sk_master_admin_${RAND_HEX}"
    echo "${MASTER_TOKEN}" | sudo tee "${TOKEN_FILE}" >/dev/null
    sudo chmod 600 "${TOKEN_FILE}"
else
    MASTER_TOKEN=$(sudo cat "${TOKEN_FILE}")
fi

# Write environment file
sudo tee "${CONFIG_DIR}/skoci.env" >/dev/null << EOF
SKOCI_TOKEN=${MASTER_TOKEN}
SKOCI_SERVER_URL=http://127.0.0.1:8787
EOF
sudo chmod 600 "${CONFIG_DIR}/skoci.env"
echo -e "${GREEN}✓ Master token tersimpan di ${TOKEN_FILE}${NC}"

# 6. Service Setup (Systemd on Linux)
echo -e "\n${BOLD}[4/4] Mengaktifkan Layanan Latar Belakang...${NC}"
if [ "${OS_NAME}" = "linux" ] && command -v systemctl >/dev/null 2>&1; then
    SERVICE_FILE="/etc/systemd/system/skoci.service"
    sudo tee "${SERVICE_FILE}" >/dev/null << EOF
[Unit]
Description=Skoci Agent-Native Deployment Platform
After=network.target docker.service
Requires=docker.service

[Service]
Type=simple
User=root
WorkingDirectory=${DATA_DIR}
EnvironmentFile=${CONFIG_DIR}/skoci.env
ExecStart=${BIN_TARGET} server --bind 0.0.0.0:8787 --db ${DATA_DIR}/skoci.db
Restart=always
RestartSec=5
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
EOF
    sudo systemctl daemon-reload
    sudo systemctl enable --now skoci
    echo -e "${GREEN}✓ Layanan systemd 'skoci' aktif dan berjalan otomatis saat boot.${NC}"
else
    echo -e "${YELLOW}Menjalankan di macOS atau sistem non-systemd.${NC}"
    echo -e "Untuk menjalankan server secara manual:"
    echo -e "  sudo skoci server --bind 0.0.0.0:8787 --db ${DATA_DIR}/skoci.db"
fi

# 7. Detect Public / Local IP
SERVER_IP=$(hostname -I 2>/dev/null | awk '{print $1}' || echo "127.0.0.1")

echo -e "\n${GREEN}${BOLD}==============================================================================${NC}"
echo -e "${GREEN}${BOLD}🎉 SKOCI BERHASIL DIPASANG!${NC}"
echo -e "${GREEN}${BOLD}==============================================================================${NC}"
echo -e "• ${BOLD}Dashboard Validasi:${NC}  http://${SERVER_IP}:8787"
echo -e "• ${BOLD}Reverse Proxy:${NC}       http://${SERVER_IP}:8000"
echo -e "• ${BOLD}RAM Idle Target:${NC}     <20 MB (aktual ~7.8 MB)"
echo -e "• ${BOLD}Master Token:${NC}        ${YELLOW}${MASTER_TOKEN}${NC}"
echo -e "\n${BOLD}Langkah Selanjutnya:${NC}"
echo -e "1. Buka Dashboard di browser untuk memvalidasi: ${BLUE}http://${SERVER_IP}:8787${NC}"
echo -e "2. Buat token khusus untuk AI Agent Anda:"
echo -e "   ${BOLD}skoci token create --app toko --name agent-ci${NC}"
echo -e "3. Di repositori aplikasi Anda (Node/Python/Go/Rust/Laravel/Docker):"
echo -e "   ${BOLD}skoci init${NC}"
echo -e "   ${BOLD}skoci deploy${NC}"
echo -e "\nDokumentasi lengkap: https://zen.ai.id/skoci"
EOF
