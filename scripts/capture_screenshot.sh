#!/usr/bin/env bash
set -euo pipefail

delay="${1:-16}"

clear
mkdir -p assets
(sleep "${delay}" && hyprshot -m output -o assets/) &
cargo run --release -- --immediate
