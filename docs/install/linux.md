# Install Mote on Linux

The supported Linux desktop package is an x86-64 Flatpak. It uses the desktop
folder chooser and receives access only to folders you select.

## Install Mote

Download the x86-64 Flatpak from the
[official release page](https://github.com/matt-jenner/mote/releases).

## Network shares

Mount a NAS or network share on the host before opening Mote. Then select its
mounted folder through Mote's folder chooser. The Flatpak does not request
general filesystem or network permissions.

To build a Flatpak locally, use the
[Linux build instructions](../developer/building.md#linux).
