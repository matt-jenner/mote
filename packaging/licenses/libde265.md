# libde265 1.1.1

Copyright (c) 2013-2014 Struktur AG  
Copyright (c) 2013-2026 Dirk Farin

The release's `AUTHORS` file names Dirk Farin and Joachim Bauch. The library is
distributed under `LGPL-3.0-or-later`. See `LGPL-3.0-or-later.txt` for the
complete applicable licence text.

## Corresponding source

- Archive: https://github.com/strukturag/libde265/releases/download/v1.1.1/libde265-1.1.1.tar.gz
- SHA-256: `fd48a927e94ed74fc7ce8829d222b9d8599fcbfe8b6448ba66705babc56ab219`
- Patches applied: none

## Material build switches

The complete self-contained inventory is in `HEIC-REBUILD.md`, installed beside
this record. The executable copies are in `packaging/heic/build-unix.sh`,
`packaging/heic/build-windows.ps1`, and the enabled Flatpak manifest:

```text
-DBUILD_SHARED_LIBS=ON
-DENABLE_DECODER=OFF
-DENABLE_ENCODER=OFF
-DENABLE_SDL=OFF
-DENABLE_SHERLOCK265=OFF
-DENABLE_INTERNAL_DEVELOPMENT_TOOLS=OFF
```

`ENABLE_DECODER=OFF` disables the standalone `dec265` application, not the
library decoder API. Mote dynamically links the resulting libde265 shared
library through libheif.
