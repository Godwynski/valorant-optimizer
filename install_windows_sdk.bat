@echo off
echo ==============================================================================
echo Launching Visual Studio Installer...
echo ==============================================================================
echo.
echo In the Visual Studio Installer window:
echo   1. Click "Modify" next to Visual Studio Community 2026.
echo   2. On the right-hand panel ("Installation details"), ensure:
echo        [X] Windows 11 SDK (10.0.26100.0 or 10.0.22621.0)
echo      is CHECKED.
echo   3. Click "Modify" in the bottom-right corner to install.
echo.
echo ==============================================================================
start "" "C:\Program Files (x86)\Microsoft Visual Studio\Installer\setup.exe"
pause
