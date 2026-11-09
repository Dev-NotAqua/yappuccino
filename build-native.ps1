# Builds Yappuccino natively on this Windows PC: prepares the pinned Skia
# archive, then runs `cargo build --release` (docs/building.md, native Windows).
# For development only: the program it makes names this machine's folders.
#
# It finds LLVM 21 (clang-cl, libclang) and NASM on PATH, in .local\tools, in
# Visual Studio or in their usual install folders, and stops saying what to
# get when one is missing. The log goes to .local\yappuccino-build.log.
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
Set-Location $root
$env:RUSTUP_TOOLCHAIN = '1.97.1'
$tools = Join-Path $root '.local\tools'

# The first of these folders that holds `program`.
function Find-Tool($program, $folders) {
    foreach ($folder in $folders) {
        if ($folder -and (Test-Path (Join-Path $folder $program))) { return (Resolve-Path $folder).Path }
    }
    return $null
}

function On-Path($program) {
    $found = Get-Command $program -ErrorAction SilentlyContinue
    if ($found) { return Split-Path $found.Source }
    return $null
}

$llvmFolders = @(
    (On-Path 'clang-cl.exe'),
    $(if ($env:LLVM_HOME) { Join-Path $env:LLVM_HOME 'bin' })
)
$llvmFolders += Get-ChildItem $tools -Directory -Filter 'clang+llvm-*' -ErrorAction SilentlyContinue |
    ForEach-Object { Join-Path $_.FullName 'bin' }
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path $vswhere) {
    $llvmFolders += & $vswhere -all -products * -requires Microsoft.VisualStudio.Component.VC.Llvm.Clang -property installationPath |
        ForEach-Object { Join-Path $_ 'VC\Tools\Llvm\x64\bin' }
}
$llvmFolders += 'C:\Program Files\LLVM\bin'
$llvm = Find-Tool 'clang-cl.exe' $llvmFolders
if (-not $llvm) {
    throw "LLVM wasn't found. Extract clang+llvm-21.1.8-x86_64-pc-windows-msvc.tar.xz from https://github.com/llvm/llvm-project/releases/tag/llvmorg-21.1.8 into $tools, or add Clang in the Visual Studio Installer."
}

$nasmFolders = @((On-Path 'nasm.exe'))
$nasmFolders += Get-ChildItem $tools -Directory -Filter 'nasm-*' -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName }
$nasmFolders += 'C:\Program Files\NASM'
$nasm = Find-Tool 'nasm.exe' $nasmFolders
if (-not $nasm) {
    throw "NASM wasn't found. Extract nasm-3.02-win64.zip from https://www.nasm.us/pub/nasm/releasebuilds/3.02/win64/ into $tools."
}

# bindgen finds libclang only through LIBCLANG_PATH, never PATH.
$env:LIBCLANG_PATH = $llvm
$env:PATH = "$llvm;$nasm;$env:PATH"
$log = Join-Path $root '.local\yappuccino-build.log'
New-Item -ItemType Directory -Force (Split-Path $log) | Out-Null
# Two cores spare keep the PC usable; the UI crate alone takes 6-8 GB of memory.
$jobs = [Math]::Max(1, [Environment]::ProcessorCount - 2)

Write-Output "=== start $(Get-Date -Format o): LLVM $llvm, NASM $nasm, $jobs jobs ==="
# The tools write progress to stderr. cmd joins it to stdout, so PowerShell 5.1
# shows plain lines rather than a NativeCommandError around each one.
$ErrorActionPreference = 'Continue'
cmd /c "python scripts\prepare-windows-skia.py --jobs $jobs --build 2>&1" | Tee-Object -FilePath $log
$code = $LASTEXITCODE
Write-Output "=== exit $code $(Get-Date -Format o) ==="
if ($code -eq 0) { Write-Output "Built: $(Join-Path $root 'target\release\yappuccino.exe')" }
exit $code
