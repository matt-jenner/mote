# Install Mote on Windows

Mote supports 64-bit Windows 10 and 11.

## Install

1. Download `Mote-<version>-windows-x64-setup.exe` from the
   [official release page](https://github.com/matt-jenner/mote/releases).
2. Run the installer.
3. If SmartScreen shows **Windows protected your PC**, select **More info**.
4. Confirm that the app name is Mote, then select **Run anyway**.
5. Open Mote from the Start menu and add a photo folder.

The installer is currently unsigned, which causes the SmartScreen prompt. Only
approve a package downloaded from the official release page or built from a
checkout you trust.

Uninstall Mote through **Settings > Apps > Installed apps**. Uninstalling the
application may retain your local catalogue and generated previews so it does
not silently discard library state.

To build the installer yourself, use the
[Windows build instructions](../developer/building.md#windows).
