# HEIC native decoder source and replacement guide

Mote's default build dynamically links two separately replaceable
`LGPL-3.0-or-later` libraries: libheif 1.23.4 and libde265 1.1.1. Their exact
URLs and SHA-256 checksums are in `versions.env`, mirrored in
`../licenses/libheif.md` and `../licenses/libde265.md`, and locked again in
`../flatpak/generated/source-lock.json`.

## Reproduce the shared libraries

These commands verify cached archives before use, fetch a missing archive from
its pinned URL, build in a disposable staging directory, run the decoder probe,
and publish only the checked shared-library prefix:

```sh
# Linux, on the target architecture
packaging/heic/build-unix.sh --platform linux --arch "$(uname -m)"

# macOS, native or universal
packaging/heic/build-unix.sh --platform macos --arch "$(uname -m)"
packaging/heic/build-unix.sh --platform macos --arch universal
```

From an x64 MSVC developer PowerShell:

```powershell
./packaging/heic/build-windows.ps1 -Arch x64
```

The builders apply the copy of `decode-only.cmake` installed beside this guide
to libheif. No patch is applied to libde265. `compiler-path-maps.cmake` is a
build reproducibility helper and does not modify either upstream source tree.

The complete material libde265 configuration shared by the Unix, Windows, and
Flatpak builders is:

<!-- BEGIN libde265 material switches -->

```text
-DBUILD_SHARED_LIBS=ON
-DENABLE_DECODER=OFF
-DENABLE_ENCODER=OFF
-DENABLE_SDL=OFF
-DENABLE_SHERLOCK265=OFF
-DENABLE_INTERNAL_DEVELOPMENT_TOOLS=OFF
-DWITH_FUZZERS=OFF
-DUSE_IWYU=OFF
-DFORCE_FULL_VISIBILITY=OFF
```

<!-- END libde265 material switches -->

The complete material libheif configuration shared by all three builders is:

<!-- BEGIN libheif material switches -->

```text
-DBUILD_SHARED_LIBS=ON
-DBUILD_TESTING=OFF
-DBUILD_DOCUMENTATION=OFF
-DBUILD_DEVELOPMENT_TOOLS=OFF
-DENABLE_COVERAGE=OFF
-DENABLE_EXPERIMENTAL_FEATURES=OFF
-DENABLE_MULTITHREADING_SUPPORT=ON
-DENABLE_PARALLEL_TILE_DECODING=ON
-DENABLE_PLUGIN_LOADING=OFF
-DWITH_LIBDE265=ON
-DWITH_LIBDE265_PLUGIN=OFF
-DWITH_X265=OFF
-DWITH_X265_PLUGIN=OFF
-DWITH_KVAZAAR=OFF
-DWITH_KVAZAAR_PLUGIN=OFF
-DWITH_UVG266=OFF
-DWITH_UVG266_PLUGIN=OFF
-DWITH_VVDEC=OFF
-DWITH_VVDEC_PLUGIN=OFF
-DWITH_VVENC=OFF
-DWITH_VVENC_PLUGIN=OFF
-DWITH_X264=OFF
-DWITH_X264_PLUGIN=OFF
-DWITH_OpenH264_DECODER=OFF
-DWITH_OpenH264_DECODER_PLUGIN=OFF
-DWITH_DAV1D=OFF
-DWITH_DAV1D_PLUGIN=OFF
-DWITH_AOM_DECODER=OFF
-DWITH_AOM_DECODER_PLUGIN=OFF
-DWITH_AOM_ENCODER=OFF
-DWITH_AOM_ENCODER_PLUGIN=OFF
-DWITH_SvtEnc=OFF
-DWITH_SvtEnc_PLUGIN=OFF
-DWITH_RAV1E=OFF
-DWITH_RAV1E_PLUGIN=OFF
-DWITH_JPEG_DECODER=OFF
-DWITH_JPEG_DECODER_PLUGIN=OFF
-DWITH_JPEG_ENCODER=OFF
-DWITH_JPEG_ENCODER_PLUGIN=OFF
-DWITH_OpenJPEG_DECODER=OFF
-DWITH_OpenJPEG_DECODER_PLUGIN=OFF
-DWITH_OpenJPEG_ENCODER=OFF
-DWITH_OpenJPEG_ENCODER_PLUGIN=OFF
-DWITH_FFMPEG_DECODER=OFF
-DWITH_FFMPEG_DECODER_PLUGIN=OFF
-DWITH_OPENJPH_ENCODER=OFF
-DWITH_OPENJPH_ENCODER_PLUGIN=OFF
-DWITH_UNCOMPRESSED_CODEC=OFF
-DWITH_WEBCODECS=OFF
-DWITH_LIBSHARPYUV=OFF
-DWITH_LIBSHARPYUV_INTERNAL=OFF
-DWITH_HEADER_COMPRESSION=OFF
-DWITH_EXAMPLES=OFF
-DWITH_EXAMPLE_HEIF_THUMB=OFF
-DWITH_EXAMPLE_HEIF_VIEW=OFF
-DWITH_GDK_PIXBUF=OFF
-DWITH_FUZZERS=OFF
-DWITH_REDUCED_VISIBILITY=ON
```

<!-- END libheif material switches -->

Non-material CMake settings select release mode, the source/build/install
paths, the `lib` installation directory where required, the target toolchain,
and parallelism. Windows additionally selects the dynamically linked MSVC
runtime and compiler path-remapping include; Unix supplies the staged libde265
include/library paths. These platform and path settings do not add a codec,
plugin, tool, static-library policy, test suite, example, or development
component.

The result is decode-only: x265 and `heif-enc` are absent, and no encoder
implementation, encoder library, encoder plugin, encoder tool, unrelated codec
plugin, or static library is shipped. libheif can still expose generic exported encoder API symbols
and declarations as part of its stable ABI; the audit distinguishes
those declarations from a linked or shipped encoder implementation.

Run the appropriate inspector after rebuilding:

```sh
packaging/heic/verify-native-deps.sh --prefix build/heic-native/linux-"$(uname -m)"
packaging/heic/verify-native-deps.sh --prefix build/heic-native/macos-"$(uname -m)" --arch "$(uname -m)"
```

```powershell
./packaging/heic/verify-native-deps.ps1 -Prefix build/heic-native/windows-x64/installed/x64-windows -Arch x64
```

## Relink or replace the libraries

Replacement libraries must provide compatible libheif and libde265 ABIs and
architectures. Re-run the platform inspector and the application tests after
replacement. If an ABI or import-library name changes, rebuild/relink Mote
against the replacement rather than renaming an incompatible binary.

### macOS

Replace `Mote.app/Contents/Frameworks/libheif.dylib` and
`Mote.app/Contents/Frameworks/libde265.dylib`. Preserve the install names
`@rpath/libheif.dylib` and `@rpath/libde265.dylib`, provide every architecture
in the app executable, then sign the nested libraries and the app again. To
relink from source, point `MOTE_HEIC_PREFIX` and `PKG_CONFIG_PATH` at the new
prefix before `scripts/desktop-build.sh` runs.

### Windows

The native CI/staging prefix keeps `heif.dll` and `libde265.dll` under
`build/heic-native/windows-x64/installed/x64-windows/bin`, with `heif.lib` and
`de265.lib` under the sibling `lib` directory. Replace the DLLs with compatible
x64 builds, or set `VCPKG_ROOT`, `VCPKGRS_TRIPLET=x64-windows`,
`VCPKGRS_DYNAMIC=1`, and `MOTE_HEIC_PREFIX` to a replacement staging tree and
relink Mote. A distributable Windows application must place both DLLs and the
six compliance files beside its runtime package.

### Linux, Flatpak, and the hosted image

The hosted image installs the versioned shared objects in `/usr/local/lib` and
the compliance set in `/usr/share/licenses/mote`. Replace compatible SONAMEs in
`/usr/local/lib`, run `ldconfig`, and inspect the complete image before use; or
rebuild the image against a changed prefix to relink. The Flatpak installs the
libraries under `/app/lib` and its compliance set under
`/app/share/licenses/io.github.matt_jenner.mote`; rebuild the manifest to relink
or replace these files in a rebuilt Flatpak. Flatpak deployments themselves are
immutable, so modify the build rather than an installed deployment.

## Source and audit record

The source archives plus `decode-only.cmake`, the build scripts, and the build
switches above provide the source and modification record for the two bundled
libraries. Release inspection searches shipped file names, dynamic
dependencies, and implementation/plugin symbols for x265, `heif-enc`, encoder
plugins and unexpected codec plugins. A generic exported encoder API symbol is
not treated as an encoder implementation; an encoder library, plugin, tool,
dependency, registration symbol, or enabled build flag is rejected.

This is a packaging and compliance record, not legal advice or a conclusion
about HEVC patent coverage in a particular territory. Copyright licences and
patent rights are separate. Open-source licensing, decoding-only use, and
non-commercial use do not automatically remove patent obligations. Obtain
appropriate advice for the uses and territories in which a build is supplied.
