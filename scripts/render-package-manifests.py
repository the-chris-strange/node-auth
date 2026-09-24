#!/usr/bin/env python3
"""Render Homebrew and Scoop definitions for already-built release archives."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

TARGETS = {
    "linux_x86_64": ("x86_64-unknown-linux-gnu", "tar.gz"),
    "macos_x86_64": ("x86_64-apple-darwin", "tar.gz"),
    "macos_arm64": ("aarch64-apple-darwin", "tar.gz"),
    "windows_x86_64": ("x86_64-pc-windows-msvc", "zip"),
}


def checksum(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as artifact:
        for chunk in iter(lambda: artifact.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repository", required=True, help="GitHub owner/repository")
    parser.add_argument("--version", required=True)
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    base_url = f"https://github.com/{args.repository}/releases/download/v{args.version}"
    releases: dict[str, dict[str, str]] = {}
    for platform, (target, extension) in TARGETS.items():
        filename = f"node-auth-v{args.version}-{target}.{extension}"
        path = args.artifacts / filename
        if not path.is_file():
            raise SystemExit(f"missing release artifact: {path}")
        releases[platform] = {
            "url": f"{base_url}/{filename}",
            "sha256": checksum(path),
        }

    formula = f'''class NodeAuth < Formula
  desc "Authenticate Node package managers to Google Artifact Registry"
  homepage "https://github.com/{args.repository}"
  version "{args.version}"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "{releases["macos_arm64"]["url"]}"
      sha256 "{releases["macos_arm64"]["sha256"]}"
    else
      url "{releases["macos_x86_64"]["url"]}"
      sha256 "{releases["macos_x86_64"]["sha256"]}"
    end
  end

  on_linux do
    depends_on arch: :x86_64
    url "{releases["linux_x86_64"]["url"]}"
    sha256 "{releases["linux_x86_64"]["sha256"]}"
  end

  def install
    bin.install "node-auth"
    bin.install "artifactregistry-auth"
  end

  test do
    assert_match "Usage", shell_output("#{{bin}}/node-auth --help")
  end
end
'''

    scoop = {
        "version": args.version,
        "description": "Authenticate Node package managers to Google Artifact Registry",
        "homepage": f"https://github.com/{args.repository}",
        "license": "Apache-2.0",
        "architecture": {
            "64bit": {
                "url": releases["windows_x86_64"]["url"],
                "hash": releases["windows_x86_64"]["sha256"],
            }
        },
        "bin": ["node-auth.exe", "artifactregistry-auth.exe"],
        "checkver": {"github": f"https://github.com/{args.repository}"},
        "autoupdate": {
            "architecture": {
                "64bit": {
                    "url": f"{base_url.rsplit('/', 1)[0]}/v$version/node-auth-v$version-x86_64-pc-windows-msvc.zip"
                }
            }
        },
    }

    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "node-auth.rb").write_text(formula, encoding="utf-8")
    (args.output / "node-auth.json").write_text(
        json.dumps(scoop, indent=2) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
