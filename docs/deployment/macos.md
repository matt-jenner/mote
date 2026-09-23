# Install Mote on macOS

Mote's macOS release is a universal Apple Silicon and Intel app distributed as
`Mote-<version>-macOS.dmg`.

## Install

1. Download the DMG from the project's official GitHub Release.
2. Open it and drag `Mote.app` onto the Applications shortcut.
3. In Applications, Control-click `Mote.app`, choose **Open**, then choose
   **Open** again when macOS asks for confirmation.
4. If **Open** is not offered, try to launch Mote once, then open **System
   Settings > Privacy & Security** and choose **Open Anyway** for Mote.

## Why macOS shows a warning

Mote is open-source hobby software. Its macOS bundle is ad-hoc signed, not
Developer ID signed or notarized through Apple's paid developer programme. The
ad-hoc signature is verified during the release build, but it does not establish
the developer identity that Gatekeeper normally recognises. A first-launch
warning is therefore expected.

Only approve a copy downloaded from the project's official GitHub Release. If
you do not want to approve an ad-hoc-signed binary, build Mote from source using
the macOS instructions in the project [README](../../README.md).
