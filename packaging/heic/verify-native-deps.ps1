param(
    [string]$Prefix,
    [string]$Binary,
    [ValidateSet('x64')][string]$Arch = 'x64',
    [switch]$NoHeic
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'DLL inspection requires Windows and dumpbin.' }
if (-not $Prefix -and -not $Binary) { throw 'Supply -Prefix or -Binary.' }
Get-Command dumpbin -ErrorAction Stop | Out-Null
$forbidden = 'x265|x264|kvazaar|rav1e|svt.?av1|vvenc|uvg266|heif-enc|enc265|en265_|get_encoder_plugin_'
$libraryDir = if ($Prefix) { Join-Path $Prefix 'bin' } else { Split-Path $Binary }

function Get-Dump([string]$Mode, [string]$File) {
    $output = & dumpbin /nologo $Mode $File 2>&1
    if ($LASTEXITCODE -ne 0) { throw "dumpbin failed for $File" }
    return $output -join "`n"
}
function Inspect-Binary([string]$File, [bool]$NativeLibrary) {
    if (-not (Test-Path -LiteralPath $File -PathType Leaf)) { throw "Missing runtime library: $File" }
    $headers = Get-Dump '/headers' $File
    if ($headers -notmatch '8664 machine \(x64\)') { throw "Missing $Arch architecture: $File" }
    $deps = Get-Dump '/dependents' $File
    $symbols = Get-Dump '/exports' $File
    if ("$deps`n$symbols" -match $forbidden) { throw "Encoder implementation or dependency in $File" }
    if ($NoHeic -and $deps -match '(lib)?heif\.dll|libde265\.dll') { throw "HEIF-disabled binary links a decoder: $File" }
    $dependencies = @([regex]::Matches($deps, '(?im)^\s+([A-Za-z0-9_.-]+\.dll)\s*$') | ForEach-Object { $_.Groups[1].Value })
    if ($dependencies.Count -eq 0) { throw "No DLL dependencies found: $File" }
    foreach ($dependency in $dependencies) {
        if ($dependency -match '^(api-ms-win-|ext-ms-win-)') { continue }
        if (-not (Test-Path (Join-Path $libraryDir $dependency)) -and
            -not (Test-Path (Join-Path "$env:SystemRoot/System32" $dependency))) {
            throw "Missing runtime dependency $dependency for $File"
        }
    }
    if ($NativeLibrary) {
        $bytes = [System.IO.File]::ReadAllBytes($File)
        $strings = [System.Text.Encoding]::ASCII.GetString($bytes)
        if ($strings -match '(?i)[A-Z]:[\\/](Users|a|agent|hostedtoolcache)[\\/]|\.windows-x64\.stage\.') {
            throw "Absolute build path in $File"
        }
    }
    Write-Host $deps
    return $dependencies
}

if ($Prefix) {
    if (-not (Test-Path -LiteralPath $Prefix -PathType Container)) { throw "Missing prefix: $Prefix" }
    $files = @(Get-ChildItem -LiteralPath $Prefix -Recurse -File)
    foreach ($file in $files) {
        if ($file.Extension -in @('.dll', '.exe', '.lib', '.a') -and $file.Name -match "$forbidden|plugin|\.a$") {
            throw "Forbidden native artifact: $($file.FullName)"
        }
        if ($NoHeic -and $file.Name -match '(lib)?heif|libde265') { throw "HEIF-disabled prefix contains $($file.Name)" }
    }
    if (-not $NoHeic) {
        $de265Deps = Inspect-Binary (Join-Path $libraryDir 'libde265.dll') $true
        $heifDeps = Inspect-Binary (Join-Path $libraryDir 'heif.dll') $true
        if ($heifDeps -notcontains 'libde265.dll') { throw 'libheif does not dynamically link libde265' }
        foreach ($library in @(@('heif', 'heif'), @('de265', 'libde265'))) {
            $importName, $runtimeName = $library
            $importLibrary = Join-Path $Prefix "lib/$importName.lib"
            if ((Get-Dump '/headers' $importLibrary) -notmatch "(?i)$runtimeName\.dll") { throw "Not a DLL import library: $importLibrary" }
        }
    }
}
if ($Binary) {
    $deps = Inspect-Binary $Binary $false
    if (-not $NoHeic -and $deps -notcontains 'heif.dll') { throw 'Enabled binary does not dynamically link libheif' }
}
Write-Host 'native dependency inspection passed'
