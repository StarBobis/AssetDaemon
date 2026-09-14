@echo off
setlocal

for /f "usebackq delims=" %%i in (`rustc --print sysroot`) do set "RUST_SYSROOT=%%i"

set "RUST_LLD=%RUST_SYSROOT%\lib\rustlib\x86_64-pc-windows-msvc\bin\rust-lld.exe"

if not exist "%RUST_LLD%" (
  echo rust-lld.exe not found at "%RUST_LLD%" 1>&2
  exit /b 1
)

"%RUST_LLD%" -flavor link %*
