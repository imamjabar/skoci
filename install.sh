#!/usr/bin/env bash
# ==============================================================================
# Skoci — Ultra-lightweight Agent-Native Deployment Platform
# 1-Line Universal Installer for Linux (x86_64, aarch64) & macOS
# ==============================================================================

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
NC='\033[0m'

echo -e "${BLUE}${BOLD}"
cat << "BANNER"
  ____  _              _ 
 / ___|| | _____   ___(_)
 \___ \| |/ / _ \ / __| |
  ___) |   < (_) | (__| |
 |____/|_|\_\___/ \___|_|
   Agent-Native Deploy
BANNER
echo -e "${NC}"

# 0. Sudo & User Helpers
if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
else
  if command -v sudo >/dev/null 2>&1; then
    SUDO="sudo"
  else
    echo -e "${RED}Error: Installer membutuhkan hak akses root atau sudo.${NC}" >&2
    exit 1
  fi
fi

INVOKING_USER="${SUDO_USER:-$USER}"

# 1. Parse Command Line Arguments
CLI_DOMAIN=""
CLI_EMAIL=""
NO_TLS=0
CLI_VERSION="latest"
CLIENT_ONLY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --domain)
      if [ -n "${2:-}" ]; then
        CLI_DOMAIN="$2"
        shift 2
      else
        echo -e "${RED}Error: --domain membutuhkan nilai${NC}" >&2
        exit 1
      fi
      ;;
    --email)
      if [ -n "${2:-}" ]; then
        CLI_EMAIL="$2"
        shift 2
      else
        echo -e "${RED}Error: --email membutuhkan nilai${NC}" >&2
        exit 1
      fi
      ;;
    --no-tls)
      NO_TLS=1
      shift
      ;;
    --version)
      if [ -n "${2:-}" ]; then
        CLI_VERSION="$2"
        shift 2
      else
        echo -e "${RED}Error: --version membutuhkan nilai${NC}" >&2
        exit 1
      fi
      ;;
    --client-only)
      CLIENT_ONLY=1
      shift
      ;;
    *)
      echo -e "${RED}Error: Opsi tidak dikenal: $1${NC}" >&2
      exit 1
      ;;
  esac
done

# 2. Detect OS & Architecture
OS="$(uname -s)"
case "${OS}" in
  Linux*)
    OS_NAME="linux"
    ;;
  Darwin*)
    OS_NAME="darwin"
    CLIENT_ONLY=1
    ;;
  *)
    echo -e "${RED}Error: Sistem operasi '${OS}' tidak didukung.${NC}" >&2
    exit 1
    ;;
esac

ARCH="$(uname -m)"
case "${ARCH}" in
  x86_64|amd64)
    ARCH_NAME="x86_64"
    ;;
  aarch64|arm64)
    ARCH_NAME="aarch64"
    ;;
  *)
    echo -e "${RED}Error: Arsitektur '${ARCH}' tidak didukung (hanya x86_64 dan aarch64).${NC}" >&2
    exit 1
    ;;
esac

echo -e "${GREEN}✓ Terdeteksi:${NC} ${OS_NAME} (${ARCH_NAME})"

# Helpers
verify_checksum() {
  local sum_file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "$sum_file"
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "$sum_file"
  else
    echo "Peringatan: tidak ditemukan sha256sum/shasum; verifikasi dilewati." >&2
    return 0
  fi
}

TMP_DIR="$(mktemp -d /tmp/skoci-install.XXXXXX)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

build_from_source() {
  local target_dest="$1"
  echo -e "\n${YELLOW}Mengompilasi Skoci dari source code (proses ini membutuhkan waktu ~4 menit)...${NC}"

  # Periksa binary lokal jika dijalankan dari repo lokal
  local script_file="${BASH_SOURCE[0]:-}"
  local script_dir=""
  if [ -n "$script_file" ] && [ -f "$script_file" ]; then
    script_dir="$(cd "$(dirname "$script_file")" 2>/dev/null && pwd || echo "")"
  fi

  if [ -n "$script_dir" ] && [ -f "${script_dir}/target/release/skoci" ]; then
    echo "Menggunakan binary lokal dari ${script_dir}/target/release/skoci"
    cp "${script_dir}/target/release/skoci" "$target_dest"
    return 0
  fi

  if [ "$OS_NAME" = "linux" ] && command -v apt-get >/dev/null 2>&1; then
    echo "Memasang dependensi build via apt-get..."
    $SUDO apt-get update -y
    $SUDO apt-get install -y build-essential pkg-config git curl
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo "Memasang Rust toolchain (minimal profile)..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env" 2>/dev/null || export PATH="$HOME/.cargo/bin:$PATH"
  fi

  local src_dir
  if [ -n "$script_dir" ] && [ -f "${script_dir}/Cargo.toml" ]; then
    src_dir="$script_dir"
  else
    src_dir="${TMP_DIR}/repo"
    echo "Mengunduh kode sumber repositori Skoci..."
    git clone https://github.com/imamjabar/skoci.git "$src_dir"
  fi

  (
    cd "$src_dir"
    cargo build --release -p skoci --locked
  )
  cp "${src_dir}/target/release/skoci" "$target_dest"
}

# 3. Download or Build Skoci Binary
ASSET_NAME="skoci-${OS_NAME}-${ARCH_NAME}"
TAG=""
if [ -n "$CLI_VERSION" ] && [ "$CLI_VERSION" != "latest" ]; then
  TAG="$CLI_VERSION"
else
  TAG="$(curl -fsSI "https://github.com/imamjabar/skoci/releases/latest" 2>/dev/null | grep -i "^location:" | sed -E 's/.*tag\/(.*)/\1/' | tr -d '\r\n ' || true)"
  if [ -z "$TAG" ]; then
    TAG="v0.1.0"
  fi
fi

DOWNLOAD_URL="https://github.com/imamjabar/skoci/releases/download/${TAG}/${ASSET_NAME}"
SHA_URL="https://github.com/imamjabar/skoci/releases/download/${TAG}/${ASSET_NAME}.sha256"

DOWNLOAD_OK=0
echo -e "\n${BOLD}[1/4] Menyiapkan binary Skoci...${NC}"
echo "Mengunduh ${ASSET_NAME} (${TAG})..."
if curl -fsSL "$DOWNLOAD_URL" -o "${TMP_DIR}/${ASSET_NAME}" 2>/dev/null && \
   curl -fsSL "$SHA_URL" -o "${TMP_DIR}/${ASSET_NAME}.sha256" 2>/dev/null; then
  cd "$TMP_DIR"
  if verify_checksum "${ASSET_NAME}.sha256"; then
    echo -e "${GREEN}✓ Verifikasi SHA256 berhasil.${NC}"
    DOWNLOAD_OK=1
  else
    echo -e "${YELLOW}Peringatan: Verifikasi SHA256 gagal.${NC}" >&2
  fi
  cd - >/dev/null
fi

if [ "$DOWNLOAD_OK" -ne 1 ]; then
  echo -e "${YELLOW}Release binary tidak tersedia di GitHub Releases. Beralih ke kompilasi source...${NC}"
  build_from_source "${TMP_DIR}/${ASSET_NAME}"
fi

# 4. Install Binary for Client-Only (macOS or --client-only)
if [ "$CLIENT_ONLY" = "1" ]; then
  INSTALL_DIR="/usr/local/bin"
  if [ "$OS_NAME" = "darwin" ] && [ ! -w "/usr/local/bin" ] && [ -z "$SUDO" ]; then
    INSTALL_DIR="$HOME/.local/bin"
    mkdir -p "$INSTALL_DIR"
    cp "${TMP_DIR}/${ASSET_NAME}" "${INSTALL_DIR}/skoci"
    chmod 0755 "${INSTALL_DIR}/skoci"
  else
    $SUDO mkdir -p "$INSTALL_DIR"
    $SUDO cp "${TMP_DIR}/${ASSET_NAME}" "${INSTALL_DIR}/skoci"
    $SUDO chmod 0755 "${INSTALL_DIR}/skoci"
  fi

  echo -e "\n${GREEN}${BOLD}==============================================================================${NC}"
  echo -e "${GREEN}${BOLD}🎉 SKOCI CLIENT BERHASIL DIPASANG!${NC}"
  echo -e "${GREEN}${BOLD}==============================================================================${NC}"
  echo -e "Binary terpasang di: ${BOLD}${INSTALL_DIR}/skoci${NC}"
  echo -e "\n${BOLD}Langkah Selanjutnya:${NC}"
  echo -e "1. Hubungkan ke server Skoci Anda:"
  echo -e "   ${BOLD}skoci connect --server https://skoci.<base-domain> --token <token>${NC}"
  echo -e "2. Masuk ke direktori repositori proyek Anda:"
  echo -e "   ${BOLD}skoci init${NC}"
  echo -e "3. Deploy aplikasi:"
  echo -e "   ${BOLD}skoci deploy${NC}"
  exit 0
fi

# 5. Linux Server Installation
INSTALL_DIR="/usr/local/bin"
BIN_TARGET="${INSTALL_DIR}/skoci"
$SUDO cp "${TMP_DIR}/${ASSET_NAME}" "${BIN_TARGET}"
$SUDO chmod 0755 "${BIN_TARGET}"
echo -e "${GREEN}✓ Binary terpasang di ${BIN_TARGET}${NC}"

# Check / Install Docker Engine
echo -e "\n${BOLD}[2/4] Memeriksa Docker Engine...${NC}"
if ! command -v docker >/dev/null 2>&1; then
  echo "Docker Engine tidak ditemukan. Mengunduh dan memasang Docker via get.docker.com..."
  curl -fsSL https://get.docker.com | $SUDO sh
fi

$SUDO systemctl start docker || true
$SUDO systemctl enable docker || true
echo -e "${GREEN}✓ Docker Engine aktif.${NC}"

# Create System User & Groups
if ! id -u skoci >/dev/null 2>&1; then
  echo "Membuat pengguna sistem 'skoci'..."
  $SUDO useradd --system --home /var/lib/skoci --shell /usr/sbin/nologin skoci
fi
$SUDO usermod -aG docker skoci

if [ -n "$INVOKING_USER" ] && [ "$INVOKING_USER" != "root" ]; then
  if id -u "$INVOKING_USER" >/dev/null 2>&1; then
    $SUDO usermod -aG skoci "$INVOKING_USER"
    echo -e "${GREEN}✓ Menambahkan user '${INVOKING_USER}' ke grup 'skoci'.${NC}"
  fi
fi

# Setup Directories & Permissions
$SUDO mkdir -p /etc/skoci /var/lib/skoci /var/lib/skoci/backups
$SUDO chown root:skoci /etc/skoci
$SUDO chmod 0750 /etc/skoci
$SUDO chown -R skoci:skoci /var/lib/skoci
$SUDO chmod 0750 /var/lib/skoci

# Setup Master Token
echo -e "\n${BOLD}[3/4] Konfigurasi Token & Jaringan...${NC}"
MASTER_TOKEN=""
if $SUDO test -f /etc/skoci/master_token; then
  MASTER_TOKEN="$($SUDO cat /etc/skoci/master_token | tr -d '\r\n ')"
fi
if [ -z "$MASTER_TOKEN" ] && $SUDO test -f /etc/skoci/skoci.env; then
  OLD_TOKEN="$($SUDO grep -E '^SKOCI_TOKEN=' /etc/skoci/skoci.env | cut -d= -f2- | tr -d '\r\n ' || true)"
  if [ -n "$OLD_TOKEN" ]; then
    MASTER_TOKEN="$OLD_TOKEN"
  fi
fi
if [ -z "$MASTER_TOKEN" ]; then
  HEX="$(openssl rand -hex 24 2>/dev/null || od -vN 24 -An -tx1 /dev/urandom | tr -d ' \n')"
  MASTER_TOKEN="sk_master_${HEX}"
fi

echo "$MASTER_TOKEN" | $SUDO tee /etc/skoci/master_token >/dev/null
$SUDO chown root:skoci /etc/skoci/master_token
$SUDO chmod 0640 /etc/skoci/master_token

# Public IP & Base Domain
is_ipv4() {
  echo "$1" | grep -Eq '^([0-9]{1,3}\.){3}[0-9]{1,3}$'
}
PUBLIC_IP=""
for source in "curl -fsS --max-time 5 https://api.ipify.org" "curl -fsS --max-time 5 https://ifconfig.me/ip"; do
  CANDIDATE="$($source 2>/dev/null | tr -d '\r\n ' || true)"
  if is_ipv4 "$CANDIDATE"; then
    PUBLIC_IP="$CANDIDATE"
    break
  fi
done
if [ -z "$PUBLIC_IP" ]; then
  CANDIDATE="$(hostname -I 2>/dev/null | awk '{print $1}')"
  if is_ipv4 "$CANDIDATE"; then
    PUBLIC_IP="$CANDIDATE"
  fi
fi
if [ -z "$PUBLIC_IP" ]; then
  echo -e "${RED}Error: IP publik server tidak terdeteksi. Jalankan ulang dengan --domain <domain-anda>.${NC}" >&2
  exit 1
fi

if [ -n "$CLI_DOMAIN" ]; then
  BASE_DOMAIN="$CLI_DOMAIN"
else
  DASHED_IP="$(echo "$PUBLIC_IP" | tr '.' '-')"
  BASE_DOMAIN="${DASHED_IP}.sslip.io"
fi

if [ "$NO_TLS" = "1" ]; then
  TLS_MODE="off"
else
  TLS_MODE="acme"
fi

# Write /etc/skoci/skoci.env
$SUDO tee /etc/skoci/skoci.env >/dev/null <<EOF
SKOCI_MASTER_TOKEN=${MASTER_TOKEN}
SKOCI_BASE_DOMAIN=${BASE_DOMAIN}
SKOCI_TLS=${TLS_MODE}
SKOCI_PUBLIC_IP=${PUBLIC_IP}
EOF
if [ -n "$CLI_EMAIL" ]; then
  echo "SKOCI_ACME_EMAIL=${CLI_EMAIL}" | $SUDO tee -a /etc/skoci/skoci.env >/dev/null
fi
$SUDO chown root:skoci /etc/skoci/skoci.env
$SUDO chmod 0640 /etc/skoci/skoci.env

# Port Conflict Check
if command -v ss >/dev/null 2>&1; then
  LISTENERS="$($SUDO ss -ltnp 2>/dev/null | grep -E ':(80|443)\b' | grep -v 'skoci' || true)"
  if [ -n "$LISTENERS" ]; then
    echo -e "\n${YELLOW}⚠️ PERINGATAN: Port 80 atau 443 sedang digunakan oleh proses lain:${NC}"
    echo "$LISTENERS"
    if echo "$LISTENERS" | grep -q "docker-proxy"; then
      CONTAINERS_80="$($SUDO docker ps --filter publish=80 --format '{{.Names}}' 2>/dev/null || true)"
      CONTAINERS_443="$($SUDO docker ps --filter publish=443 --format '{{.Names}}' 2>/dev/null || true)"
      ALL_CONFLICT="$(echo -e "${CONTAINERS_80}\n${CONTAINERS_443}" | grep -v '^$' | sort -u || true)"
      if [ -n "$ALL_CONFLICT" ]; then
        echo -e "\n${YELLOW}Kontainer Docker berikut memublikasikan port 80/443:${NC}"
        for cname in $ALL_CONFLICT; do
          echo -e "  - ${BOLD}$cname${NC} (Bebaskan dengan: ${BOLD}sudo docker rm -f $cname${NC})"
        done
      fi
    fi
    echo -e "${YELLOW}Proxy Skoci tidak dapat mengikat port 80/443 sampai proses di atas dihentikan.${NC}"
    echo -e "${YELLOW}Instalasi akan tetap dilanjutkan.${NC}\n"
  fi
fi

# 6. Setup Systemd Service
echo -e "\n${BOLD}[4/4] Mengaktifkan Layanan Systemd...${NC}"
if systemctl is-active --quiet skoci 2>/dev/null; then
  $SUDO systemctl stop skoci
fi
$SUDO chown -R skoci:skoci /var/lib/skoci

$SUDO tee /etc/systemd/system/skoci.service >/dev/null << "EOF"
[Unit]
Description=Skoci Agent-Native Deploy Platform
After=network.target docker.service
Wants=docker.service

[Service]
Type=simple
User=skoci
Group=skoci
SupplementaryGroups=docker
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/var/lib/skoci
EnvironmentFile=/etc/skoci/skoci.env
ExecStart=/usr/local/bin/skoci server --data-dir /var/lib/skoci --bind 127.0.0.1:8787
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF

$SUDO systemctl daemon-reload
$SUDO systemctl enable skoci >/dev/null 2>&1
$SUDO systemctl restart skoci

# Firewall Configuration (UFW)
if command -v ufw >/dev/null 2>&1 && $SUDO ufw status 2>/dev/null | grep -qw "active"; then
  echo "Mengonfigurasi UFW firewall..."
  $SUDO ufw allow 80/tcp >/dev/null 2>&1 || true
  $SUDO ufw allow 443/tcp >/dev/null 2>&1 || true
  if $SUDO ufw status 2>/dev/null | grep -q "8787"; then
    echo -e "${YELLOW}Port 8787 tidak lagi dipublikasikan secara eksternal (hanya 127.0.0.1).${NC}"
    echo -e "Hapus aturan lama: ${BOLD}sudo ufw delete allow 8787/tcp${NC}"
  fi
fi
echo -e "${YELLOW}PENTING: Pastikan port 80 dan 443 juga dibuka pada cloud firewall (misal Sumopod panel, AWS Security Group, Hetzner Firewall).${NC}"

# Health Check Wait (up to 20s)
echo "Menunggu layanan Skoci siap..."
HEALTHY=0
for i in $(seq 1 20); do
  if curl -fsS http://127.0.0.1:8787/api/health >/dev/null 2>&1; then
    HEALTHY=1
    break
  fi
  sleep 1
done

if [ "$HEALTHY" -ne 1 ]; then
  echo -e "${RED}❌ Skoci gagal merespons di http://127.0.0.1:8787/api/health setelah 20 detik.${NC}"
  echo -e "Log service terakhir:"
  $SUDO journalctl -u skoci -n 50 --no-pager || true
  exit 1
fi

API_HTTPS_OK=0
if [ "$TLS_MODE" = "acme" ]; then
  echo "Menunggu sertifikat HTTPS untuk skoci.${BASE_DOMAIN} (maks 90 detik)..."
  for i in $(seq 1 45); do
    if curl -fsS --max-time 5 "https://skoci.${BASE_DOMAIN}/api/health" >/dev/null 2>&1; then
      API_HTTPS_OK=1
      break
    fi
    sleep 2
  done
  if [ "$API_HTTPS_OK" -ne 1 ]; then
    echo -e "${YELLOW}⚠️ HTTPS untuk skoci.${BASE_DOMAIN} belum aktif.${NC}"
    echo -e "${YELLOW}   Penyebab umum: port 80/443 belum dibuka di firewall cloud, port dipakai proses lain, atau limit Let's Encrypt untuk sslip.io.${NC}"
    echo -e "${YELLOW}   Skoci akan terus mencoba di latar belakang. Cek dengan: sudo journalctl -u skoci -n 50 --no-pager${NC}"
    echo -e "${YELLOW}   Sementara itu gunakan SSH tunnel (lihat di bawah). Domain sendiri: jalankan ulang installer dengan --domain <domain> --email <email>.${NC}"
  fi
fi

echo -e "\n${GREEN}${BOLD}==============================================================================${NC}"
echo -e "${GREEN}${BOLD}🎉 SKOCI SERVER BERHASIL DIPASANG!${NC}"
echo -e "${GREEN}${BOLD}==============================================================================${NC}"
if [ "$API_HTTPS_OK" = "1" ]; then
  echo -e "• ${BOLD}Dashboard & API:${NC}      https://skoci.${BASE_DOMAIN}"
else
  echo -e "• ${BOLD}Dashboard & API (SSH):${NC} Jalankan tunnel SSH:"
  echo -e "                          ${BOLD}ssh -L 8787:127.0.0.1:8787 ${INVOKING_USER}@${PUBLIC_IP}${NC}"
  echo -e "                          lalu buka di browser: ${BOLD}http://localhost:8787${NC}"
fi
echo -e "• ${BOLD}Base Domain Aplikasi:${NC} *.${BASE_DOMAIN}"
echo -e "• ${BOLD}Master Token:${NC}         Tersimpan aman di /etc/skoci/master_token"
echo -e "                          (Baca dengan: ${BOLD}sudo cat /etc/skoci/master_token${NC})"
echo -e "\n${BOLD}Langkah Selanjutnya:${NC}"
echo -e "1. Buat token untuk laptop/agen Anda (di server ini):"
echo -e "   ${BOLD}sudo skoci token create --app <nama-app> --name laptop${NC}"
if [ -n "$INVOKING_USER" ] && [ "$INVOKING_USER" != "root" ]; then
  echo -e "   (Jika baru ditambahkan ke grup skoci, lakukan re-login/logout terlebih dahulu agar bisa menjalankan skoci tanpa sudo)"
fi
echo -e "\n2. Pada laptop/komputer pengembang Anda:"
if [ "$API_HTTPS_OK" = "1" ]; then
  echo -e "   ${BOLD}skoci connect --server https://skoci.${BASE_DOMAIN} --token <token>${NC}"
else
  echo -e "   Buka tunnel di terminal terpisah (biarkan tetap jalan):"
  echo -e "   ${BOLD}ssh -N -L 8787:127.0.0.1:8787 ${INVOKING_USER}@${PUBLIC_IP}${NC}"
  echo -e "   ${BOLD}skoci connect --server http://127.0.0.1:8787 --token <token>${NC}"
fi
echo -e "   Masuk ke repositori proyek:"
echo -e "   ${BOLD}skoci init${NC}"
echo -e "   ${BOLD}skoci deploy${NC}"
echo -e "\n${GREEN}Selesai! Aplikasi Anda siap dideploy.${NC}"
