# Install the complete Cowdex package using PowerShell and the built-in ZIP APIs.
param(
    [string]$Archive = '',
    [string]$Repo = 'cutoken/cowdex',
    [string]$Release = 'latest',
    [string]$PackageRoot = '',
    [string]$BinDir = ''
)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Use install.sh on Linux and macOS.' }
$architecture = [Environment]::GetEnvironmentVariable('PROCESSOR_ARCHITECTURE', 'Machine')
if (!$architecture) { $architecture = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE } }
$target = switch ($architecture.ToUpperInvariant()) {
    'AMD64' { 'x86_64-pc-windows-msvc' }
    'ARM64' { 'aarch64-pc-windows-msvc' }
    default { throw "Unsupported processor architecture: $architecture" }
}
if (!$PackageRoot) {
    $stateHome = if ($env:CODEX_HOME) { $env:CODEX_HOME } else { Join-Path $HOME '.codex' }
    $PackageRoot = if ($env:COWDEX_PACKAGE_ROOT) { $env:COWDEX_PACKAGE_ROOT } else { Join-Path $stateHome 'packages/cowdex' }
}
if (!$BinDir) { $BinDir = if ($env:COWDEX_INSTALL_DIR) { $env:COWDEX_INSTALL_DIR } else { Join-Path $HOME '.local/bin' } }
$PackageRoot = [IO.Path]::GetFullPath($PackageRoot)
$BinDir = [IO.Path]::GetFullPath($BinDir)
$releases = Join-Path $PackageRoot 'releases'
[IO.Directory]::CreateDirectory($releases) | Out-Null
$scratch = Join-Path $PackageRoot ('.install-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($scratch) | Out-Null
try {
    if (!$Archive) {
        if ($Repo -notmatch '^[\w.-]+/[\w.-]+$') { throw 'Invalid repository.' }
        if ($Release -eq 'latest') { $sourceUrl = "https://github.com/$Repo/releases/latest/download" }
        else {
            $Release = $Release -replace '^cowdex-v', ''
            if ($Release -notmatch '^\d+\.\d+\.\d+(-r[1-9]\d*)?$') { throw 'Invalid release version.' }
            $sourceUrl = "https://github.com/$Repo/releases/download/cowdex-v$Release"
        }
        $Archive = Join-Path $scratch 'package.zip'
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -UseBasicParsing -Uri "$sourceUrl/cowdex-$target.zip" -OutFile $Archive
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead([IO.Path]::GetFullPath($Archive))
    try {
        foreach ($entry in $zip.Entries) {
            if ($entry.FullName -match '(^[\\/]|(^|[\\/])\.\.([\\/]|$)|:)') { throw "Unsafe archive path: $($entry.FullName)" }
            $kind = ($entry.ExternalAttributes -shr 16) -band 0xF000
            if ($kind -notin @(0, 0x8000, 0x4000)) { throw 'Package archives cannot contain links.' }
        }
    } finally { $zip.Dispose() }
    $package = Join-Path $scratch 'package'
    [IO.Compression.ZipFile]::ExtractToDirectory([IO.Path]::GetFullPath($Archive), $package)
    $metadata = Get-Content -Raw (Join-Path $package 'codex-package.json') | ConvertFrom-Json
    if ($metadata.variant -ne 'cowdex' -or $metadata.target -ne $target) { throw "Package must be a Cowdex release for $target." }
    $version = if ($metadata.releaseVersion) { $metadata.releaseVersion } else { $metadata.version }
    if ($version -notmatch '^\d+\.\d+\.\d+(-r[1-9]\d*)?$' -or ($version -replace '-r\d+$', '') -ne $metadata.version) { throw 'Invalid package version.' }
    foreach ($helper in @('bin/cowdex.exe', 'bin/codex-code-mode-host.exe', 'bin/cowdex-vault.exe', 'codex-path/rg.exe', 'codex-resources/codex-command-runner.exe', 'codex-resources/codex-windows-sandbox-setup.exe')) {
        if (!(Test-Path -PathType Leaf (Join-Path $package $helper))) { throw "Incomplete package: missing $helper" }
    }
    & (Join-Path $package 'bin/cowdex.exe') --version
    if ($LASTEXITCODE -ne 0) { throw 'The Cowdex executable could not run.' }
    $installed = Join-Path $releases ("$version-$target-" + [Guid]::NewGuid().ToString('N'))
    [IO.Directory]::Move($package, $installed)
    [IO.Directory]::CreateDirectory($BinDir) | Out-Null
    foreach ($program in @('cowdex', 'cowdex-vault')) {
        $executable = (Join-Path $installed "bin/$program.exe").Replace('%', '%%')
        $arguments = if ($program -eq 'cowdex') { ' -c features.multi_agent_v2.preserve_fork_prefix=true' } else { '' }
        $contents = "@echo off`r`n`"$executable`"$arguments %*`r`n"
        $temporary = Join-Path $BinDir ('.launcher-' + [Guid]::NewGuid().ToString('N'))
        [IO.File]::WriteAllText($temporary, $contents, [Text.UTF8Encoding]::new($false))
        $launcher = Join-Path $BinDir "$program.cmd"
        if ([IO.File]::Exists($launcher)) { [IO.File]::Replace($temporary, $launcher, [NullString]::Value) }
        else { [IO.File]::Move($temporary, $launcher) }
    }
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $BinDir) {
        [Environment]::SetEnvironmentVariable('Path', "$BinDir;$userPath".TrimEnd(';'), 'User')
    }
    if (($env:Path -split ';') -notcontains $BinDir) { $env:Path = "$BinDir;$env:Path" }
    Write-Host "Installed Cowdex $version at $installed"
    Write-Host "Command: $(Join-Path $BinDir 'cowdex.cmd')"
} finally {
    if (Test-Path $scratch) { Remove-Item -Recurse -Force $scratch }
}
