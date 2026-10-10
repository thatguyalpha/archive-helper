@echo off
title Publish Archive Helper
cd /d "%~dp0"
echo ===================================================
echo   Publishing Archive Helper (Single-File .EXE)
echo ===================================================
echo.

echo Building portable single-file executable...
dotnet publish -c Release -r win-x64 --self-contained false -p:PublishSingleFile=true -o "./publish"

echo.
if exist "publish\ArchiveHelper.exe" (
    echo ===================================================
    echo   BUILD SUCCESSFUL!
    echo   Output: "%~dp0publish\ArchiveHelper.exe"
    echo ===================================================
) else (
    echo ===================================================
    echo   BUILD FAILED. Check errors above.
    echo ===================================================
)
echo.
pause
