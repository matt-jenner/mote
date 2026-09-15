param([ValidateSet('x64')][string]$Arch = 'x64')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $IsWindows) { throw 'Run this builder on Windows in an MSVC developer shell.' }
foreach ($tool in @('cmake', 'cl', 'dumpbin', 'tar.exe')) { Get-Command $tool -ErrorAction Stop | Out-Null }
$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$versions = @{}
Get-Content (Join-Path $PSScriptRoot 'versions.env') | ForEach-Object {
    if ($_ -and -not $_.StartsWith('#')) {
        if ($_ -notmatch '^([A-Z0-9_]+)=([^\s]+)$') { throw "Invalid native pin: $_" }
        $versions[$Matches[1]] = $Matches[2]
    }
}
$nativeRoot = Join-Path $repositoryRoot 'build/heic-native'
$cache = Join-Path $nativeRoot 'cache'
$destination = Join-Path $nativeRoot "windows-$Arch"
$work = Join-Path $nativeRoot ".windows-$Arch.stage.$([guid]::NewGuid().ToString('N'))"
$backup = Join-Path $work 'previous'
$vcpkgStage = Join-Path $work 'vcpkg'
$triplet = "$Arch-windows"
$prefix = Join-Path $vcpkgStage "installed/$triplet"
$committed = $false
$publicationActive = $false
$destinationExisted = Test-Path $destination
New-Item -ItemType Directory -Force $cache, $prefix | Out-Null

function Invoke-Native([string]$Program, [string[]]$Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program failed with exit code $LASTEXITCODE" }
}
function Test-Archive([string]$File, [string]$Sha256) {
    return (Test-Path $File) -and ((Get-FileHash -LiteralPath $File -Algorithm SHA256).Hash.ToLowerInvariant() -eq $Sha256)
}
function Configure-Native([string]$Name, [string[]]$Arguments) {
    $output = & cmake --warn-uninitialized -Werror=dev @Arguments 2>&1
    $status = $LASTEXITCODE
    $output | Out-Host
    $text = $output -join "`n"
    if ($status -ne 0 -or $text -match 'Manually-specified variables were not used|Unknown CMake command|Unknown argument') {
        throw "CMake rejected $Name configuration"
    }
    if ($Name -eq 'libheif' -and ($text -notmatch 'libde265 HEVC decoder\s*: \+ built-in' -or $text -notmatch 'x265 HEVC encoder\s*: - disabled')) {
        throw 'libheif did not configure only the in-process HEVC decoder'
    }
}

try {
    # Only the two pinned archives survive a run. All other artifacts belong
    # to this adjacent staging directory and are removed in finally.
    $allowed = @("$($versions.LIBDE265_SHA256).tar.gz", "$($versions.LIBHEIF_SHA256).tar.gz")
    Get-ChildItem -LiteralPath $cache -Force | Where-Object Name -NotIn $allowed | Remove-Item -Recurse -Force
    foreach ($name in @('LIBDE265', 'LIBHEIF')) {
        $sha = $versions["${name}_SHA256"]
        $url = $versions["${name}_URL"]
        if ($sha -notmatch '^[a-f0-9]{64}$' -or $url -notmatch '^https://') { throw "Invalid $name source pin" }
        $archive = Join-Path $cache "$sha.tar.gz"
        if (-not (Test-Archive $archive $sha)) {
            if (Test-Path $archive) { Remove-Item -LiteralPath $archive -Force }
            $partial = Join-Path $work "$sha.partial"
            Invoke-WebRequest -Uri $url -OutFile $partial
            if (-not (Test-Archive $partial $sha)) { throw "SHA-256 mismatch for $name" }
            Move-Item -LiteralPath $partial -Destination $archive
        }
        $source = Join-Path $work $name
        New-Item -ItemType Directory $source | Out-Null
        Invoke-Native 'tar.exe' @('-xzf', $archive, '-C', $source, '--strip-components=1')
    }
    Invoke-Native 'cmake' @("-DSOURCE_DIR=$(Join-Path $work 'LIBHEIF')", '-P', (Join-Path $PSScriptRoot 'decode-only.cmake'))
    $common = @('-G', 'NMake Makefiles', '-DCMAKE_BUILD_TYPE=Release', "-DCMAKE_INSTALL_PREFIX=$prefix",
        '-DCMAKE_INSTALL_LIBDIR=lib', '-DBUILD_SHARED_LIBS=ON', '-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL',
        "-DCMAKE_C_FLAGS=/pathmap:$work=.", "-DCMAKE_CXX_FLAGS=/pathmap:$work=.")
    $de265Build = Join-Path $work 'de265-build'
    Configure-Native 'libde265' ($common + @('-S', (Join-Path $work 'LIBDE265'), '-B', $de265Build,
        '-DENABLE_DECODER=OFF', '-DENABLE_ENCODER=OFF', '-DENABLE_SDL=OFF', '-DENABLE_SHERLOCK265=OFF',
        '-DENABLE_INTERNAL_DEVELOPMENT_TOOLS=OFF', '-DWITH_FUZZERS=OFF', '-DUSE_IWYU=OFF', '-DFORCE_FULL_VISIBILITY=OFF'))
    Invoke-Native 'cmake' @('--build', $de265Build, '--target', 'de265', '--config', 'Release')
    Invoke-Native 'cmake' @('--install', $de265Build, '--config', 'Release')
    $heifBuild = Join-Path $work 'heif-build'
    Configure-Native 'libheif' ($common + @('-S', (Join-Path $work 'LIBHEIF'), '-B', $heifBuild,
        "-DLIBDE265_INCLUDE_DIR=$prefix/include", "-DLIBDE265_LIBRARY=$prefix/lib/libde265.lib",
        '-DBUILD_TESTING=OFF', '-DBUILD_DOCUMENTATION=OFF', '-DBUILD_DEVELOPMENT_TOOLS=OFF',
        '-DENABLE_COVERAGE=OFF', '-DENABLE_EXPERIMENTAL_FEATURES=OFF', '-DENABLE_PLUGIN_LOADING=OFF',
        '-DENABLE_MULTITHREADING_SUPPORT=ON', '-DENABLE_PARALLEL_TILE_DECODING=ON',
        '-DWITH_LIBDE265=ON', '-DWITH_LIBDE265_PLUGIN=OFF',
        '-DWITH_X265=OFF', '-DWITH_X265_PLUGIN=OFF', '-DWITH_KVAZAAR=OFF', '-DWITH_KVAZAAR_PLUGIN=OFF',
        '-DWITH_UVG266=OFF', '-DWITH_UVG266_PLUGIN=OFF', '-DWITH_VVDEC=OFF', '-DWITH_VVDEC_PLUGIN=OFF',
        '-DWITH_VVENC=OFF', '-DWITH_VVENC_PLUGIN=OFF', '-DWITH_X264=OFF', '-DWITH_X264_PLUGIN=OFF',
        '-DWITH_OpenH264_DECODER=OFF', '-DWITH_OpenH264_DECODER_PLUGIN=OFF',
        '-DWITH_DAV1D=OFF', '-DWITH_DAV1D_PLUGIN=OFF', '-DWITH_AOM_DECODER=OFF', '-DWITH_AOM_DECODER_PLUGIN=OFF',
        '-DWITH_AOM_ENCODER=OFF', '-DWITH_AOM_ENCODER_PLUGIN=OFF', '-DWITH_SvtEnc=OFF', '-DWITH_SvtEnc_PLUGIN=OFF',
        '-DWITH_RAV1E=OFF', '-DWITH_RAV1E_PLUGIN=OFF', '-DWITH_JPEG_DECODER=OFF', '-DWITH_JPEG_DECODER_PLUGIN=OFF',
        '-DWITH_JPEG_ENCODER=OFF', '-DWITH_JPEG_ENCODER_PLUGIN=OFF', '-DWITH_OpenJPEG_DECODER=OFF',
        '-DWITH_OpenJPEG_DECODER_PLUGIN=OFF', '-DWITH_OpenJPEG_ENCODER=OFF', '-DWITH_OpenJPEG_ENCODER_PLUGIN=OFF',
        '-DWITH_FFMPEG_DECODER=OFF', '-DWITH_FFMPEG_DECODER_PLUGIN=OFF',
        '-DWITH_OPENJPH_ENCODER=OFF', '-DWITH_OPENJPH_ENCODER_PLUGIN=OFF', '-DWITH_UNCOMPRESSED_CODEC=OFF',
        '-DWITH_WEBCODECS=OFF', '-DWITH_LIBSHARPYUV=OFF', '-DWITH_LIBSHARPYUV_INTERNAL=OFF',
        '-DWITH_HEADER_COMPRESSION=OFF', '-DWITH_EXAMPLES=OFF', '-DWITH_EXAMPLE_HEIF_THUMB=OFF',
        '-DWITH_EXAMPLE_HEIF_VIEW=OFF', '-DWITH_GDK_PIXBUF=OFF', '-DWITH_FUZZERS=OFF', '-DWITH_REDUCED_VISIBILITY=ON'))
    Invoke-Native 'cmake' @('--build', $heifBuild, '--target', 'heif', '--config', 'Release')
    Invoke-Native 'cmake' @('--install', $heifBuild, '--config', 'Release')

    # libheif-sys on MSVC uses vcpkg-rs, not pkg-config. Register these exact
    # shared builds using its installed/status and info manifests. No vcpkg
    # global installation, package selection, or embedded source is involved.
    New-Item -ItemType File (Join-Path $vcpkgStage '.vcpkg-root') | Out-Null
    $metadata = Join-Path $vcpkgStage 'installed/vcpkg'
    New-Item -ItemType Directory (Join-Path $metadata 'info'), (Join-Path $metadata 'updates') | Out-Null
    $status = @()
    foreach ($port in @(@('libde265', $versions.LIBDE265_VERSION, 'libde265'), @('libheif', $versions.LIBHEIF_VERSION, 'heif'))) {
        $name, $version, $library = $port
        $status += "Package: $name`nVersion: $version`nArchitecture: $triplet`nStatus: install ok installed"
        if ($name -eq 'libheif') { $status[-1] += "`nDepends: libde265" }
        $status[-1] += "`n"
        @("$triplet/lib/$library.lib", "$triplet/bin/$library.dll") |
            Set-Content (Join-Path $metadata "info/${name}_${version}_${triplet}.list") -Encoding utf8
    }
    $status | Set-Content (Join-Path $metadata 'status') -Encoding utf8
    # CMake exports contain the temporary libde265 import path. They are not
    # used by Cargo. Omit them and make pkg-config metadata relocatable.
    Remove-Item -LiteralPath (Join-Path $prefix 'lib/cmake') -Recurse -Force
    Get-ChildItem (Join-Path $prefix 'lib/pkgconfig') -Filter '*.pc' | ForEach-Object {
        (Get-Content $_.FullName) -replace '^prefix=.*', 'prefix=${pcfiledir}/../..' | Set-Content $_.FullName -Encoding utf8
    }
    & (Join-Path $PSScriptRoot 'verify-native-deps.ps1') -Prefix $prefix -Arch $Arch
    $probe = Join-Path $work 'verify-decoder.exe'
    Invoke-Native 'cl' @('/nologo', '/MD', "/I$prefix/include", (Join-Path $PSScriptRoot 'verify-decoder.c'),
        "/Fe:$probe", "/Fo:$(Join-Path $work 'verify-decoder.obj')", '/link', "/LIBPATH:$prefix/lib", 'heif.lib', 'libde265.lib')
    $savedPath = $env:PATH
    try {
        $env:PATH = "$prefix/bin;$env:PATH"
        Invoke-Native $probe @($versions.LIBHEIF_VERSION, $versions.LIBDE265_VERSION)
    } finally {
        $env:PATH = $savedPath
    }
    $publicationActive = $true
    if ($destinationExisted) { Move-Item -LiteralPath $destination -Destination $backup }
    Move-Item -LiteralPath $vcpkgStage -Destination $destination
    $committed = $true
} finally {
    if (-not $committed -and $publicationActive) {
        if (Test-Path $backup) {
            if (Test-Path $destination) { Remove-Item -LiteralPath $destination -Recurse -Force }
            Move-Item -LiteralPath $backup -Destination $destination
        } elseif (-not $destinationExisted -and (Test-Path $destination)) {
            Remove-Item -LiteralPath $destination -Recurse -Force
        }
    }
    if (Test-Path $work) { Remove-Item -LiteralPath $work -Recurse -Force }
}
$env:VCPKG_ROOT = $destination
$env:VCPKGRS_TRIPLET = $triplet
$env:VCPKGRS_DYNAMIC = '1'
$env:MOTE_HEIC_PREFIX = Join-Path $destination "installed/$triplet"
$env:PATH = "$env:MOTE_HEIC_PREFIX/bin;$env:PATH"
if ($env:GITHUB_ENV) {
    @("VCPKG_ROOT=$env:VCPKG_ROOT", "VCPKGRS_TRIPLET=$triplet", 'VCPKGRS_DYNAMIC=1', "MOTE_HEIC_PREFIX=$env:MOTE_HEIC_PREFIX") | Add-Content $env:GITHUB_ENV
    "$env:MOTE_HEIC_PREFIX/bin" | Add-Content $env:GITHUB_PATH
}
Write-Host "Native DLL prefix: $env:MOTE_HEIC_PREFIX"
