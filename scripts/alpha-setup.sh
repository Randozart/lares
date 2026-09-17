#!/usr/bin/env bash
# One-time setup for the local inference box (Alienware Alpha R2).
#
# Installs llama.cpp with Vulkan, downloads a grounded vision model
# (Qwen2.5-VL 3B Instruct, Q4 + mmproj), and installs a systemd user unit
# serving the OpenAI chat-completions API on :8866.
#
# Run as the user who will own the service. NOT run automatically; review
# before executing.
set -euo pipefail

LLAMA_DIR="${LLAMA_DIR:-$HOME/tools/llama.cpp}"
MODELS_DIR="${MODELS_DIR:-$HOME/models/qwen2.5-vl-3b}"
MODEL_URL="${MODEL_URL:-https://huggingface.co/Qwen/Qwen2.5-VL-3B-Instruct-GGUF/resolve/main/qwen2.5-vl-3b-instruct-q4_k_m.gguf}"
MMproj_URL="${MMproj_URL:-https://huggingface.co/Qwen/Qwen2.5-VL-3B-Instruct-GGUF/resolve/main/mmproj-Qwen2.5-VL-3B-Instruct-f16.gguf}"
PORT="${PORT:-8866}"

echo "==> Installing build dependencies (sudo needed)"
sudo apt-get update
sudo apt-get install -y build-essential cmake git libvulkan-dev vulkan-tools curl

echo "==> Cloning llama.cpp into $LLAMA_DIR"
if [ -d "$LLAMA_DIR/.git" ]; then
    git -C "$LLAMA_DIR" pull --ff-only
else
    git clone --depth 1 https://github.com/ggml-org/llama.cpp.git "$LLAMA_DIR"
fi

echo "==> Building llama-server (Vulkan backend)"
cmake -B "$LLAMA_DIR/build" -S "$LLAMA_DIR" -DGGML_VULKAN=ON -DLLAMA_CURL=ON
cmake --build "$LLAMA_DIR/build" --target llama-server -j"$(nproc)"

echo "==> Downloading model + mmproj into $MODELS_DIR"
mkdir -p "$MODELS_DIR"
curl -fL --retry 3 -o "$MODELS_DIR/model.gguf" "$MODEL_URL"
curl -fL --retry 3 -o "$MODELS_DIR/mmproj.gguf" "$MMproj_URL"

echo "==> Installing systemd user unit lares-llama.service"
mkdir -p "$HOME/.config/systemd/user"
cat > "$HOME/.config/systemd/user/lares-llama.service" << UNIT
[Unit]
Description=Lares local vision inference (llama-server)
After=network.target

[Service]
ExecStart=$LLAMA_DIR/build/bin/llama-server \\
    --host 0.0.0.0 --port $PORT \\
    -m $MODELS_DIR/model.gguf \\
    --mmproj $MODELS_DIR/mmproj.gguf \\
    -c 8192 -ngl 99
Restart=on-failure

[Install]
WantedBy=default.target
UNIT

systemctl --user daemon-reload
systemctl --user enable --now lares-llama.service

echo "==> Done. Verify with:"
echo "    systemctl --user status lares-llama"
echo "    curl -s localhost:$PORT/v1/health"
echo ""
echo "Then point Lares at it:"
echo "    LARES_ENGINE=local LARES_LOCAL_ENDPOINT=http://localhost:$PORT"
