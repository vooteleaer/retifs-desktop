# Builds the Android app (a debug APK): the core's JNI library and retifsd
# for each ABI with the NDK, into app/src/main/jniLibs, then Gradle. No prnsd:
# on a phone Reticulum is Prns's own app, which retifsd joins.
#
#     powershell -ExecutionPolicy Bypass -File android\build-apk.ps1 [-Abis arm64-v8a,x86_64] [-Release]
#
# -Release: shrunk with R8 and signed with the debug key (for testers).
#
# Needs the Android SDK with NDK 27 ($env:ANDROID_HOME, default C:\Android\Sdk),
# JDK 17, rustup targets aarch64-linux-android and x86_64-linux-android, retifs
# and Prns 0.3.8 (retifsd's transport) next to retifs-desktop.
param([string[]]$Abis = @("arm64-v8a", "x86_64"), [switch]$Release)

$ErrorActionPreference = "Stop"
$Here = $PSScriptRoot
$Desktop = (Resolve-Path "$Here\..").Path
$Projects = Split-Path $Desktop
$Retifs = Join-Path $Projects "retifs"
$Sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { "C:\Android\Sdk" }
$Ndk = Get-ChildItem "$Sdk\ndk" | Sort-Object Name | Select-Object -Last 1
$Bin = "$($Ndk.FullName)\toolchains\llvm\prebuilt\windows-x86_64\bin"
$env:CARGO_TARGET_DIR = Join-Path $Desktop "target\android"
$Api = 26

function Invoke-Cargo { & cargo.exe @args; if ($LASTEXITCODE -ne 0) { throw "cargo $args failed" } }

# Only the ABIs asked for go into the APK (an arm64-only one for phones is smaller).
Get-ChildItem "$Here\app\src\main\jniLibs" -Directory -ErrorAction SilentlyContinue |
    Where-Object { $Abis -notcontains $_.Name } | Remove-Item -Recurse -Force

foreach ($abi in $Abis) {
    $triple = @{ "arm64-v8a" = "aarch64-linux-android"; "x86_64" = "x86_64-linux-android" }[$abi]
    $var = $triple.ToUpper().Replace("-", "_")
    $under = $triple.Replace("-", "_")
    Set-Item "env:CARGO_TARGET_${var}_LINKER" "$Bin\$triple$Api-clang.cmd"
    Set-Item "env:CC_$under" "$Bin\$triple$Api-clang.cmd"
    Set-Item "env:CXX_$under" "$Bin\$triple$Api-clang++.cmd"
    Set-Item "env:AR_$under" "$Bin\llvm-ar.exe"
    Write-Host "== $abi ($triple)"
    Invoke-Cargo build --release --target $triple --manifest-path "$Desktop\Cargo.toml" -p retifs-desktop-jni
    Invoke-Cargo build --release --target $triple --manifest-path "$Retifs\Cargo.toml" -p retifs-node
    $out = "$Here\app\src\main\jniLibs\$abi"
    New-Item -ItemType Directory -Force $out | Out-Null
    $built = "$env:CARGO_TARGET_DIR\$triple\release"
    Copy-Item "$built\libretifs_desktop.so" "$out\libretifs_desktop.so"
    # A program, named as a library so Android extracts it where it may run.
    Copy-Item "$built\retifsd" "$out\libretifsd.so"
    Remove-Item "$out\libprnsd.so" -ErrorAction SilentlyContinue # from older builds
    & "$Bin\llvm-strip.exe" "$out\libretifsd.so" "$out\libretifs_desktop.so"
}

Write-Host "== APK"
Push-Location $Here
try {
    & .\gradlew.bat --no-daemon $(if ($Release) { "assembleRelease" } else { "assembleDebug" })
    if ($LASTEXITCODE -ne 0) { throw "gradle failed" }
} finally { Pop-Location }
$apk = if ($Release) { "$Here\app\build\outputs\apk\release\app-release.apk" } else { "$Here\app\build\outputs\apk\debug\app-debug.apk" }
"== $apk ({0:N1} MB)" -f ((Get-Item $apk).Length / 1MB)
