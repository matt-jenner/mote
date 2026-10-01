# Install Mote on macOS

Mote's macOS release is a universal Apple Silicon and Intel application. It
requires macOS 11 or newer, though current macOS releases receive the most
testing.

## Install

1. Download `Mote-<version>-macOS.dmg` from the
   [official release page](https://github.com/matt-jenner/mote/releases).
2. Open the disk image and drag `Mote.app` onto Applications.
3. In Applications, Control-click Mote and select **Open**.
4. Select **Open** again when macOS asks for confirmation.
5. If **Open** is unavailable, try to launch Mote once. Open **System Settings
   > Privacy & Security**, then select **Open Anyway** for Mote.

## Why macOS shows a warning

Mote is an open-source hobby project. Its bundle has an ad-hoc signature, not a
paid Developer ID signature, and Apple has not notarized it. The release build
checks the ad-hoc signature for accidental changes, but Gatekeeper cannot use
it to verify a registered developer identity.

Only approve a copy downloaded from the official release page. You can instead
[build Mote from source](../developer/building.md#macos).
