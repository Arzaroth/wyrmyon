# Usage: powershell -c "irm https://raw.githubusercontent.com/Arzaroth/wyrmyon/master/scripts/install.ps1 | iex"
#
# Installs wyrmyon.exe and wyrm.exe from a GitHub release into
# %LOCALAPPDATA%\Programs\wyrmyon, the MSI's directory, and adds it to the
# user PATH. WYRMYON_VERSION picks a release (default: the latest).

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$repo = if ($env:WYRMYON_REPO) { $env:WYRMYON_REPO } else { 'Arzaroth/wyrmyon' }
$base = $env:WYRMYON_DOWNLOAD_BASE
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\wyrmyon'

$machine = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
$arch = switch ($machine) {
    'AMD64' { 'x86_64' }
    'ARM64' { 'aarch64' }
    default { throw "install: no release for $machine" }
}

if (-not $base) {
    $tag = $env:WYRMYON_VERSION
    if (-not $tag) {
        $tag = (Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest").tag_name
    }
    if (-not $tag.StartsWith('v')) { $tag = "v$tag" }
    $base = "https://github.com/$repo/releases/download/$tag"
}

function Fetch($name, $to) {
    if ($base -match '^[a-z]+://') {
        Invoke-WebRequest -UseBasicParsing "$base/$name" -OutFile $to
    } else {
        Copy-Item (Join-Path $base $name) $to
    }
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
New-Item -ItemType Directory $tmp | Out-Null
try {
    Fetch 'SHA256SUMS' "$tmp\SHA256SUMS"
    $line = Get-Content "$tmp\SHA256SUMS" | Where-Object { $_ -match "  (wyrmyon-v\S+-$arch-pc-windows-msvc\.zip)$" } | Select-Object -First 1
    if (-not $line) { throw "install: no $arch Windows archive in $base" }
    $expected, $archive = $line -split '  ', 2
    Write-Host "Downloading $archive"
    Fetch $archive "$tmp\$archive"
    $actual = (Get-FileHash -Algorithm SHA256 "$tmp\$archive").Hash.ToLower()
    if ($actual -ne $expected) { throw "install: $archive does not match its SHA256SUMS entry" }

    Expand-Archive "$tmp\$archive" -DestinationPath $tmp
    $src = Join-Path $tmp ([IO.Path]::GetFileNameWithoutExtension($archive))
    New-Item -ItemType Directory -Force $installDir | Out-Null
    foreach ($exe in 'wyrmyon.exe', 'wyrm.exe') {
        $old = Join-Path $installDir $exe
        Remove-Item "$old.old" -Force -ErrorAction SilentlyContinue
        if (Test-Path $old) { Move-Item $old "$old.old" -Force }
    }
    foreach ($file in 'wyrmyon.exe', 'wyrm.exe', 'README.md', 'LICENSE') {
        Copy-Item (Join-Path $src $file) $installDir -Force
    }
    foreach ($exe in 'wyrmyon.exe', 'wyrm.exe') {
        Remove-Item (Join-Path $installDir "$exe.old") -Force -ErrorAction SilentlyContinue
    }
} finally {
    Remove-Item -Recurse -Force $tmp
}

$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
try {
    $userPath = $key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    $entries = $userPath -split ';' | Where-Object { $_ } | ForEach-Object { $_.TrimEnd('\') }
    if ($entries -notcontains $installDir) {
        $joined = if ($userPath) { "$($userPath.TrimEnd(';'));$installDir" } else { $installDir }
        $key.SetValue('Path', $joined, [Microsoft.Win32.RegistryValueKind]::ExpandString)
        # A registry write alone does not tell running programs; this broadcasts WM_SETTINGCHANGE.
        [Environment]::SetEnvironmentVariable('WYRMYON_PATH_REFRESH', '1', 'User')
        [Environment]::SetEnvironmentVariable('WYRMYON_PATH_REFRESH', $null, 'User')
        Write-Host "Added $installDir to your PATH: open a new terminal to use wyrm."
    }
} finally {
    $key.Close()
}
Write-Host "Installed $(& (Join-Path $installDir 'wyrmyon.exe') --version) into $installDir"
