# SPDX-License-Identifier: GPL-3.0-or-later
#
# Builds the BabelCraft Windows installer (.msi) with the WiX Toolset 5.
#
#   powershell -ExecutionPolicy Bypass -File packaging/windows/make-msi.ps1 -Exe target/release/babelcraft.exe -Arch x64
#
#   -Exe    the built babelcraft.exe for that architecture
#   -Arch   x64, x86 or arm64 (the installer's platform)
#   -Label  name part of the file (default: the architecture, e.g. "x64-whisper")
#   -Out    output folder (default: dist)
#
# Result: dist/BabelCraft-<version>-windows-<label>.msi
#
# One-time setup (needs the .NET SDK):
#   dotnet tool install --global wix --version 5.0.2
#   wix extension add -g WixToolset.UI.wixext/5.0.2
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [ValidateSet('x64', 'x86', 'arm64')][string]$Arch = 'x64',
    [string]$Label = '',
    [string]$Out = 'dist'
)
$ErrorActionPreference = 'Stop'
if (-not $Label) { $Label = $Arch }

$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
function Get-FullPath([string]$p) { [IO.Path]::GetFullPath($(if ([IO.Path]::IsPathRooted($p)) { $p } else { Join-Path (Get-Location) $p })) }
$exePath = Get-FullPath $Exe
if (-not (Test-Path $exePath)) { throw "BabelCraft.exe not found: $exePath" }
$outDir = Get-FullPath $Out
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.*)"').Matches[0].Groups[1].Value
$work = Join-Path $root "target\msi\$Label"
New-Item -ItemType Directory -Force $work, $outDir | Out-Null

# ---------------------------------------------------------------- licence page (GPL text as RTF)
# Paragraphs are re-flowed so the text wraps nicely in the installer's licence box.
function ConvertTo-Rtf([string]$text) {
    $text = ($text -replace "`r`n", "`n").Trim()
    $sb = New-Object Text.StringBuilder
    [void]$sb.Append('{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fswiss\fcharset0 Segoe UI;}}\viewkind4\uc1\pard\sa120\f0\fs17 ')
    $first = $true
    foreach ($para in [regex]::Split($text, "\n[ \t]*\n")) {
        $joined = (($para -split "\n") | ForEach-Object { $_.Trim() }) -join ' '
        $chars = foreach ($c in $joined.ToCharArray()) {
            if ($c -eq '\' -or $c -eq '{' -or $c -eq '}') { '\' + $c }
            elseif ([int]$c -gt 127) { '\u' + [int]$c + '?' }
            else { [string]$c }
        }
        $line = $chars -join ''
        if ($first) { $line = '\b ' + $line + '\b0 '; $first = $false }
        [void]$sb.Append($line).Append("\par`n")
    }
    [void]$sb.Append('}')
    $sb.ToString()
}
$rtf = Join-Path $work 'License.rtf'
[IO.File]::WriteAllText($rtf, (ConvertTo-Rtf ([IO.File]::ReadAllText((Join-Path $root 'LICENSE')))), [Text.Encoding]::ASCII)

# ---------------------------------------------------------------- installer pictures (from the app icon)
Add-Type -AssemblyName System.Drawing
$icon = [System.Drawing.Image]::FromFile((Join-Path $root 'assets\icon-256.png'))
$ink = [System.Drawing.Color]::FromArgb(23, 21, 15)
$yellow = [System.Drawing.Color]::FromArgb(247, 194, 27)
$cream = [System.Drawing.Color]::FromArgb(242, 234, 211)

function New-Picture([int]$w, [int]$h, [string]$path, [scriptblock]$paint) {
    $bmp = New-Object System.Drawing.Bitmap($w, $h, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    & $paint $g
    $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Bmp)
    $bmp.Dispose()
}

# Welcome / finish pages: 493 x 312, dark panel with the icon on the left, white where the text goes.
$dialog = Join-Path $work 'dialog.bmp'
New-Picture 493 312 $dialog {
    param($g)
    $g.Clear([System.Drawing.Color]::White)
    $g.FillRectangle((New-Object System.Drawing.SolidBrush($ink)), 0, 0, 164, 312)
    $g.FillRectangle((New-Object System.Drawing.SolidBrush($yellow)), 164, 0, 3, 312)
    $g.DrawImage($icon, 22, 64, 120, 120)
    $center = New-Object System.Drawing.StringFormat
    $center.Alignment = [System.Drawing.StringAlignment]::Center
    $title = New-Object System.Drawing.Font('Segoe UI', 15, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
    $small = New-Object System.Drawing.Font('Segoe UI', 11, [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel)
    $g.DrawString('BabelCraft', $title, (New-Object System.Drawing.SolidBrush($yellow)), (New-Object System.Drawing.RectangleF(0, 196, 164, 24)), $center)
    $g.DrawString('English to Japanese', $small, (New-Object System.Drawing.SolidBrush($cream)), (New-Object System.Drawing.RectangleF(0, 220, 164, 18)), $center)
    $g.DrawString('video translator', $small, (New-Object System.Drawing.SolidBrush($cream)), (New-Object System.Drawing.RectangleF(0, 236, 164, 18)), $center)
}

# Top banner of the other pages: 493 x 58, white with the icon on the right.
$banner = Join-Path $work 'banner.bmp'
New-Picture 493 58 $banner {
    param($g)
    $g.Clear([System.Drawing.Color]::White)
    $g.DrawImage($icon, 437, 6, 46, 46)
}
$icon.Dispose()

# ---------------------------------------------------------------- build the .msi
$wix = Get-Command wix -ErrorAction SilentlyContinue
$wixExe = if ($wix) { $wix.Source } else { Join-Path $env:USERPROFILE '.dotnet\tools\wix.exe' }
if (-not (Test-Path $wixExe)) {
    throw 'WiX Toolset 5 not found. Install it with: dotnet tool install --global wix --version 5.0.2 ; wix extension add -g WixToolset.UI.wixext/5.0.2'
}
$msi = Join-Path $outDir "BabelCraft-$version-windows-$Label.msi"
$wixArgs = @(
    'build', (Join-Path $PSScriptRoot 'babelcraft.wxs'),
    '-arch', $Arch,
    '-ext', 'WixToolset.UI.wixext',
    '-d', "Version=$version",
    '-d', "ExeFile=$exePath",
    '-d', "LicenseFile=$(Join-Path $root 'LICENSE')",
    '-d', "ReadmeFile=$(Join-Path $root 'README.md')",
    '-d', "IconFile=$(Join-Path $root 'assets\icon.ico')",
    '-d', "LicenseRtf=$rtf",
    '-d', "BannerBmp=$banner",
    '-d', "DialogBmp=$dialog",
    '-intermediatefolder', (Join-Path $work 'obj'),
    '-pdb', (Join-Path $work 'BabelCraft.wixpdb'),
    '-o', $msi
)
& $wixExe @wixArgs
if ($LASTEXITCODE -ne 0) { throw "wix build failed (exit $LASTEXITCODE)" }
Write-Host "Built $msi"
