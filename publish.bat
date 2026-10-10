@echo off
setlocal
title Publish Archive Helper
cd /d "%~dp0"
echo ===================================================
echo   Publishing Archive Helper (Single-File .EXE)
echo ===================================================
echo.

set "ARCHIVE_HELPER_DOTNET="
call :try_dotnet "dotnet"
if not defined ARCHIVE_HELPER_DOTNET if defined DOTNET_ROOT call :try_dotnet "%DOTNET_ROOT%\dotnet.exe"
if not defined ARCHIVE_HELPER_DOTNET call :try_dotnet "%USERPROFILE%\.dotnet\dotnet.exe"
if not defined ARCHIVE_HELPER_DOTNET goto missing_sdk

echo Building single-file executable...
"%ARCHIVE_HELPER_DOTNET%" publish "ArchiveHelper.csproj" -c Release -r win-x64 --self-contained false -p:PublishSingleFile=true -o "./publish"
if errorlevel 1 goto build_failed
if not exist "publish\ArchiveHelper.exe" goto build_failed

echo.
echo ===================================================
echo   BUILD SUCCESSFUL!
echo   Output: "%~dp0publish\ArchiveHelper.exe"
echo ===================================================
echo.
if /i not "%~1"=="--no-pause" pause
exit /b 0

:missing_sdk
echo No .NET SDK was found. A runtime alone cannot build the app.
echo Install the .NET 8 SDK from https://aka.ms/dotnet/download
echo Then run this script again.
goto build_failed

:build_failed
echo.
echo ===================================================
echo   BUILD FAILED. Check errors above.
echo ===================================================
echo.
if /i not "%~1"=="--no-pause" pause
exit /b 1

:try_dotnet
if /i "%~1"=="dotnet" (
    where dotnet >nul 2>&1
    if errorlevel 1 exit /b 0
) else (
    if not exist "%~1" exit /b 0
)
"%~1" --list-sdks 2>nul | findstr /r "[0-9]" >nul
if not errorlevel 1 set "ARCHIVE_HELPER_DOTNET=%~1"
exit /b 0
