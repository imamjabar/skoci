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
# Probe real sudo usability: passwordless (sudo -n true) or interactive TTY.
SUDO=""
if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
elif command -v sudo >/dev/null 2>&1; then
  if sudo -n true 2>/dev/null; then
    SUDO="sudo"
  elif [ -t 0 ]; then
    SUDO="sudo"
  fi
fi

INVOKING_USER="${SUDO_USER:-${USER:-$(id -un 2>/dev/null || echo "user")}}"

# 1. Parse Command Line Arguments
CLI_DOMAIN=""
CLI_EMAIL=""
NO_TLS=0
CLI_VERSION="latest"
CLIENT_ONLY=0
MAGIC_DNS=0
HARDEN_SSH=0
NO_UFW=0
CLI_SSH_PORT=""
ALLOW_MAGIC_DNS_PRODUCTION=0
STRICT_APPROVALS=0
while [ $# -gt 0 ]; do
  case "$1" in
    --domain)
      if [ -n "${2:-}" ]; then
        CLI_DOMAIN="$2"
        shift 2
      else
        echo -e "${RED}Error: --domain requires a value${NC}" >&2
        exit 1
      fi
      ;;
    --email)
      if [ -n "${2:-}" ]; then
        CLI_EMAIL="$2"
        shift 2
      else
        echo -e "${RED}Error: --email requires a value${NC}" >&2
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
        echo -e "${RED}Error: --version requires a value${NC}" >&2
        exit 1
      fi
      ;;
    --client-only)
      CLIENT_ONLY=1
      shift
      ;;
    --magic-dns)
      MAGIC_DNS=1
      shift
      ;;
    --harden-ssh)
      HARDEN_SSH=1
      shift
      ;;
    --no-ufw)
      NO_UFW=1
      shift
      ;;
    --ssh-port)
      if [ -n "${2:-}" ]; then
        CLI_SSH_PORT="$2"
        shift 2
      else
        echo -e "${RED}Error: --ssh-port requires a value${NC}" >&2
        exit 1
      fi
      ;;
    --allow-magic-dns-production)
      ALLOW_MAGIC_DNS_PRODUCTION=1
      shift
      ;;
    --strict-approvals)
      STRICT_APPROVALS=1
      shift
      ;;
    -h|--help)
      echo "Usage: install.sh [options]"
      echo ""
      echo "Options:"
      echo "  --domain <domain>   Base domain you own, used for app hostnames and TLS"
      echo "  --email <email>     Contact address for Let's Encrypt"
      echo "  --version <tag>     Skoci version tag to install (default: latest)"
      echo "  --magic-dns         Enable sslip.io preview hostnames"
      echo "  --no-tls            Disable TLS (HTTP only)"
      echo "  --client-only       Install Skoci CLI client only"
      echo "  --harden-ssh        Harden SSH daemon (disable password auth)"
      echo "  --no-ufw            Skip UFW firewall configuration"
      echo "  --ssh-port <port>   SSH port for firewall rules and hardening"
      echo "  --allow-magic-dns-production  Allow magic-DNS hostnames in production"
      echo "  --strict-approvals  Require manual approval for all production promotions"
      echo "  -h, --help          Show this help message"
      exit 0
      ;;
    *)
      echo -e "${RED}Error: Unknown option: $1${NC}" >&2
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
    echo -e "${RED}Error: Operating system '${OS}' is not supported.${NC}" >&2
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
    echo -e "${RED}Error: Architecture '${ARCH}' is not supported (only x86_64 and aarch64).${NC}" >&2
    exit 1
    ;;
esac

echo -e "${GREEN}✓ Detected:${NC} ${OS_NAME} (${ARCH_NAME})"

if [ "$CLIENT_ONLY" != "1" ]; then
  if [ "$(id -u)" -ne 0 ] && [ -z "$SUDO" ]; then
    echo -e "${RED}Error: Linux server installation requires root or sudo privileges.${NC}" >&2
    echo -e "${RED}Neither root access nor a usable sudo environment was detected.${NC}" >&2
    exit 1
  fi
fi

# Helpers
verify_checksum() {
  local sum_file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "$sum_file"
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "$sum_file"
  else
    echo -e "${RED}Error: Neither sha256sum nor shasum found; cannot verify release integrity.${NC}" >&2
    return 1
  fi
}

TMP_DIR="$(mktemp -d /tmp/skoci-install.XXXXXX)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

build_from_source() {
  local target_dest="$1"
  echo -e "\n${YELLOW}Compiling Skoci from source (this takes ~4 minutes)...${NC}"

  # Check local binary if running from local repo
  local script_file="${BASH_SOURCE[0]:-}"
  local script_dir=""
  if [ -n "$script_file" ] && [ -f "$script_file" ]; then
    script_dir="$(cd "$(dirname "$script_file")" 2>/dev/null && pwd || echo "")"
  fi

  if [ -n "$script_dir" ] && [ -f "${script_dir}/target/release/skoci" ]; then
    echo "Using local binary from ${script_dir}/target/release/skoci"
    cp "${script_dir}/target/release/skoci" "$target_dest"
    return 0
  fi

  if [ "$OS_NAME" = "linux" ] && command -v apt-get >/dev/null 2>&1; then
    echo "Installing build dependencies via apt-get..."
    $SUDO apt-get update -y
    $SUDO apt-get install -y build-essential pkg-config git curl
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo "Installing Rust toolchain (minimal profile)..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env" 2>/dev/null || export PATH="$HOME/.cargo/bin:$PATH"
  fi

  local src_dir
  if [ -n "$script_dir" ] && [ -f "${script_dir}/Cargo.toml" ]; then
    src_dir="$script_dir"
  else
    src_dir="${TMP_DIR}/repo"
    echo "Cloning Skoci repository source code (${TAG})..."
    git clone --depth 1 --branch "$TAG" https://github.com/imamjabar/skoci.git "$src_dir"
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
    echo -e "${RED}Error: Failed to determine latest release tag from GitHub.${NC}" >&2
    echo "Check your internet connection or specify a version explicitly: --version <tag>" >&2
    exit 1
  fi
fi

DOWNLOAD_URL="https://github.com/imamjabar/skoci/releases/download/${TAG}/${ASSET_NAME}"
SHA_URL="https://github.com/imamjabar/skoci/releases/download/${TAG}/${ASSET_NAME}.sha256"

echo -e "\n${BOLD}[1/4] Preparing Skoci binary...${NC}"
echo "Downloading ${ASSET_NAME} (${TAG})..."

ASSET_HTTP_CODE="$(curl -sSL -w "%{http_code}" -o "${TMP_DIR}/${ASSET_NAME}" "$DOWNLOAD_URL" || true)"

if [ "$ASSET_HTTP_CODE" = "404" ]; then
  echo -e "${YELLOW}Prebuilt binary not found on GitHub Releases (HTTP 404). Falling back to building from source...${NC}"
  build_from_source "${TMP_DIR}/${ASSET_NAME}"
elif [ "$ASSET_HTTP_CODE" = "200" ]; then
  SHA_HTTP_CODE="$(curl -sSL -w "%{http_code}" -o "${TMP_DIR}/${ASSET_NAME}.sha256" "$SHA_URL" || true)"
  if [ "$SHA_HTTP_CODE" != "200" ]; then
    echo -e "${RED}Error: Failed to download checksum file for ${ASSET_NAME} (HTTP ${SHA_HTTP_CODE}).${NC}" >&2
    exit 1
  fi

  (
    cd "$TMP_DIR"
    if ! verify_checksum "${ASSET_NAME}.sha256"; then
      echo -e "${RED}Error: SHA256 verification failed for ${ASSET_NAME}. Aborting.${NC}" >&2
      exit 1
    fi
  )
  echo -e "${GREEN}✓ SHA256 verification succeeded.${NC}"
else
  echo -e "${RED}Error: Failed to download ${ASSET_NAME} (HTTP ${ASSET_HTTP_CODE}).${NC}" >&2
  exit 1
fi

# 4. Install Binary for Client-Only (macOS or --client-only)
if [ "$CLIENT_ONLY" = "1" ]; then
  INSTALL_DIR="/usr/local/bin"
  if [ "$(id -u)" -ne 0 ] && { [ -z "$SUDO" ] || [ ! -w "/usr/local/bin" ]; }; then
    INSTALL_DIR="${HOME}/.local/bin"
  fi

  if [ "$INSTALL_DIR" = "${HOME}/.local/bin" ]; then
    mkdir -p "$INSTALL_DIR"
    install -m 0755 "${TMP_DIR}/${ASSET_NAME}" "${INSTALL_DIR}/skoci.new"
    mv -f "${INSTALL_DIR}/skoci.new" "${INSTALL_DIR}/skoci"
    case ":${PATH:-}:" in
      *":${INSTALL_DIR}:"*) ;;
      *)
        echo -e "\n${YELLOW}Note:${NC} ${INSTALL_DIR} is not in your PATH."
        echo -e "Add it to your PATH by adding this line to your ~/.bashrc or ~/.zshrc:"
        echo -e "  ${BOLD}export PATH=\"\$HOME/.local/bin:\$PATH\"${NC}"
        ;;
    esac
  elif [ -w "/usr/local/bin" ]; then
    mkdir -p "$INSTALL_DIR"
    install -m 0755 "${TMP_DIR}/${ASSET_NAME}" "${INSTALL_DIR}/skoci.new"
    mv -f "${INSTALL_DIR}/skoci.new" "${INSTALL_DIR}/skoci"
  else
    $SUDO mkdir -p "$INSTALL_DIR"
    $SUDO install -m 0755 "${TMP_DIR}/${ASSET_NAME}" "${INSTALL_DIR}/skoci.new"
    $SUDO mv -f "${INSTALL_DIR}/skoci.new" "${INSTALL_DIR}/skoci"
  fi

  echo -e "\n${GREEN}${BOLD}==============================================================================${NC}"
  echo -e "${GREEN}${BOLD}🎉 SKOCI CLIENT INSTALLED SUCCESSFULLY!${NC}"
  echo -e "${GREEN}${BOLD}==============================================================================${NC}"
  echo -e "Binary installed at: ${BOLD}${INSTALL_DIR}/skoci${NC}"
  echo -e "\n${BOLD}Next Steps:${NC}"
  echo -e "1. Connect to your Skoci server:"
  echo -e "   ${BOLD}skoci connect --server <server-ip> --token <token> --pin <pin>${NC}"
  echo -e "   (Or with custom domain: ${BOLD}skoci connect --server https://skoci.<base-domain>${NC})"
  echo -e "   Tip: Use ${BOLD}skoci server setup --host <ip> --user <user>${NC} for automatic zero-SSH setup."
  echo -e "2. Navigate to your project repository:"
  echo -e "   ${BOLD}skoci init${NC}"
  echo -e "3. Deploy the application:"
  echo -e "   ${BOLD}skoci deploy${NC}"
  exit 0
fi

# 5. Linux Server Installation
INSTALL_DIR="/usr/local/bin"
BIN_TARGET="${INSTALL_DIR}/skoci"
# Write next to the target and rename: overwriting a running executable fails with "Text file busy".
$SUDO install -m 0755 "${TMP_DIR}/${ASSET_NAME}" "${BIN_TARGET}.new"
$SUDO mv -f "${BIN_TARGET}.new" "${BIN_TARGET}"
echo -e "${GREEN}✓ Binary installed at ${BIN_TARGET}${NC}"

# Check / Install Docker Engine
echo -e "\n${BOLD}[2/4] Checking Docker Engine...${NC}"
if ! command -v docker >/dev/null 2>&1; then
  echo "Docker Engine not found. Installing Docker via get.docker.com..."
  curl -fsSL https://get.docker.com | $SUDO sh
fi

$SUDO systemctl start docker || true
$SUDO systemctl enable docker || true
echo -e "${GREEN}✓ Docker Engine is active.${NC}"

# Create System User & Groups
if ! id -u skoci >/dev/null 2>&1; then
  echo "Creating system user 'skoci'..."
  $SUDO useradd --system --home /var/lib/skoci --shell /usr/sbin/nologin skoci
fi
$SUDO usermod -aG docker skoci

# Setup Directories & Permissions
$SUDO mkdir -p /etc/skoci /var/lib/skoci /var/lib/skoci/backups
$SUDO chown root:root /etc/skoci
$SUDO chmod 0700 /etc/skoci
$SUDO chown -R skoci:skoci /var/lib/skoci
$SUDO chmod 0700 /var/lib/skoci

# Setup Master Token
echo -e "\n${BOLD}[3/4] Configuring Token & Network...${NC}"
MASTER_TOKEN=""
if $SUDO test -f /etc/skoci/master_token; then
  MASTER_TOKEN="$($SUDO cat /etc/skoci/master_token | tr -d '\r\n ')"
fi
if [ -z "$MASTER_TOKEN" ] && $SUDO test -f /etc/skoci/skoci.env; then
  OLD_TOKEN="$($SUDO grep -E '^SKOCI_MASTER_TOKEN=' /etc/skoci/skoci.env | cut -d= -f2- | tr -d '\r\n ' || true)"
  if [ -n "$OLD_TOKEN" ]; then
    MASTER_TOKEN="$OLD_TOKEN"
  fi
fi
if [ -z "$MASTER_TOKEN" ]; then
  HEX="$(openssl rand -hex 24 2>/dev/null || od -vN 24 -An -tx1 /dev/urandom | tr -d ' \n')"
  MASTER_TOKEN="sk_master_${HEX}"
fi

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
  echo -e "${RED}Error: Server public IP not detected. Re-run with --domain <your-domain>.${NC}" >&2
  exit 1
fi

BASE_DOMAIN=""
if [ -n "$CLI_DOMAIN" ]; then
  BASE_DOMAIN="$CLI_DOMAIN"
elif [ "$MAGIC_DNS" = "1" ]; then
  DASHED_IP="$(echo "$PUBLIC_IP" | tr '.' '-')"
  BASE_DOMAIN="${DASHED_IP}.sslip.io"
  echo -e "\n${YELLOW}⚠️ WARNING: Using magic DNS (${BASE_DOMAIN}).${NC}"
  echo -e "${YELLOW}   Magic DNS (sslip.io) is a third-party zone with shared Let's Encrypt rate limits.${NC}"
  echo -e "${YELLOW}   It is suitable for preview only, NOT for production!${NC}\n"
fi

if [ "$NO_TLS" = "1" ]; then
  TLS_MODE="off"
else
  TLS_MODE="acme"
fi

# Write /etc/skoci/skoci.env
ENV_CONTENT="SKOCI_MASTER_TOKEN=${MASTER_TOKEN}"
if [ -n "$BASE_DOMAIN" ]; then
  ENV_CONTENT="${ENV_CONTENT}
SKOCI_BASE_DOMAIN=${BASE_DOMAIN}"
fi
ENV_CONTENT="${ENV_CONTENT}
SKOCI_TLS=${TLS_MODE}
SKOCI_PUBLIC_IP=${PUBLIC_IP}
SKOCI_CONTROL_TLS_BIND=0.0.0.0:8443"
if [ "$ALLOW_MAGIC_DNS_PRODUCTION" = "1" ]; then
  ENV_CONTENT="${ENV_CONTENT}
SKOCI_ALLOW_MAGIC_DNS_PRODUCTION=true"
fi
if [ "$STRICT_APPROVALS" = "1" ]; then
  ENV_CONTENT="${ENV_CONTENT}
SKOCI_REQUIRE_APPROVAL_ALWAYS=true"
fi
if [ -n "$CLI_EMAIL" ]; then
  ENV_CONTENT="${ENV_CONTENT}
SKOCI_ACME_EMAIL=${CLI_EMAIL}"
fi

echo "$ENV_CONTENT" | $SUDO tee /etc/skoci/skoci.env >/dev/null
$SUDO chown root:root /etc/skoci/skoci.env
$SUDO chmod 0600 /etc/skoci/skoci.env
$SUDO rm -f /etc/skoci/master_token

# Port Conflict Check
if command -v ss >/dev/null 2>&1; then
  LISTENERS="$($SUDO ss -ltnp 2>/dev/null | grep -E ':(80|443)\b' | grep -v 'skoci' || true)"
  if [ -n "$LISTENERS" ]; then
    echo -e "\n${YELLOW}⚠️ WARNING: Port 80 or 443 is in use by another process:${NC}"
    echo "$LISTENERS"
    if echo "$LISTENERS" | grep -q "docker-proxy"; then
      CONTAINERS_80="$($SUDO docker ps --filter publish=80 --format '{{.Names}}' 2>/dev/null || true)"
      CONTAINERS_443="$($SUDO docker ps --filter publish=443 --format '{{.Names}}' 2>/dev/null || true)"
      ALL_CONFLICT="$(echo -e "${CONTAINERS_80}\n${CONTAINERS_443}" | grep -v '^$' | sort -u || true)"
      if [ -n "$ALL_CONFLICT" ]; then
        echo -e "\n${YELLOW}The following Docker containers publish port 80/443:${NC}"
        for cname in $ALL_CONFLICT; do
          echo -e "  - ${BOLD}$cname${NC} (Free port with: ${BOLD}sudo docker rm -f $cname${NC})"
        done
      fi
    fi
    echo -e "${YELLOW}Skoci proxy cannot bind to port 80/443 until the processes above are stopped.${NC}"
    echo -e "${YELLOW}Installation will continue.${NC}\n"
  fi
fi

# 6. Setup Systemd Service
echo -e "\n${BOLD}[4/4] Enabling Systemd Service...${NC}"
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
UMask=0077
LimitNOFILE=65536
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
LockPersonality=true
RestrictNamespaces=true
ReadWritePaths=/var/lib/skoci
EnvironmentFile=/etc/skoci/skoci.env
ExecStart=/usr/local/bin/skoci server run
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF

$SUDO systemctl daemon-reload
$SUDO systemctl enable skoci >/dev/null 2>&1
$SUDO systemctl restart skoci

# Derive SSH port
SSH_PORT="${CLI_SSH_PORT:-}"
if [ -z "$SSH_PORT" ] && [ -n "${SSH_CONNECTION:-}" ]; then
  read -r _ _ _ detected_port <<< "${SSH_CONNECTION}"
  if [ -n "$detected_port" ]; then
    SSH_PORT="$detected_port"
  fi
fi
if [ -z "$SSH_PORT" ]; then
  SSH_PORT="22"
fi

# Firewall Configuration (UFW)
if [ "$NO_UFW" = "0" ]; then
  if ! command -v ufw >/dev/null 2>&1; then
    echo -e "\n${YELLOW}Notice: ufw is not installed. Skipping firewall configuration.${NC}"
    echo "To configure the firewall manually, run:"
    echo "  sudo apt-get install -y ufw"
    echo "  sudo ufw allow ${SSH_PORT}/tcp"
    echo "  sudo ufw allow 80/tcp"
    echo "  sudo ufw allow 443/tcp"
    echo "  sudo ufw allow 8443/tcp"
    echo "  sudo ufw default deny incoming"
    echo "  sudo ufw default allow outgoing"
    echo -e "  sudo ufw --force enable\n"
  else
    echo "Configuring UFW firewall (SSH port: ${SSH_PORT})..."
    if ! $SUDO ufw allow "${SSH_PORT}/tcp" >/dev/null 2>&1; then
      echo -e "${RED}Could not add the UFW rule for SSH port ${SSH_PORT}; leaving the firewall untouched to avoid locking you out.${NC}" >&2
      echo -e "${YELLOW}Run 'sudo ufw allow ${SSH_PORT}/tcp' manually, then re-run install.sh.${NC}" >&2
    else
      $SUDO ufw allow 80/tcp >/dev/null 2>&1 || true
      $SUDO ufw allow 443/tcp >/dev/null 2>&1 || true
      $SUDO ufw allow 8443/tcp >/dev/null 2>&1 || true
      $SUDO ufw default deny incoming >/dev/null 2>&1 || true
      $SUDO ufw default allow outgoing >/dev/null 2>&1 || true
      $SUDO ufw --force enable >/dev/null 2>&1 || true
      echo -e "${GREEN}✓ UFW firewall configured and enabled.${NC}"
    fi
    if $SUDO ufw status 2>/dev/null | grep -q "8787"; then
      echo -e "${YELLOW}Port 8787 is no longer published externally (127.0.0.1 only).${NC}"
      echo -e "Remove old rule: ${BOLD}sudo ufw delete allow 8787/tcp${NC}"
    fi
  fi
fi

# SSH Hardening
if [ "$HARDEN_SSH" = "1" ]; then
  echo "Hardening SSH configuration..."
  INVOKING_HOME=""
  if [ -n "$INVOKING_USER" ]; then
    INVOKING_HOME="$(getent passwd "$INVOKING_USER" 2>/dev/null | cut -d: -f6 || true)"
  fi
  if [ -z "$INVOKING_HOME" ]; then
    INVOKING_HOME="${HOME:-/root}"
  fi

  AUTH_KEYS="${INVOKING_HOME}/.ssh/authorized_keys"
  if [ ! -f "$AUTH_KEYS" ] || [ ! -s "$AUTH_KEYS" ]; then
    echo -e "\n${YELLOW}⚠️  WARNING: Refusing to harden SSH!${NC}"
    echo -e "${YELLOW}   ${AUTH_KEYS} for user '${INVOKING_USER}' is missing or empty.${NC}"
    echo -e "${YELLOW}   Disabling password authentication without authorized keys would lock you out.${NC}\n"
  elif ! $SUDO grep -Eq '^[[:space:]]*Include[[:space:]]+/etc/ssh/sshd_config\.d/' /etc/ssh/sshd_config 2>/dev/null; then
    echo -e "\n${YELLOW}⚠️  WARNING: Skipping SSH hardening.${NC}"
    echo -e "${YELLOW}   /etc/ssh/sshd_config has no 'Include /etc/ssh/sshd_config.d/*.conf' line, so a drop-in would be ignored.${NC}"
    echo -e "${YELLOW}   Add that Include line (or set 'PasswordAuthentication no' by hand), then re-run with --harden-ssh.${NC}\n"
  else
    $SUDO mkdir -p /etc/ssh/sshd_config.d
    # sshd keeps the FIRST value it reads for a keyword, so this drop-in must sort
    # ahead of distro files such as 50-cloud-init.conf (PasswordAuthentication yes).
    $SUDO rm -f /etc/ssh/sshd_config.d/99-skoci.conf
    $SUDO tee /etc/ssh/sshd_config.d/00-skoci-harden.conf >/dev/null << 'EOF'
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin prohibit-password
EOF
    $SUDO chmod 0644 /etc/ssh/sshd_config.d/00-skoci-harden.conf

    if $SUDO sshd -t && [ "$($SUDO sshd -T 2>/dev/null | grep -c '^passwordauthentication no')" = "1" ]; then
      if systemctl list-unit-files ssh.service >/dev/null 2>&1; then
        $SUDO systemctl reload ssh 2>/dev/null || $SUDO systemctl restart ssh 2>/dev/null || true
      elif systemctl list-unit-files sshd.service >/dev/null 2>&1; then
        $SUDO systemctl reload sshd 2>/dev/null || $SUDO systemctl restart sshd 2>/dev/null || true
      fi
      echo -e "${GREEN}✓ SSH hardened: password login disabled (/etc/ssh/sshd_config.d/00-skoci-harden.conf). Keep your SSH key.${NC}"
    else
      echo -e "${RED}⚠️  sshd rejected the hardened configuration, or another drop-in still enables password login. Reverting to avoid a lockout.${NC}" >&2
      $SUDO rm -f /etc/ssh/sshd_config.d/00-skoci-harden.conf
    fi
  fi
fi
echo -e "${YELLOW}IMPORTANT: Ensure ports 80, 443, and 8443 are also open in your cloud firewall (e.g., Sumopod panel, AWS Security Group, Hetzner Firewall).${NC}"

# Health Check Wait (up to 20s)
echo "Waiting for Skoci service to become ready..."
HEALTHY=0
for _ in $(seq 1 20); do
  if curl -fsS http://127.0.0.1:8787/api/health >/dev/null 2>&1; then
    HEALTHY=1
    break
  fi
  sleep 1
done

if [ "$HEALTHY" -ne 1 ]; then
  echo -e "${RED}❌ Skoci failed to respond at http://127.0.0.1:8787/api/health after 20 seconds.${NC}"
  echo -e "Recent service logs:"
  $SUDO journalctl -u skoci -n 50 --no-pager || true
  exit 1
fi

API_HTTPS_OK=0
if [ -n "$BASE_DOMAIN" ]; then
  if [ "$TLS_MODE" = "acme" ]; then
    echo "Waiting for HTTPS certificate for skoci.${BASE_DOMAIN} (up to 90 seconds)..."
    for _ in $(seq 1 45); do
      if curl -fsS --max-time 5 "https://skoci.${BASE_DOMAIN}/api/health" >/dev/null 2>&1; then
        API_HTTPS_OK=1
        break
      fi
      sleep 2
    done
    if [ "$API_HTTPS_OK" -ne 1 ]; then
      echo -e "${YELLOW}⚠️ HTTPS for skoci.${BASE_DOMAIN} is not active yet.${NC}"
      echo -e "${YELLOW}   Common causes: ports 80/443 not open in cloud firewall, port conflict, or Let's Encrypt rate limits.${NC}"
      echo -e "${YELLOW}   Skoci will keep retrying in the background. Check logs with: sudo journalctl -u skoci -n 50 --no-pager${NC}"
      echo -e "${YELLOW}   In the meantime, use the pinned control endpoint or an SSH tunnel.${NC}"
    fi
  fi
else
  echo "Waiting for pinned control TLS endpoint (up to 20 seconds)..."
  CONTROL_TLS_OK=0
  for _ in $(seq 1 20); do
    # -k is acceptable here because this is a loopback probe for liveness only, not an authentication path.
    if curl -k -fsS --max-time 5 "https://127.0.0.1:8443/api/health" >/dev/null 2>&1; then
      CONTROL_TLS_OK=1
      break
    fi
    sleep 1
  done
  if [ "$CONTROL_TLS_OK" -ne 1 ]; then
    echo -e "${YELLOW}⚠️ Pinned control TLS endpoint did not respond on https://127.0.0.1:8443/api/health.${NC}"
    echo -e "${YELLOW}   Check logs with: sudo journalctl -u skoci -n 50 --no-pager${NC}"
  fi
fi

# Read server TLS key fingerprint
PIN=""
FINGERPRINT_JSON="$($SUDO "$BIN_TARGET" server fingerprint --json 2>/dev/null || true)"
if [ -n "$FINGERPRINT_JSON" ]; then
  PIN="$(echo "$FINGERPRINT_JSON" | grep -o '"spki_pin"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"spki_pin"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/' || true)"
fi
if [ -z "$PIN" ] && $SUDO test -f /var/lib/skoci/tls/control.pin; then
  PIN="$($SUDO cat /var/lib/skoci/tls/control.pin 2>/dev/null | tr -d '\r\n ' || true)"
fi

echo -e "\n${GREEN}${BOLD}==============================================================================${NC}"
echo -e "${GREEN}${BOLD}🎉 SKOCI SERVER INSTALLED SUCCESSFULLY!${NC}"
echo -e "${GREEN}${BOLD}==============================================================================${NC}"
if [ -n "$BASE_DOMAIN" ]; then
  if [ "$API_HTTPS_OK" = "1" ]; then
    echo -e "• ${BOLD}Dashboard & API:${NC}      https://skoci.${BASE_DOMAIN}"
  fi
  echo -e "• ${BOLD}App Base Domain:${NC}      *.${BASE_DOMAIN}"
fi
echo -e "• ${BOLD}Control URL:${NC}          https://${PUBLIC_IP}:8443"
if [ -n "$PIN" ]; then
  echo -e "• ${BOLD}TLS SPKI Pin:${NC}         ${PIN}"
fi
echo -e "• ${BOLD}Master Token:${NC}         Stored securely in /etc/skoci/skoci.env"
echo -e "                          (Read with: ${BOLD}sudo grep '^SKOCI_MASTER_TOKEN=' /etc/skoci/skoci.env | cut -d= -f2-${NC})"
if [ "$ALLOW_MAGIC_DNS_PRODUCTION" = "1" ]; then
  echo -e "• ${YELLOW}Warning:${NC}              Magic-DNS hostnames are allowed in production (--allow-magic-dns-production enabled)"
fi
if [ "$STRICT_APPROVALS" = "1" ]; then
  echo -e "• ${BOLD}Approval Policy:${NC}      Strict approvals enabled (all production promotions require approval)"
fi

if [ -n "$PIN" ]; then
  echo -e "\n${BLUE}${BOLD}┌────────────────────────────────────────────────────────────────────────────┐${NC}"
  echo -e "${BLUE}${BOLD}│ PINNED CONTROL ENDPOINT (Zero Third-Party Trust)                          │${NC}"
  echo -e "${BLUE}${BOLD}├────────────────────────────────────────────────────────────────────────────┤${NC}"
  echo -e "${BLUE}${BOLD}│${NC} Control URL: ${BOLD}https://${PUBLIC_IP}:8443${NC}"
  echo -e "${BLUE}${BOLD}│${NC} SPKI Pin:    ${BOLD}${PIN}${NC}"
  echo -e "${BLUE}${BOLD}│${NC}"
  echo -e "${BLUE}${BOLD}│${NC} This SPKI pin authenticates your server's TLS certificate directly,"
  echo -e "${BLUE}${BOLD}│${NC} making the connection safe against MITM attacks without a domain."
  echo -e "${BLUE}${BOLD}└────────────────────────────────────────────────────────────────────────────┘${NC}"
fi

echo -e "\n${BOLD}Next Steps:${NC}"
echo -e "1. Create a token for your laptop/agent (on this server):"
echo -e "   ${BOLD}sudo skoci token create --app <app> --name laptop${NC}"

echo -e "\n2. On your laptop/developer workstation:"
if [ -n "$PIN" ]; then
  echo -e "   ${BOLD}skoci connect --server ${PUBLIC_IP} --token <token> --pin ${PIN}${NC}"
else
  echo -e "   ${BOLD}skoci connect --server ${PUBLIC_IP} --token <token>${NC}"
fi

echo -e "\n3. Deploy your application:"
echo -e "   Navigate to your project repository:"
echo -e "   ${BOLD}skoci init${NC}"
echo -e "   ${BOLD}skoci deploy${NC}"

echo -e "\n${GREEN}Done! Your app is ready to deploy.${NC}"
