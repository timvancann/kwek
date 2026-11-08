#!/usr/bin/env bash
# Fetches the on-device search model (multilingual-e5-small, int8 ONNX, ~135 MB) into
# models/, pinned to one Hugging Face revision and checked against models/*.sha256.
# Does nothing when the files are already there and intact.
set -euo pipefail
cd "$(dirname "$0")/.."
rev=761b726dd34fb83930e26aab4e9ac3899aa1fa78
dir=models/multilingual-e5-small
mkdir -p "$dir"
cd "$dir"
if shasum -a 256 --status -c ../multilingual-e5-small.sha256 2>/dev/null; then exit 0; fi
for f in onnx/model_quantized.onnx tokenizer.json config.json special_tokens_map.json tokenizer_config.json; do
    curl -fsSL --retry 5 --retry-all-errors -o "$(basename "$f")" \
        "https://huggingface.co/Xenova/multilingual-e5-small/resolve/$rev/$f"
done
shasum -a 256 -c ../multilingual-e5-small.sha256
