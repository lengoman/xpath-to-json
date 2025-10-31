#!/bin/bash

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${GREEN}Installing xpath-to-json...${NC}"

# Check if cargo is installed
if ! command -v cargo &> /dev/null; then
    echo -e "${RED}Error: cargo is not installed. Please install Rust and Cargo first.${NC}"
    echo "Visit https://rustup.rs/ for installation instructions."
    exit 1
fi

# Build the release version
echo "Building release version..."
cargo build --release
if [ $? -ne 0 ]; then
    echo -e "${RED}Error: Build failed${NC}"
    exit 1
fi

# Create the bin directory if it doesn't exist
mkdir -p "$HOME/.local/bin"

# Add ~/.local/bin to PATH if it's not already there
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
    echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$HOME/.bashrc"
    echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$HOME/.zshrc"
    export PATH="$HOME/.local/bin:$PATH"
fi

# Copy the binary to the bin directory
BIN_PATH="target/release/xpath-to-json"
INSTALL_PATH="$HOME/.local/bin/xpath-to-json"
echo "Installing xpath-to-json to $INSTALL_PATH..."
cp "$BIN_PATH" "$INSTALL_PATH"
if [ $? -ne 0 ]; then
    echo -e "${RED}Error: Failed to copy binary${NC}"
    exit 1
fi

# Make the binary executable
chmod +x "$INSTALL_PATH"
if [ $? -ne 0 ]; then
    echo -e "${RED}Error: Failed to make binary executable${NC}"
    exit 1
fi

echo -e "${GREEN}Installation successful!${NC}"
echo "You can now use 'xpath-to-json' from the command line."
echo "Example: xpath-to-json --xpath-config config.json --html file.html"
echo "Example: xpath-to-json --xpath-config config.json --html file.html --output results.json"

# Check if shell needs to be restarted
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
    echo -e "\n${GREEN}Note:${NC} Please restart your shell or run:"
    echo "source ~/.bashrc  # if you use bash"
    echo "source ~/.zshrc   # if you use zsh"
fi


