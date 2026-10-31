param([Parameter(Mandatory=$true)][string]$Root)
$ErrorActionPreference = 'Stop'
$rootPath = [IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
foreach ($name in @('cl.exe', 'link.exe')) {
  $compiler = (Get-Command $name -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
  if (!$compiler.StartsWith($rootPath, [StringComparison]::OrdinalIgnoreCase)) { throw "$name must come from $rootPath, not $compiler" }
  Write-Host "$name`: $compiler"
}
