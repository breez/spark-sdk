#!/bin/bash
set -e

PROTOC_VERSION="30.2"
INSTALL_DIR="$HOME/.local"

echo "Setting up build environment..."

# ============================================
# 1. Install protoc if needed
# ============================================
install_protoc() {
    if command -v protoc &> /dev/null; then
        INSTALLED_VERSION=$(protoc --version | grep -oE '[0-9]+\.[0-9]+' | head -1)
        if [[ "$INSTALLED_VERSION" == "$PROTOC_VERSION" ]]; then
            echo "protoc $PROTOC_VERSION already installed"
            return 0
        fi
    fi

    echo "Installing protoc $PROTOC_VERSION..."

    # Detect OS and architecture
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)

    case "$OS" in
        linux)
            case "$ARCH" in
                x86_64) PROTOC_ARCH="linux-x86_64" ;;
                aarch64) PROTOC_ARCH="linux-aarch_64" ;;
                *) echo "Unsupported architecture: $ARCH"; return 1 ;;
            esac
            ;;
        darwin)
            PROTOC_ARCH="osx-universal_binary"
            ;;
        *)
            echo "Unsupported OS: $OS"
            return 1
            ;;
    esac

    # Download and install protoc
    PROTOC_ZIP="protoc-${PROTOC_VERSION}-${PROTOC_ARCH}.zip"
    PROTOC_URL="https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/${PROTOC_ZIP}"

    mkdir -p "$INSTALL_DIR/bin"
    curl -sLO "$PROTOC_URL"
    unzip -o "$PROTOC_ZIP" -d "$INSTALL_DIR"
    rm "$PROTOC_ZIP"

    echo "protoc installed to $INSTALL_DIR/bin/protoc"
}

# ============================================
# 2. Setup Rust toolchain components
# ============================================
setup_rust() {
    if command -v rustup &> /dev/null; then
        echo "Setting up Rust components..."

        # Ensure clippy is installed
        rustup component add clippy 2>/dev/null || true

        # Ensure rustfmt is installed (for fmt-check)
        rustup component add rustfmt 2>/dev/null || true

        # Add WASM target for wasm-clippy-check
        rustup target add wasm32-unknown-unknown 2>/dev/null || true

        echo "Rust components ready"
    else
        echo "Warning: rustup not found, skipping Rust setup"
    fi
}

# ============================================
# 3. Setup Docker (for integration tests)
# ============================================
setup_docker() {
    if command -v docker &> /dev/null; then
        echo "Docker already installed"
        return 0
    fi

    echo "Installing Docker..."

    # Detect OS
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')

    if [[ "$OS" == "darwin" ]] && command -v brew &> /dev/null; then
        brew install --cask docker
        echo "Note: Docker Desktop installed. You may need to start it manually from Applications."
    elif [[ "$OS" == "linux" ]]; then
        # Install Docker Engine for Ubuntu/Debian
        sudo apt-get update
        sudo apt-get install -y ca-certificates curl gnupg lsb-release

        # Add Docker's official GPG key
        sudo install -m 0755 -d /etc/apt/keyrings
        curl -fsSL https://download.docker.com/linux/ubuntu/gpg | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg
        sudo chmod a+r /etc/apt/keyrings/docker.gpg

        # Set up the repository
        echo \
          "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu \
          $(lsb_release -cs) stable" | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null

        # Install Docker Engine
        sudo apt-get update
        sudo apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin

        # Add current user to docker group to run without sudo
        sudo usermod -aG docker $USER
        echo "Note: You may need to log out and back in for docker group membership to take effect."
    else
        echo "Warning: Could not install Docker. Please install manually from https://docs.docker.com/get-docker/"
        return 1
    fi
}

# ============================================
# Run setup
# ============================================
install_protoc
setup_rust
setup_docker

# ============================================
# Persist PATH for subsequent Claude commands
# ============================================
if [ -n "$CLAUDE_ENV_FILE" ]; then
    echo "export PATH=\"$INSTALL_DIR/bin:\$PATH\"" >> "$CLAUDE_ENV_FILE"
    echo "Environment updated in CLAUDE_ENV_FILE"
fi

# Export for current session
export PATH="$INSTALL_DIR/bin:$PATH"

# Verify installations
echo ""
echo "Build environment ready:"
command -v protoc &> /dev/null && echo "  - protoc $(protoc --version | grep -oE '[0-9]+\.[0-9]+')"
command -v rustc &> /dev/null && echo "  - rustc $(rustc --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')"
command -v cargo &> /dev/null && echo "  - cargo $(cargo --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')"
command -v docker &> /dev/null && echo "  - docker $(docker --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)" || echo "  - docker not found (optional for integration tests)"

exit 0
