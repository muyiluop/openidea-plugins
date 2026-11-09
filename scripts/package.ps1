# Builds the plugin and packages it into a zip that can be shared and installed.
#
#   powershell -ExecutionPolicy Bypass -File scripts/package.ps1
#
# Steps: unit tests of the SDK-free core crate -> wasm component -> staging
# directory in the runtime layout -> dist/<id>-<version>.zip.
#
# The zip is a plain plugin directory: extract it into
#   %APPDATA%\ai.l0ng.openidea\plugins\<anything>\
# (or point `openidea-plugin link` at the extracted folder) and the IDE picks it up
# from Settings > Plugins after a reload.

[CmdletBinding()]
param(
    [switch]$SkipTests,
    # Package the repository contents instead of building a plugin: useful for
    # sharing code with someone who will build it themselves.
    [switch]$SourceOnly,
    # Reuse the dist/plugin.wasm that is already there instead of rebuilding:
    # for repackaging after a manifest/settings change, or on a machine where the
    # Rust SDK cannot be fetched.
    [switch]$UseExistingWasm,
    # Do not also copy the package into release/ (the directory that is committed
    # so it can be downloaded straight from GitHub).
    [switch]$NoRelease,
    [string]$OutputDir = "dist"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    # 1. Version and id come from the manifest, so the archive name cannot drift.
    $manifest = Get-Content "plugin.toml" -Raw
    $id = [regex]::Match($manifest, '(?m)^id\s*=\s*"([^"]+)"').Groups[1].Value
    $version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    if (-not $id -or -not $version) { throw "plugin.toml: cannot read id/version" }
    Write-Host "packaging $id $version"

    $out = Join-Path $root $OutputDir
    New-Item -ItemType Directory -Force -Path $out | Out-Null

    if ($SourceOnly) {
        # `git archive` if the checkout is a repository (it already honours
        # .gitignore); otherwise zip everything except build output.
        $zip = Join-Path $out "$id-$version-source.zip"
        if (Test-Path $zip) { Remove-Item $zip -Force }
        if (Test-Path ".git") {
            git archive --format=zip -v --prefix="$id-$version/" -o $zip HEAD
            if ($LASTEXITCODE -ne 0) { throw "git archive failed" }
        } else {
            $stageRoot = Join-Path $root "target/source-stage"
            if (Test-Path $stageRoot) { Remove-Item $stageRoot -Recurse -Force }
            $stage = Join-Path $stageRoot "$id-$version"
            New-Item -ItemType Directory -Force -Path $stage | Out-Null
            $exclude = @("target", ".git", "dist")
            Get-ChildItem -Force | Where-Object { $exclude -notcontains $_.Name } |
                Copy-Item -Destination $stage -Recurse
            [System.IO.Compression.ZipFile]::CreateFromDirectory(
                $stage, $zip, [System.IO.Compression.CompressionLevel]::Optimal, $true)
        }
        Write-Host ("source package: {0} ({1:N0} bytes)" -f $zip, (Get-Item $zip).Length)
        return
    }

    # 2. Build the component: tests, the wasm target, then the compile itself.
    #    -UseExistingWasm skips all of that and repacks dist/plugin.wasm as is.
    $wasm = Join-Path $root "dist/plugin.wasm"
    if (-not $UseExistingWasm) {
        # 2a. The core crate is testable without the SDK or network access.
        if (-not $SkipTests) {
            Write-Host "cargo test (crates/jvm-params-core)"
            Push-Location "crates/jvm-params-core"
            try {
                cargo test --offline
                if ($LASTEXITCODE -ne 0) { throw "core tests failed" }
            } finally { Pop-Location }
        }

        # 2b. wasm32-wasip2 is required for `cargo build --target wasm32-wasip2`.
        $targets = & rustup target list --installed
        if ($targets -notcontains "wasm32-wasip2") {
            Write-Host "installing the wasm32-wasip2 target"
            rustup target add wasm32-wasip2
            if ($LASTEXITCODE -ne 0) { throw "rustup target add wasm32-wasip2 failed" }
        }

        # 2c. Prefer the official tool (it lands the file in dist/ and runs `check`).
        if (Get-Command openidea-plugin -ErrorAction SilentlyContinue) {
            Write-Host "openidea-plugin build"
            openidea-plugin build
            if ($LASTEXITCODE -ne 0) { throw "openidea-plugin build failed" }
        } else {
            Write-Warning "openidea-plugin not on PATH; falling back to cargo build + copy."
            Write-Warning "Install it with: cargo install --locked --git https://github.com/l0ng-ai/openidea ide-host --bin openidea-plugin"
            cargo build --release --target wasm32-wasip2
            if ($LASTEXITCODE -ne 0) { throw "cargo build --target wasm32-wasip2 failed" }
            $built = "target/wasm32-wasip2/release/openidea_plugin_jvm_params.wasm"
            if (-not (Test-Path $built)) { throw "expected $built, adjust the file name for the crate name" }
            New-Item -ItemType Directory -Force -Path "dist" | Out-Null
            Copy-Item $built $wasm -Force
        }
    } else {
        Write-Host "reusing dist/plugin.wasm (no build)"
    }
    if (-not (Test-Path $wasm)) { throw "no wasm component at dist/plugin.wasm" }
    $size = (Get-Item $wasm).Length
    Write-Host ("wasm component: {0:N0} bytes" -f $size)

    # 5. Stage the runtime layout: exactly what the plugin needs at run time.
    $stageRoot = Join-Path $root "target/package"
    $stage = Join-Path $stageRoot $id
    if (Test-Path $stageRoot) { Remove-Item $stageRoot -Recurse -Force }
    New-Item -ItemType Directory -Force -Path (Join-Path $stage "dist") | Out-Null
    foreach ($f in @("plugin.toml", "settings.schema.json", "icon.svg", "README.md", "LICENSE")) {
        if (Test-Path $f) { Copy-Item $f (Join-Path $stage $f) }
    }
    Copy-Item $wasm (Join-Path $stage "dist/plugin.wasm")

    # 6. Zip the stage directory itself (the archive root is the plugin folder).
    #    ZipFile.CreateFromDirectory writes `/` separators, so the archive also
    #    extracts correctly on POSIX (Compress-Archive would write `\`).
    $zip = Join-Path $out "$id-$version.zip"
    if (Test-Path $zip) { Remove-Item $zip -Force }
    Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction SilentlyContinue
    [System.IO.Compression.ZipFile]::CreateFromDirectory(
        $stage, $zip, [System.IO.Compression.CompressionLevel]::Optimal, $true)
    $zipSize = (Get-Item $zip).Length
    Write-Host ("package: {0} ({1:N0} bytes)" -f $zip, $zipSize)

    # 7. Keep a committed copy in release/ so the package is one click away on
    #    GitHub (dist/ is build output and stays ignored).
    if (-not $NoRelease) {
        $release = Join-Path $root "release"
        New-Item -ItemType Directory -Force -Path $release | Out-Null
        Copy-Item $zip $release -Force
        Write-Host ("release copy: {0}" -f (Join-Path $release (Split-Path -Leaf $zip)))
    }
    Write-Host "share $zip; the recipient extracts it into %APPDATA%\ai.l0ng.openidea\plugins\"
} finally {
    Pop-Location
}
