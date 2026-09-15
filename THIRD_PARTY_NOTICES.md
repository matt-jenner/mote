# Third-party notices

Mote's own licence is separate and unchanged. HEIC support uses the following
separately replaceable shared libraries when that build feature is enabled.
Both libraries are distributed under the GNU Lesser General Public License,
version 3 or (at your option) any later version (`LGPL-3.0-or-later`). The full
applicable licence text is installed as `LGPL-3.0-or-later.txt` beside this
notice.

## libheif 1.23.4

Copyright (c) 2017-2020 Struktur AG  
Copyright (c) 2017-2026 Dirk Farin

- Licence: LGPL-3.0-or-later
- Source archive: https://github.com/strukturag/libheif/releases/download/v1.23.4/libheif-1.23.4.tar.gz
- SHA-256: `d0c02b4b0e978f34a1974b6f3eea7975a537bf7a9195ffeea38e7242ff316fdd`
- Mote patch: `packaging/heic/decode-only.cmake`
- Detailed record: `libheif.md`

## libde265 1.1.1

Copyright (c) 2013-2014 Struktur AG  
Copyright (c) 2013-2026 Dirk Farin

Authors named by the release include Dirk Farin and Joachim Bauch.

- Licence: LGPL-3.0-or-later
- Source archive: https://github.com/strukturag/libde265/releases/download/v1.1.1/libde265-1.1.1.tar.gz
- SHA-256: `fd48a927e94ed74fc7ce8829d222b9d8599fcbfe8b6448ba66705babc56ab219`
- Mote patches: none
- Detailed record: `libde265.md`

## Source, modification, and replacement

The verified source archives, the complete Mote patch, exact
build switches, rebuild commands, and instructions for relinking or replacing
the shared libraries are installed together. The patch is
`decode-only.cmake`; the rebuild record is `HEIC-REBUILD.md`. In the matching
Mote source tree they are `packaging/heic/decode-only.cmake` and
`packaging/heic/README.md`.

Mote builds these components for decoding only. It does not build or ship x265,
`heif-enc`, encoder implementations, encoder plugins, or encoder tools. Generic
encoder API declarations exported by libheif remain part of its public ABI and
do not by themselves indicate that an encoder implementation is present.

This notice is a packaging and compliance record, not legal advice or a
determination of patent coverage. Copyright licences and patent rights are
different. Open-source licensing, decoding-only use, and non-commercial use do
not automatically settle patent obligations; those can depend on use and
territory and should be assessed independently where relevant.
