# Install Mote

Download release packages from the
[official GitHub Releases page](https://github.com/matt-jenner/mote/releases).
Choose the file for your operating system.

## Windows

Download `Mote-<version>-windows-x64-setup.exe`, run it, then open Mote from the
Start menu. Windows may show a SmartScreen warning because the installer is not
signed. Follow the [Windows installation notes](windows.md) before approving
the first launch.

## macOS

Download `Mote-<version>-macOS.dmg`, open it, and drag Mote into Applications.
The universal app supports Apple Silicon and Intel Macs. macOS will ask you to
approve the first launch because the app is ad-hoc signed rather than
notarized. Follow the [macOS installation notes](macos.md).

## Linux

Download the x86-64 Flatpak.

See the [Linux installation notes](linux.md) for sandbox and network-share
behaviour.

## Hosted

The hosted version is built from this repository and runs with Podman or
Docker. It is intended for a private server where the photo archive is already
mounted. Start with the [hosted installation guide](../hosted/README.md).

## First run

1. Open Mote.
2. Select **Add folder**.
3. Choose a photo folder that your account can read.
4. Leave Mote open while it builds the first set of thumbnails.

Mote reads the selected source and stores its catalogue and previews elsewhere.
It does not alter the selected folder.
