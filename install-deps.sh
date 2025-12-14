#!/bin/bash
# Script to install required system dependencies for building cvmsback

echo "Installing build tools (required for vendored OpenSSL)..."
sudo apt-get update
sudo apt-get install -y build-essential pkg-config

echo ""
echo "Installation complete!"
echo ""
echo "Note: The project uses vendored OpenSSL, so you don't need libssl-dev."
echo "However, you still need build tools (gcc, make, etc.) to compile OpenSSL from source."
echo ""
echo "You can now run: cargo build"
