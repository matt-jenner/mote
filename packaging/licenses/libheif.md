# libheif 1.23.4

Copyright (c) 2017-2020 Struktur AG
Copyright (c) 2017-2026 Dirk Farin

The library is distributed under `LGPL-3.0-or-later`. See
`LGPL-3.0-or-later.txt` for the complete applicable licence text.

## Corresponding source

- Archive: https://github.com/strukturag/libheif/releases/download/v1.23.4/libheif-1.23.4.tar.gz
- SHA-256: `d0c02b4b0e978f34a1974b6f3eea7975a537bf7a9195ffeea38e7242ff316fdd`
- Patches applied: `packaging/heic/decode-only.cmake`

The patch removes the built-in mask encoder source and registration from the
library build. The complete patch is installed as `decode-only.cmake` beside
this record and is also present at `packaging/heic/decode-only.cmake` in the
matching Mote source tree.

## Material build switches

The complete self-contained inventory is in `HEIC-REBUILD.md`, installed beside
this record. The executable copies are in `packaging/heic/build-unix.sh`,
`packaging/heic/build-windows.ps1`, and the enabled Flatpak manifest. The
decode-only core is:

```text
-DBUILD_SHARED_LIBS=ON
-DBUILD_TESTING=OFF
-DBUILD_DEVELOPMENT_TOOLS=OFF
-DENABLE_PLUGIN_LOADING=OFF
-DWITH_LIBDE265=ON
-DWITH_LIBDE265_PLUGIN=OFF
-DWITH_X265=OFF
-DWITH_X265_PLUGIN=OFF
-DWITH_EXAMPLES=OFF
```

Every other codec implementation and codec plugin is explicitly disabled in
the full inventory. Mote dynamically links the resulting libheif shared
library.
