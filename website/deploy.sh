#!/usr/bin/env bash
set -e

echo "Deploying worldprobe website to Cloudflare Workers..."
npx wrangler deploy
