param(
    [string]$Url = "https://raw.githubusercontent.com/dysolix/hasagi-types/main/swagger.json",
    [string]$OutFile = "schema\swagger.json"
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$target = Join-Path $root $OutFile
$targetDirectory = Split-Path -Parent $target
$temporary = "$target.tmp"

New-Item -ItemType Directory -Force $targetDirectory | Out-Null
Invoke-WebRequest -UseBasicParsing $Url -OutFile $temporary

$json = Get-Content $temporary -Raw | ConvertFrom-Json
if (-not $json.openapi -or -not $json.paths) {
    Remove-Item -LiteralPath $temporary -Force
    throw "Downloaded file is not an OpenAPI document with paths."
}

Move-Item -LiteralPath $temporary -Destination $target -Force

$pathCount = ($json.paths.PSObject.Properties | Measure-Object).Count
$schemaCount = if ($json.components.schemas) {
    ($json.components.schemas.PSObject.Properties | Measure-Object).Count
} else {
    0
}

Write-Host "Updated $OutFile"
Write-Host "OpenAPI: $($json.openapi)"
Write-Host "Version: $($json.info.version)"
Write-Host "Paths: $pathCount"
Write-Host "Schemas: $schemaCount"
