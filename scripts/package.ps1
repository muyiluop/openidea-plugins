# Builds one plugin of this repository and packages it into a shareable zip.
#
#   pwsh -File scripts/package.ps1 -Plugin jvm-default-parameters
#   pwsh -File scripts/package.ps1                 # works when there is exactly one plugin
#
# Steps: unit tests of the plugin's SDK-free core crate -> wasm component ->
# staging directory in the runtime layout -> <plugin>/dist/<id>-<version>.zip
# plus a copy in <repo>/release/ (committed, so it can be downloaded from GitHub).
#
# The zip is a plain plugin directory: extract it into
#   %APPDATA%\ai.l0ng.openidea\plugins\<anything>\
# (or point `openidea-plugin link` at the extracted folder) and the IDE picks it
# up from Settings > Plugins after a reload.

[CmdletBinding()]
param(
    # Plugin directory at the repository root (the folder holding plugin.toml).
    [string]$Plugin,
    [switch]$SkipTests,
    # Package the plugin sources instead of a built plugin: useful for sharing
    # code with someone who will build it themselves.
    [switch]$SourceOnly,
    # Reuse <plugin>/dist/plugin.wasm instead of rebuilding: for repackaging
    # after a manifest/settings change, or on a machine where the Rust SDK cannot
    # be fetched.
    [switch]$UseExistingWasm,
    # Do not also copy the package into release/.
    [switch]$NoRelease,
    [string]$OutputDir = "dist"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

# 0. Find the plugin: every repository folder with a plugin.toml is one.
$plugins = @(Get-ChildItem -Path $root -Directory |
    Where-Object { Test-Path (Join-Path $_.FullName "plugin.toml") } |
    Select-Object -ExpandProperty Name |
    Sort-Object)
if (-not $Plugin) {
    if ($plugins.Count -eq 1) {
        $Plugin = $plugins[0]
    } elseif ($plugins.Count -eq 0) {
        throw "no plugin found: a plugin is a repository folder containing plugin.toml"
    } else {
        throw "more than one plugin; pass -Plugin <name>. Available: $($plugins -join ', ')"
    }
}
$dir = Join-Path $root $Plugin
if (-not (Test-Path (Join-Path $dir "plugin.toml"))) {
    throw "${Plugin}: no plugin.toml (available: $($plugins -join ', '))"
}
$license = Join-Path $root "LICENSE"

Push-Location $dir
try {
    # 1. Version and id come from the manifest, so the archive name cannot drift.
    $manifest = Get-Content "plugin.toml" -Raw
    $id = [regex]::Match($manifest, '(?m)^id\s*=\s*"([^"]+)"').Groups[1].Value
    $version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    if (-not $id -or -not $version) { throw "plugin.toml: cannot read id/version" }
    Write-Host "packaging $Plugin -> $id $version"

    $out = Join-Path $dir $OutputDir
    New-Item -ItemType Directory -Force -Path $out | Out-Null

    if ($SourceOnly) {
        # `git archive` of the plugin subtree: it already honours .gitignore and
        # keeps the archive rooted at the plugin folder.
        $zip = Join-Path $out "$id-$version-source.zip"
        if (Test-Path $zip) { Remove-Item $zip -Force }
        if (Test-Path (Join-Path $root ".git")) {
            git -C $root archive --format=zip -v --prefix="$Plugin-$version/" -o $zip "HEAD:$Plugin"
            if ($LASTEXITCODE -ne 0) { throw "git archive failed" }
        } else {
            $stageRoot = Join-Path $dir "target/source-stage"
            if (Test-Path $stageRoot) { Remove-Item $stageRoot -Recurse -Force }
            $stage = Join-Path $stageRoot "$id-$version"
            New-Item -ItemType Directory -Force -Path $stage | Out-Null
            $exclude = @("target", ".git", "dist")
            Get-ChildItem -Force |
                Where-Object { $exclude -notcontains $_.Name } |
                Copy-Item -Destination $stage -Recurse
            if (Test-Path $license) { Copy-Item $license (Join-Path $stage "LICENSE") }
            Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction SilentlyContinue
            [System.IO.Compression.ZipFile]::CreateFromDirectory(
                $stage, $zip, [System.IO.Compression.CompressionLevel]::Optimal, $true)
        }
        Write-Host ("source package: {0} ({1:N0} bytes)" -f $zip, (Get-Item $zip).Length)
        return
    }

    # 2. Build the component: tests, the wasm target, then the compile itself.
    #    -UseExistingWasm skips all of that and repacks dist/plugin.wasm as is.
    $wasm = Join-Path $dir "dist/plugin.wasm"
    if (-not $UseExistingWasm) {
        # 2a. The core crate is testable without the SDK or network access.
        if (-not $SkipTests -and (Test-Path "crates")) {
            $core = Get-ChildItem "crates" -Directory |
                Where-Object { Test-Path (Join-Path $_.FullName "tests") -or (Test-Path (Join-Path $_.FullName "src")) } |
                Select-Object -First 1
            if ($core) {
                Write-Host "cargo test ($($core.Name))"
                Push-Location $core.FullName
                try {
                    cargo test --offline
                    if ($LASTEXITCODE -ne 0) { throw "core tests failed" }
                } finally { Pop-Location }
            }
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
            # artifact name follows the crate name, not the plugin id
            $crate = [regex]::Match((Get-Content "Cargo.toml" -Raw), '(?m)^name\s*=\s*"([^"]+)"').Groups[1].Value
            $built = "target/wasm32-wasip2/release/$($crate -replace '-', '_').wasm"
            if (-not (Test-Path $built)) { throw "expected $built, check the crate name" }
            New-Item -ItemType Directory -Force -Path "dist" | Out-Null
            Copy-Item $built $wasm -Force
        }
    } else {
        Write-Host "reusing dist/plugin.wasm (no build)"
    }
    if (-not (Test-Path $wasm)) { throw "no wasm component at $Plugin/dist/plugin.wasm" }
    Write-Host ("wasm component: {0:N0} bytes" -f (Get-Item $wasm).Length)

    # 3. Stage the runtime layout: exactly what the plugin needs at run time.
    $stage = Join-Path $dir "target/package/$id"
    if (Test-Path (Join-Path $dir "target/package")) {
        Remove-Item (Join-Path $dir "target/package") -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path (Join-Path $stage "dist") | Out-Null
    foreach ($f in @("plugin.toml", "settings.schema.json", "icon.svg", "README.md")) {
        if (Test-Path $f) { Copy-Item $f (Join-Path $stage $f) }
    }
    if (Test-Path $license) { Copy-Item $license (Join-Path $stage "LICENSE") }
    Copy-Item $wasm (Join-Path $stage "dist/plugin.wasm")

    # 4. Zip the stage directory itself (the archive root is the plugin folder).
    #    ZipFile.CreateFromDirectory writes `/` separators, so the archive also
    #    extracts correctly on POSIX (Compress-Archive would write `\`).
    $zip = Join-Path $out "$id-$version.zip"
    if (Test-Path $zip) { Remove-Item $zip -Force }
    Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction SilentlyContinue
    [System.IO.Compression.ZipFile]::CreateFromDirectory(
        $stage, $zip, [System.IO.Compression.CompressionLevel]::Optimal, $true)
    Write-Host ("package: {0} ({1:N0} bytes)" -f $zip, (Get-Item $zip).Length)

    # 5. Keep a committed copy in release/ so the package is one click away on
    #    GitHub (each plugin's dist/ is build output and stays ignored).
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
