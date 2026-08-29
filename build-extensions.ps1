# Builds two zip files from the same extension source:
#   extension.zip          -> Chrome / Edge / Brave / Opera / Vivaldi (service_worker)
#   extension-firefox.zip  -> Firefox (background.scripts + gecko id)
#
# The only file that differs between the two is manifest.json.

$ErrorActionPreference = "Stop"
$ext = Join-Path $PSScriptRoot "extension"
$out_chrome  = Join-Path $PSScriptRoot "extension.zip"
$out_firefox = Join-Path $PSScriptRoot "extension-firefox.zip"

$shared = @(
    "inject.js",
    "content.js",
    "background.js",
    "popup.html",
    "popup.js",
    "welcome.html",
    "icon16.png",
    "icon32.png",
    "icon48.png",
    "icon128.png"
) | ForEach-Object { Join-Path $ext $_ }

# Chrome build: shared files + the default manifest.json
$chrome_files = @($shared) + (Join-Path $ext "manifest.json")
Compress-Archive -Path $chrome_files -DestinationPath $out_chrome -Force
Write-Host "wrote $out_chrome"

# Firefox build: copy manifest-firefox.json to a temp file named manifest.json,
# then zip it alongside the shared files.
$temp = New-Item -ItemType Directory -Path (Join-Path $env:TEMP ("ytrpc-fx-" + [Guid]::NewGuid())) -Force
try {
    foreach ($f in $shared) { Copy-Item $f $temp.FullName }
    Copy-Item (Join-Path $ext "manifest-firefox.json") (Join-Path $temp.FullName "manifest.json")
    $firefox_files = Get-ChildItem $temp.FullName | ForEach-Object { $_.FullName }
    Compress-Archive -Path $firefox_files -DestinationPath $out_firefox -Force
    Write-Host "wrote $out_firefox"
} finally {
    Remove-Item $temp.FullName -Recurse -Force
}
