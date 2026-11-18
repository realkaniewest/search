#!/usr/bin/env bash
set -euo pipefail

# The EC2 Ubuntu mirror stalled during package downloads on hosted runners.
# Use Ubuntu's main archive for those entries and keep the runner's signing configuration.
for source in /etc/apt/sources.list /etc/apt/sources.list.d/*.sources /etc/apt/sources.list.d/*.list; do
  if [[ -f "$source" ]] && grep -q 'us-east-2\.ec2\.archive\.ubuntu\.com/ubuntu' "$source"; then
    sudo sed -i -E 's@https?://us-east-2\.ec2\.archive\.ubuntu\.com/ubuntu@https://archive.ubuntu.com/ubuntu@g' "$source"
  fi
done

sudo apt-get -o Acquire::Retries=3 update
sudo apt-get -o Acquire::Retries=3 install -y "$@"
