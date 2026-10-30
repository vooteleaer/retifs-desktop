# Builds the Android app (a debug APK): the core's JNI library, retifsd and
# prnsd for each ABI with the NDK, into app/src/main/jniLibs, then Gradle.
#
#     powershell -ExecutionPolicy Bypass -File android\build-apk.ps1 [-Abis arm64-v8a,x86_64]
#
# Needs the Android SDK with NDK 27 ($env:ANDROID_HOME, default C:\Android\Sdk),
# JDK 17, rustup targets aarch64-linux-android and x86_64-linux-android, retifs
# and Prns 0.3.8 next to retifs-desktop.
param([string[]]$Abis = @("arm64-v8a", "x86_64"))

$ErrorActionPreference = "Stop"
$Here = $PSScriptRoot
$Desktop = (Resolve-Path "$Here\..").Path
$Projects = Split-Path $Desktop
$Retifs = Join-Path $Projects "retifs"
$Prns = if ($env:PRNS) { $env:PRNS } else { Join-Path $Projects "Prns-0.3.8" }
$Sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { "C:\Android\Sdk" }
$Ndk = Get-ChildItem "$Sdk\ndk" | Sort-Object Name | Select-Object -Last 1
$Bin = "$($Ndk.FullName)\toolchains\llvm\prebuilt\windows-x86_64\bin"
$env:CARGO_TARGET_DIR = Join-Path $Desktop "target\android"
$Api = 26

function Invoke-Cargo { & cargo.exe @args; if ($LASTEXITCODE -ne 0) { throw "cargo $args failed" } }

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
    Invoke-Cargo build --release --locked --target $triple --manifest-path "$Prns\prnsd\Cargo.toml" --no-default-features --features "tokio-cloud-host,observability"
    $out = "$Here\app\src\main\jniLibs\$abi"
    New-Item -ItemType Directory -Force $out | Out-Null
    $built = "$env:CARGO_TARGET_DIR\$triple\release"
    Copy-Item "$built\libretifs_desktop.so" "$out\libretifs_desktop.so"
    # Programs, named as libraries so Android extracts them where they may run.
    Copy-Item "$built\retifsd" "$out\libretifsd.so"
    Copy-Item "$built\prnsd" "$out\libprnsd.so"
    & "$Bin\llvm-strip.exe" "$out\libretifsd.so" "$out\libprnsd.so" "$out\libretifs_desktop.so"
}

Write-Host "== APK"
Push-Location $Here
try {
    & .\gradlew.bat --no-daemon assembleDebug
    if ($LASTEXITCODE -ne 0) { throw "gradle failed" }
} finally { Pop-Location }
$apk = "$Here\app\build\outputs\apk\debug\app-debug.apk"
"== $apk ({0:N1} MB)" -f ((Get-Item $apk).Length / 1MB)
